//! Removes the selected-target marker without expanding a compact rustc command line.
//!
//! Cargo can place compiler arguments in a UTF-8 response file when the direct command line would
//! exceed the operating-system limit. This module recognizes rustc's ordinary line-based response
//! files, copies a selected file without the private marker, and leaves every other argument byte
//! unchanged. It passes shell-style response files through because rustc owns their parsing.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process;

pub(crate) fn remove_selected_target_marker(
    arguments: &mut Vec<OsString>,
    marker: &str,
    output_directory: &Path,
) -> Result<bool, String> {
    let mut marker_count = 0_usize;

    for (index, argument) in arguments.iter_mut().enumerate() {
        let removed = if argument == OsStr::new(marker) {
            1
        } else {
            remove_response_file_marker(argument, marker, output_directory, index)?
        };
        marker_count = marker_count.saturating_add(removed);
        if marker_count > 1 {
            return Err(format!(
                "selected-target marker must appear at most once, got {marker_count}"
            ));
        }
    }

    arguments.retain(|argument| argument != OsStr::new(marker));

    Ok(marker_count == 1)
}

/// Replaces a response-file argument with a filtered copy when it contains exactly one marker.
///
/// Returns the number of markers found so the caller can reject duplicates across arguments.
fn remove_response_file_marker(
    argument: &mut OsString,
    marker: &str,
    output_directory: &Path,
    argument_index: usize,
) -> Result<usize, String> {
    let Some(response_path) = response_file_path(argument) else {
        return Ok(0);
    };
    let contents = fs::read_to_string(response_path).map_err(|error| {
        format!(
            "rustc response file must be readable UTF-8 at {}, got {error}",
            response_path.display()
        )
    })?;
    let (filtered_contents, marker_count) = filter_response_file(&contents, marker);
    if marker_count != 1 {
        return Ok(marker_count);
    }

    let filtered_path = output_directory.join(format!(
        "optic-rustc-arguments-{}-{argument_index}.rsp",
        process::id()
    ));
    fs::write(&filtered_path, filtered_contents).map_err(|error| {
        format!(
            "filtered rustc response file must be writable at {}, got {error}",
            filtered_path.display()
        )
    })?;
    let mut filtered_argument = OsString::from("@");
    filtered_argument.push(filtered_path);
    *argument = filtered_argument;

    Ok(marker_count)
}

fn response_file_path(argument: &OsStr) -> Option<&Path> {
    let argument = argument.to_str()?;
    let path = argument.strip_prefix('@')?;
    if path.starts_with("shell:") {
        return None;
    }

    Some(Path::new(path))
}

fn filter_response_file(contents: &str, marker: &str) -> (String, usize) {
    let mut filtered = String::with_capacity(contents.len());
    let mut marker_count = 0_usize;

    for line in contents.split_inclusive('\n') {
        let argument = line.strip_suffix('\n').unwrap_or(line);
        let argument = argument.strip_suffix('\r').unwrap_or(argument);
        if argument == marker {
            marker_count = marker_count.saturating_add(1);
        } else {
            filtered.push_str(line);
        }
    }

    (filtered, marker_count)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use super::filter_response_file;
    use super::remove_selected_target_marker;

    const MARKER: &str = "--cfg=cargo_optic_selected_target=\"fixture\"";

    #[track_caller]
    fn assert_rejects_duplicate_markers(direct_count: usize, response_count: usize) {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let response_path = temporary.path().join("arguments.rsp");
        let contents = format!("{MARKER}\n").repeat(response_count);
        fs::write(&response_path, contents).expect("the response file can be written");
        let mut response_argument = OsString::from("@");
        response_argument.push(response_path);
        let mut arguments = vec![OsString::from(MARKER); direct_count];
        arguments.push(response_argument);

        let error = remove_selected_target_marker(&mut arguments, MARKER, temporary.path())
            .expect_err("duplicate markers must be rejected");

        assert_eq!(
            error,
            "selected-target marker must appear at most once, got 2"
        );
    }

    #[test]
    fn rejects_duplicate_direct_markers() {
        assert_rejects_duplicate_markers(2, 0);
    }

    #[test]
    fn rejects_duplicate_response_file_markers() {
        assert_rejects_duplicate_markers(0, 2);
    }

    #[test]
    fn rejects_markers_in_both_direct_arguments_and_a_response_file() {
        assert_rejects_duplicate_markers(1, 1);
    }

    #[test]
    fn removes_a_direct_marker() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let mut arguments = vec![
            // Compiler.
            OsString::from("rustc"), //
            // Selected-target marker.
            OsString::from(MARKER), //
            // Compiler input.
            OsString::from("input.rs"), //
        ];

        let selected = remove_selected_target_marker(&mut arguments, MARKER, temporary.path())
            .expect("the direct marker can be removed");

        assert!(selected);
        assert_eq!(arguments, ["rustc", "input.rs"]);
    }

    #[test]
    fn preserves_an_absent_marker() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let response_path = temporary.path().join("arguments.rsp");
        let contents = "--crate-name\nfixture\ninput.rs\n";
        fs::write(&response_path, contents).expect("the response file can be written");
        let mut response_argument = OsString::from("@");
        response_argument.push(&response_path);
        let mut arguments = vec![OsString::from("rustc"), response_argument.clone()];

        let selected = remove_selected_target_marker(&mut arguments, MARKER, temporary.path())
            .expect("the absent marker can be checked");

        assert!(!selected);
        assert_eq!(arguments, [OsString::from("rustc"), response_argument]);
        assert_eq!(
            fs::read_to_string(response_path).expect("the response file can be read"),
            contents
        );
    }

    #[test]
    fn preserves_a_shell_style_response_file() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let mut arguments = vec![
            // Compiler.
            OsString::from("rustc"), //
            // Shell-style response-file feature.
            OsString::from("-Zshell-argfiles"), //
            // Shell-style response file.
            OsString::from("@shell:arguments.rsp"), //
        ];
        let expected = arguments.clone();

        let selected = remove_selected_target_marker(&mut arguments, MARKER, temporary.path())
            .expect("the shell-style response file can pass through");

        assert!(!selected);
        assert_eq!(arguments, expected);
    }

    #[test]
    fn filters_a_line_feed_response_file() {
        let temporary = tempfile::tempdir().expect("the test can create a temporary directory");
        let response_path = temporary.path().join("arguments.rsp");
        let contents = format!("--crate-name\nfixture\n{MARKER}\ninput.rs\n");
        fs::write(&response_path, contents).expect("the response file can be written");
        let mut response_argument = OsString::from("@");
        response_argument.push(response_path);
        let mut arguments = vec![OsString::from("rustc"), response_argument];

        let selected = remove_selected_target_marker(&mut arguments, MARKER, temporary.path())
            .expect("the response-file marker can be removed");
        let filtered_path = arguments[1]
            .to_str()
            .expect("the filtered argument is UTF-8")
            .strip_prefix('@')
            .expect("the filtered argument names a response file");

        assert!(selected);
        assert_eq!(
            fs::read_to_string(filtered_path).expect("the filtered response file can be read"),
            "--crate-name\nfixture\ninput.rs\n"
        );
    }

    #[test]
    fn filters_a_carriage_return_response_file_without_other_changes() {
        let contents = format!("--crate-name\r\nfixture\r\n{MARKER}\r\ninput.rs");

        let (filtered, count) = filter_response_file(&contents, MARKER);

        assert_eq!(count, 1);
        assert_eq!(filtered, "--crate-name\r\nfixture\r\ninput.rs");
    }
}
