//! Streams bounded Cargo output while collecting constant-size artifact state.
//!
//! [`run`] drains both child-process pipes concurrently. It forwards user-visible bytes, parses
//! one bounded Cargo JSON message at a time, and retains only a bounded failure tail.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;

use crate::{Error, Result};

// This capacity absorbs short interleaved bursts while keeping queued output bounded.
const CHANNEL_CAPACITY: usize = 16;
const READ_BUFFER_BYTES: usize = 8 * 1024;
// The public Cargo-output contract documents these limits. A larger value increases peak memory.
const MAX_CARGO_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_FAILURE_DIAGNOSTIC_BYTES: usize = 1024 * 1024;

/// One user-visible output event from a Cargo subprocess.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CargoProcessEvent {
    /// A rendered compiler diagnostic from Cargo's JSON output.
    CompilerDiagnostic {
        /// The rendered diagnostic bytes.
        bytes: Vec<u8>,
    },

    /// Output that was not a Cargo JSON message.
    Stdout {
        /// The original standard-output bytes.
        bytes: Vec<u8>,
    },

    /// Output that Cargo or a child process wrote to standard error.
    Stderr {
        /// The original standard-error bytes.
        bytes: Vec<u8>,
    },
}

impl CargoProcessEvent {
    /// Returns the bytes that the event contains.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::CompilerDiagnostic { bytes }
            | Self::Stdout { bytes }
            | Self::Stderr { bytes } => bytes,
        }
    }
}

#[derive(Debug, Default)]
struct ArtifactSummary {
    count: usize,
    has_stale_artifact: bool,
}

impl ArtifactSummary {
    fn record(&mut self, fresh: bool) {
        self.count = self.count.saturating_add(1);
        self.has_stale_artifact |= !fresh;
    }

    fn fresh_count(&self) -> Option<usize> {
        (self.count != 0 && !self.has_stale_artifact).then_some(self.count)
    }
}

pub(crate) struct CargoRun {
    status: ExitStatus,
    artifacts: ArtifactSummary,
    diagnostics: BoundedTail,
}

impl CargoRun {
    pub(crate) fn status(&self) -> ExitStatus {
        self.status
    }

    pub(crate) fn fresh_artifact_count(&self) -> Option<usize> {
        self.artifacts.fresh_count()
    }

    pub(crate) fn diagnostics(&self) -> String {
        self.diagnostics.to_string()
    }
}

enum ReaderMessage {
    Event(CargoProcessEvent),
    Artifact { fresh: bool },
    MessageLimit { actual: usize },
    ReadError(io::Error),
}

pub(crate) fn run(
    command: &mut Command,
    program: &str,
    on_event: &mut dyn FnMut(CargoProcessEvent),
) -> Result<CargoRun> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|source| Error::StartProcess {
        program: program.to_owned(),
        source,
    })?;
    let stdout = child
        .stdout
        .take()
        .expect("Cargo stdout was configured as a pipe before the child started");
    let stderr = child
        .stderr
        .take()
        .expect("Cargo stderr was configured as a pipe before the child started");
    let (sender, receiver) = sync_channel(CHANNEL_CAPACITY);
    let stdout_sender = sender.clone();
    let stdout_reader = thread::spawn(move || read_stdout(stdout, &stdout_sender));
    let stderr_reader = thread::spawn(move || read_stderr(stderr, &sender));
    let mut artifacts = ArtifactSummary::default();
    let mut diagnostics = BoundedTail::new(MAX_FAILURE_DIAGNOSTIC_BYTES);
    let mut output_error = None;

    for message in receiver {
        match message {
            ReaderMessage::Event(event) => {
                diagnostics.push(event.bytes());
                on_event(event);
            }
            ReaderMessage::Artifact { fresh } => {
                artifacts.record(fresh);
            }
            ReaderMessage::MessageLimit { actual } => {
                output_error.get_or_insert_with(|| {
                    format!(
                        "Cargo JSON messages must not exceed {} bytes, got {actual}",
                        MAX_CARGO_MESSAGE_BYTES
                    )
                });
            }
            ReaderMessage::ReadError(error) => {
                output_error.get_or_insert_with(|| format!("failed to read Cargo output: {error}"));
            }
        }
    }

    let stdout_result = stdout_reader.join();
    let stderr_result = stderr_reader.join();
    let status = child.wait().map_err(|error| Error::CargoOutput {
        program: program.to_owned(),
        message: format!("failed to wait for the child process: {error}"),
    })?;
    assert!(
        stdout_result.is_ok() && stderr_result.is_ok(),
        "Cargo output reader threads must not panic"
    );

    if let Some(message) = output_error {
        return Err(Error::CargoOutput {
            program: program.to_owned(),
            message,
        });
    }

    Ok(CargoRun {
        status,
        artifacts,
        diagnostics,
    })
}

fn read_stdout(mut stdout: impl Read, sender: &SyncSender<ReaderMessage>) {
    let mut read_buffer = [0; READ_BUFFER_BYTES];
    let mut line = Vec::with_capacity(READ_BUFFER_BYTES);
    let mut oversized = false;
    let mut line_length = 0_usize;

    loop {
        let length = match stdout.read(&mut read_buffer) {
            Ok(0) => break,
            Ok(length) => length,
            Err(error) => {
                send(sender, ReaderMessage::ReadError(error));

                return;
            }
        };

        for byte in &read_buffer[..length] {
            line_length = line_length.saturating_add(1);

            // An oversized line cannot be parsed safely. Forward its exact bytes in bounded chunks
            // and continue draining so that the child process cannot block on a full pipe.
            if !oversized && line.len() == MAX_CARGO_MESSAGE_BYTES {
                send_stdout(sender, std::mem::take(&mut line));
                oversized = true;
            }

            line.push(*byte);

            if oversized && (line.len() == READ_BUFFER_BYTES || *byte == b'\n') {
                send_stdout(sender, std::mem::take(&mut line));
            }

            if *byte == b'\n' {
                line = finish_stdout_line(sender, line, oversized, line_length);
                oversized = false;
                line_length = 0;
            }
        }
    }

    if line_length != 0 {
        let _ = finish_stdout_line(sender, line, oversized, line_length);
    }
}

/// Emits one complete Cargo stdout line and returns storage for the next line.
fn finish_stdout_line(
    sender: &SyncSender<ReaderMessage>,
    line: Vec<u8>,
    oversized: bool,
    line_length: usize,
) -> Vec<u8> {
    if oversized {
        if !line.is_empty() {
            send_stdout(sender, line);
        }
        send(
            sender,
            ReaderMessage::MessageLimit {
                actual: line_length,
            },
        );

        return Vec::with_capacity(READ_BUFFER_BYTES);
    }

    process_stdout_line(sender, line)
}

/// Classifies one bounded Cargo stdout line and returns its cleared storage when possible.
fn process_stdout_line(sender: &SyncSender<ReaderMessage>, mut line: Vec<u8>) -> Vec<u8> {
    let Ok(message) = serde_json::from_slice::<serde_json::Value>(&line) else {
        send_stdout(sender, line);

        return Vec::with_capacity(READ_BUFFER_BYTES);
    };

    match message["reason"].as_str() {
        Some("compiler-message") => {
            if let Some(rendered) = message["message"]["rendered"].as_str() {
                send(
                    sender,
                    ReaderMessage::Event(CargoProcessEvent::CompilerDiagnostic {
                        bytes: rendered.as_bytes().to_vec(),
                    }),
                );
            }
        }
        Some("compiler-artifact") => {
            if let Some(fresh) = message["fresh"].as_bool() {
                send(sender, ReaderMessage::Artifact { fresh });
            }
        }
        _ => {}
    }

    line.clear();

    line
}

fn read_stderr(mut stderr: impl Read, sender: &SyncSender<ReaderMessage>) {
    let mut buffer = [0; READ_BUFFER_BYTES];

    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => return,
            Ok(length) => send(
                sender,
                ReaderMessage::Event(CargoProcessEvent::Stderr {
                    bytes: buffer[..length].to_vec(),
                }),
            ),
            Err(error) => {
                send(sender, ReaderMessage::ReadError(error));

                return;
            }
        }
    }
}

fn send_stdout(sender: &SyncSender<ReaderMessage>, bytes: Vec<u8>) {
    send(
        sender,
        ReaderMessage::Event(CargoProcessEvent::Stdout { bytes }),
    );
}

fn send(sender: &SyncSender<ReaderMessage>, message: ReaderMessage) {
    let _ = sender.send(message);
}

struct BoundedTail {
    bytes: VecDeque<u8>,
    maximum: usize,
    truncated: bool,
}

impl BoundedTail {
    fn new(maximum: usize) -> Self {
        assert!(maximum != 0, "bounded tail capacity must be nonzero");

        Self {
            bytes: VecDeque::new(),
            maximum,
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.len() >= self.maximum {
            self.bytes.clear();
            self.bytes.extend(&bytes[bytes.len() - self.maximum..]);
            self.truncated = true;

            return;
        }

        let required = self.bytes.len() + bytes.len();
        if required > self.maximum {
            self.bytes.drain(..required - self.maximum);
            self.truncated = true;
        }
        self.bytes.extend(bytes);
    }
}

impl std::fmt::Display for BoundedTail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.truncated {
            formatter.write_str("[earlier Cargo output was truncated]\n")?;
        }

        let bytes = self.bytes.iter().copied().collect::<Vec<_>>();

        formatter.write_str(&String::from_utf8_lossy(&bytes))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};
    use std::process::Command;
    use std::sync::mpsc::sync_channel;
    use std::thread;

    use super::{
        ArtifactSummary, BoundedTail, CHANNEL_CAPACITY, CargoProcessEvent, MAX_CARGO_MESSAGE_BYTES,
        ReaderMessage, read_stdout,
    };

    struct ChunkedReader<'a> {
        cursor: Cursor<&'a [u8]>,
        maximum: usize,
    }

    impl<'a> ChunkedReader<'a> {
        fn new(bytes: &'a [u8], maximum: usize) -> Self {
            Self {
                cursor: Cursor::new(bytes),
                maximum,
            }
        }
    }

    impl Read for ChunkedReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let maximum = self.maximum.min(buffer.len());

            self.cursor.read(&mut buffer[..maximum])
        }
    }

    fn read_messages(reader: impl Read + Send) -> Vec<ReaderMessage> {
        let (sender, receiver) = sync_channel(CHANNEL_CAPACITY);

        thread::scope(|scope| {
            let reader = scope.spawn(move || read_stdout(reader, &sender));
            let messages = receiver.into_iter().collect();
            reader
                .join()
                .expect("the Cargo stdout test reader must not panic");

            messages
        })
    }

    #[test]
    fn parses_fragmented_diagnostics_and_artifacts() {
        let input = br#"{"reason":"compiler-message","message":{"rendered":"warning: example\n"}}
{"reason":"compiler-artifact","fresh":true}
"#;
        let messages = read_messages(ChunkedReader::new(input, 3));

        assert!(matches!(
            &messages[0],
            ReaderMessage::Event(CargoProcessEvent::CompilerDiagnostic { bytes })
                if bytes == b"warning: example\n"
        ));
        assert!(matches!(
            messages[1],
            ReaderMessage::Artifact { fresh: true }
        ));
    }

    #[test]
    fn preserves_non_json_and_non_utf8_stdout() {
        let bytes = vec![0xff, b'\n'];
        let messages = read_messages(Cursor::new(bytes.clone()));

        assert!(matches!(
            &messages[0],
            ReaderMessage::Event(CargoProcessEvent::Stdout { bytes: actual }) if *actual == bytes
        ));
    }

    #[test]
    fn fresh_artifacts_require_at_least_one_artifact_and_no_stale_artifacts() {
        let mut artifacts = ArtifactSummary::default();

        assert_eq!(artifacts.fresh_count(), None);

        artifacts.record(true);
        assert_eq!(artifacts.fresh_count(), Some(1));

        artifacts.record(false);
        assert_eq!(artifacts.fresh_count(), None);
    }

    #[test]
    fn retains_only_the_end_of_failure_output() {
        let mut tail = BoundedTail::new(5);

        tail.push(b"1234");
        tail.push(b"5678");

        assert_eq!(
            tail.to_string(),
            "[earlier Cargo output was truncated]\n45678"
        );
    }

    #[test]
    fn reports_an_oversized_stdout_message_after_reading_it() {
        let mut input = vec![b'x'; MAX_CARGO_MESSAGE_BYTES + 1];
        input.push(b'\n');
        let messages = read_messages(Cursor::new(input));

        assert!(messages.into_iter().any(|message| matches!(
            message,
            ReaderMessage::MessageLimit { actual } if actual == MAX_CARGO_MESSAGE_BYTES + 2
        )));
    }

    #[cfg(unix)]
    #[test]
    fn reads_stdout_and_stderr_without_waiting_for_one_stream() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf '%s\\n' '{\"reason\":\"compiler-artifact\",\"fresh\":true}'; \
             printf 'cargo warning\\n' >&2",
        ]);
        let mut events = Vec::new();

        let output = super::run(&mut command, "test Cargo", &mut |event| events.push(event))
            .expect("the supervised process succeeds");

        assert!(output.status().success());
        assert_eq!(output.fresh_artifact_count(), Some(1));
        assert!(events.iter().any(|event| matches!(
            event,
            CargoProcessEvent::Stderr { bytes } if bytes == b"cargo warning\n"
        )));
    }
}
