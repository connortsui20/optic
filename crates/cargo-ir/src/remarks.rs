//! Reads LLVM optimization remarks emitted for the selected Rust target.
//!
//! [`parse_optimization_remarks`] reads the YAML document stream from one `*.opt.opt.yaml` file.
//! It bounds the file, each document, the record count, strings, and argument count before it
//! returns typed [`OptimizationRemark`] records. Collection and persistent storage remain the
//! caller's responsibility.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::{Error, Result};

const DOCUMENT_START: &[u8] = b"---";
const DOCUMENT_END: &[u8] = b"...";

/// Resource limits for one LLVM optimization-remark file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemarkParseLimits {
    /// The maximum size of the complete file in bytes.
    pub max_file_bytes: u64,

    /// The maximum size of one YAML document in bytes.
    pub max_document_bytes: usize,

    /// The maximum number of remark documents in one file.
    pub max_records: usize,

    /// The maximum UTF-8 byte length of one string value.
    pub max_string_bytes: usize,

    /// The maximum number of argument fragments in one remark.
    pub max_arguments: usize,
}

impl Default for RemarkParseLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 256 * 1024 * 1024,
            max_document_bytes: 4 * 1024 * 1024,
            max_records: 1_000_000,
            max_string_bytes: 64 * 1024,
            max_arguments: 4_096,
        }
    }
}

/// The category encoded by an LLVM YAML document tag.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RemarkKind {
    /// An optimization was applied.
    Passed,

    /// An optimization was not applied.
    Missed,

    /// The compiler reports analysis information.
    Analysis,

    /// The compiler reports a floating-point reassociation opportunity.
    AnalysisFpCommute,

    /// The compiler reports an alias-analysis result.
    AnalysisAliasing,

    /// An optimization failed after it started.
    Failure,

    /// LLVM emitted a tag that this Cargo Optic version does not classify.
    Unknown {
        /// The tag without its leading `!` marker.
        tag: String,
    },
}

/// A source location attached to a remark or one argument fragment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemarkSourceLocation {
    /// The source path reported by LLVM.
    pub file: String,

    /// The one-based source line.
    pub line: u64,

    /// The one-based source column.
    pub column: u64,
}

/// One ordered fragment of an LLVM optimization-remark message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemarkArgument {
    /// The LLVM argument key, such as `String`, `Callee`, or `Cost`.
    pub key: String,

    /// The printable scalar value for this fragment.
    pub value: String,

    /// The optional source location attached to this fragment.
    pub source_location: Option<RemarkSourceLocation>,
}

/// One typed LLVM optimization remark.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptimizationRemark {
    /// The category from the YAML document tag.
    pub kind: RemarkKind,

    /// The optimization pass that emitted this record.
    pub pass_name: String,

    /// The stable remark name within the pass.
    pub remark_name: String,

    /// The raw LLVM function symbol.
    pub function: String,

    /// The optional source location for the complete remark.
    pub source_location: Option<RemarkSourceLocation>,

    /// The optional profile hotness recorded by LLVM.
    pub hotness: Option<u64>,

    /// The ordered, typed fragments that form the message.
    pub arguments: Vec<RemarkArgument>,

    /// The printable message formed by concatenating argument values.
    pub message: String,
}

/// Parses one rustc LLVM optimization-remark YAML file.
pub fn parse_optimization_remarks(
    path: impl AsRef<Path>,
    limits: RemarkParseLimits,
) -> Result<Vec<OptimizationRemark>> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| Error::Filesystem {
        operation: "open",
        path: path.to_path_buf(),
        source,
    })?;
    let file_size = file
        .metadata()
        .map_err(|source| Error::Filesystem {
            operation: "read metadata for",
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if file_size > limits.max_file_bytes {
        return Err(invalid_remarks(
            path,
            format!(
                "file length exceeds {} bytes, got {file_size}",
                limits.max_file_bytes
            ),
        ));
    }

    let bounded_file = file.take(limits.max_file_bytes.saturating_add(1));
    let mut reader = BufReader::new(bounded_file);
    let mut parser = RemarkStreamParser::new(path, limits);
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes_read =
            reader
                .read_until(b'\n', &mut line)
                .map_err(|source| Error::Filesystem {
                    operation: "read",
                    path: path.to_path_buf(),
                    source,
                })?;
        if bytes_read == 0 {
            break;
        }

        parser.push_line(&line)?;
    }

    parser.finish()
}

struct RemarkStreamParser<'a> {
    path: &'a Path,
    limits: RemarkParseLimits,
    total_bytes: u64,
    saw_document: bool,
    document: Vec<u8>,
    records: Vec<OptimizationRemark>,
}

impl<'a> RemarkStreamParser<'a> {
    fn new(path: &'a Path, limits: RemarkParseLimits) -> Self {
        Self {
            path,
            limits,
            total_bytes: 0,
            saw_document: false,
            document: Vec::new(),
            records: Vec::new(),
        }
    }

    fn push_line(&mut self, line: &[u8]) -> Result<()> {
        self.total_bytes = self.total_bytes.saturating_add(line.len() as u64);
        if self.total_bytes > self.limits.max_file_bytes {
            return Err(invalid_remarks(
                self.path,
                format!(
                    "file length exceeds {} bytes, got {}",
                    self.limits.max_file_bytes, self.total_bytes
                ),
            ));
        }

        let marker = trim_ascii_end(line);
        if starts_document(marker) {
            if !self.document.is_empty() {
                self.finish_document()?;
            }

            self.saw_document = true;
            self.push_document_bytes(line)?;
            return Ok(());
        }

        if marker == DOCUMENT_END {
            if self.document.is_empty() {
                return Err(invalid_remarks(
                    self.path,
                    "document end marker does not follow a document",
                ));
            }

            self.push_document_bytes(line)?;
            return self.finish_document();
        }

        if self.document.is_empty() {
            let trimmed = trim_ascii(line);
            if trimmed.is_empty() || trimmed.starts_with(b"#") {
                return Ok(());
            }

            let position = if self.saw_document {
                "unexpected content follows a completed document"
            } else {
                "first document must start with `---`"
            };
            return Err(invalid_remarks(self.path, position));
        }

        self.push_document_bytes(line)
    }

    fn push_document_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let document_bytes = self.document.len().saturating_add(bytes.len());
        if document_bytes > self.limits.max_document_bytes {
            return Err(invalid_remarks(
                self.path,
                format!(
                    "document length exceeds {} bytes, got {document_bytes}",
                    self.limits.max_document_bytes
                ),
            ));
        }

        self.document.extend_from_slice(bytes);
        Ok(())
    }

    fn finish_document(&mut self) -> Result<()> {
        if self.records.len() >= self.limits.max_records {
            return Err(invalid_remarks(
                self.path,
                format!(
                    "record count exceeds {}, got {}",
                    self.limits.max_records,
                    self.records.len() + 1
                ),
            ));
        }

        let value: Value = serde_yaml::from_slice(&self.document).map_err(|source| {
            invalid_remarks(
                self.path,
                format!(
                    "document {} is invalid YAML: {source}",
                    self.records.len() + 1
                ),
            )
        })?;
        let record = parse_document(self.path, value, self.limits)?;

        self.records.push(record);
        self.document.clear();
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<OptimizationRemark>> {
        if !self.document.is_empty() {
            self.finish_document()?;
        }

        Ok(self.records)
    }
}

fn parse_document(
    path: &Path,
    value: Value,
    limits: RemarkParseLimits,
) -> Result<OptimizationRemark> {
    let Value::Tagged(tagged) = value else {
        return Err(invalid_remarks(
            path,
            "document root must have an LLVM remark tag",
        ));
    };
    let kind = RemarkKind::from_tag(tagged.tag.to_string(), path, limits)?;
    let mapping = expect_mapping(path, tagged.value, "document root")?;

    let pass_name = required_string(path, &mapping, "Pass", limits)?;
    let remark_name = required_string(path, &mapping, "Name", limits)?;
    let function = required_string(path, &mapping, "Function", limits)?;
    let source_location = optional_location(path, &mapping, "DebugLoc", limits)?;
    let hotness = optional_u64(path, &mapping, "Hotness")?;
    let arguments = parse_arguments(path, &mapping, limits)?;
    let message = render_message(path, &arguments, limits)?;

    Ok(OptimizationRemark {
        kind,
        pass_name,
        remark_name,
        function,
        source_location,
        hotness,
        arguments,
        message,
    })
}

impl RemarkKind {
    fn from_tag(tag: String, path: &Path, limits: RemarkParseLimits) -> Result<Self> {
        let tag = tag.trim_start_matches('!');
        ensure_string_limit(path, "remark tag", tag, limits)?;

        Ok(match tag {
            "Passed" => Self::Passed,
            "Missed" => Self::Missed,
            "Analysis" => Self::Analysis,
            "AnalysisFPCommute" => Self::AnalysisFpCommute,
            "AnalysisAliasing" => Self::AnalysisAliasing,
            "Failure" => Self::Failure,
            _ => Self::Unknown {
                tag: tag.to_owned(),
            },
        })
    }
}

fn parse_arguments(
    path: &Path,
    mapping: &Mapping,
    limits: RemarkParseLimits,
) -> Result<Vec<RemarkArgument>> {
    let Some(value) = mapping.get(Value::String("Args".to_owned())) else {
        return Ok(Vec::new());
    };
    let Value::Sequence(values) = value else {
        return Err(invalid_remarks(path, "`Args` must be a sequence"));
    };
    if values.len() > limits.max_arguments {
        return Err(invalid_remarks(
            path,
            format!(
                "argument count exceeds {}, got {}",
                limits.max_arguments,
                values.len()
            ),
        ));
    }

    values
        .iter()
        .enumerate()
        .map(|(index, value)| parse_argument(path, value, index, limits))
        .collect()
}

fn parse_argument(
    path: &Path,
    value: &Value,
    index: usize,
    limits: RemarkParseLimits,
) -> Result<RemarkArgument> {
    let Value::Mapping(mapping) = value else {
        return Err(invalid_remarks(
            path,
            format!("argument {} must be a mapping", index + 1),
        ));
    };
    let source_location = optional_location(path, mapping, "DebugLoc", limits)?;
    let mut fragments = mapping
        .iter()
        .filter(|(key, _)| key.as_str() != Some("DebugLoc"));
    let Some((key, value)) = fragments.next() else {
        return Err(invalid_remarks(
            path,
            format!("argument {} has no value fragment", index + 1),
        ));
    };
    if fragments.next().is_some() {
        return Err(invalid_remarks(
            path,
            format!("argument {} has more than one value fragment", index + 1),
        ));
    }

    let key = expect_string(path, key, "argument key", limits)?;
    let value = scalar_text(path, value, "argument value", limits)?;

    Ok(RemarkArgument {
        key,
        value,
        source_location,
    })
}

fn render_message(
    path: &Path,
    arguments: &[RemarkArgument],
    limits: RemarkParseLimits,
) -> Result<String> {
    let message = arguments
        .iter()
        .map(|argument| argument.value.as_str())
        .collect::<String>();
    ensure_string_limit(path, "rendered message", &message, limits)?;

    Ok(message)
}

fn required_string(
    path: &Path,
    mapping: &Mapping,
    key: &'static str,
    limits: RemarkParseLimits,
) -> Result<String> {
    let Some(value) = mapping.get(Value::String(key.to_owned())) else {
        return Err(invalid_remarks(
            path,
            format!("document must contain `{key}`"),
        ));
    };

    expect_string(path, value, key, limits)
}

fn expect_string(
    path: &Path,
    value: &Value,
    field: &str,
    limits: RemarkParseLimits,
) -> Result<String> {
    let Value::String(value) = value else {
        return Err(invalid_remarks(path, format!("`{field}` must be a string")));
    };
    ensure_string_limit(path, field, value, limits)?;

    Ok(value.clone())
}

fn scalar_text(
    path: &Path,
    value: &Value,
    field: &str,
    limits: RemarkParseLimits,
) -> Result<String> {
    let value = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_owned(),
        _ => return Err(invalid_remarks(path, format!("`{field}` must be a scalar"))),
    };
    ensure_string_limit(path, field, &value, limits)?;

    Ok(value)
}

fn optional_u64(path: &Path, mapping: &Mapping, key: &'static str) -> Result<Option<u64>> {
    let Some(value) = mapping.get(Value::String(key.to_owned())) else {
        return Ok(None);
    };
    let Value::Number(value) = value else {
        return Err(invalid_remarks(path, format!("`{key}` must be an integer")));
    };
    let Some(value) = value.as_u64() else {
        return Err(invalid_remarks(
            path,
            format!("`{key}` must be a non-negative integer"),
        ));
    };

    Ok(Some(value))
}

fn optional_location(
    path: &Path,
    mapping: &Mapping,
    key: &'static str,
    limits: RemarkParseLimits,
) -> Result<Option<RemarkSourceLocation>> {
    let Some(value) = mapping.get(Value::String(key.to_owned())) else {
        return Ok(None);
    };
    let location = expect_mapping(path, value.clone(), key)?;
    let file = required_string(path, &location, "File", limits)?;
    let line = required_u64(path, &location, "Line")?;
    let column = required_u64(path, &location, "Column")?;

    Ok(Some(RemarkSourceLocation { file, line, column }))
}

fn required_u64(path: &Path, mapping: &Mapping, key: &'static str) -> Result<u64> {
    let Some(value) = mapping.get(Value::String(key.to_owned())) else {
        return Err(invalid_remarks(
            path,
            format!("mapping must contain `{key}`"),
        ));
    };
    let Value::Number(value) = value else {
        return Err(invalid_remarks(path, format!("`{key}` must be an integer")));
    };
    let Some(value) = value.as_u64() else {
        return Err(invalid_remarks(
            path,
            format!("`{key}` must be a non-negative integer"),
        ));
    };

    Ok(value)
}

fn expect_mapping(path: &Path, value: Value, field: &str) -> Result<Mapping> {
    let Value::Mapping(mapping) = value else {
        return Err(invalid_remarks(
            path,
            format!("`{field}` must be a mapping"),
        ));
    };

    Ok(mapping)
}

fn ensure_string_limit(
    path: &Path,
    field: &str,
    value: &str,
    limits: RemarkParseLimits,
) -> Result<()> {
    if value.len() > limits.max_string_bytes {
        return Err(invalid_remarks(
            path,
            format!(
                "`{field}` length exceeds {} bytes, got {}",
                limits.max_string_bytes,
                value.len()
            ),
        ));
    }

    Ok(())
}

fn starts_document(line: &[u8]) -> bool {
    line == DOCUMENT_START
        || line
            .strip_prefix(DOCUMENT_START)
            .is_some_and(|rest| rest.first().is_some_and(u8::is_ascii_whitespace))
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);

    &bytes[start..end]
}

fn trim_ascii_end(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |index| index + 1);

    &bytes[..end]
}

fn invalid_remarks(path: &Path, message: impl Into<String>) -> Error {
    Error::InvalidOptimizationRemarks {
        path: PathBuf::from(path),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    const MULTIPLE_REMARKS: &str = r#"--- !Passed
Pass:            inline
Name:            Inlined
DebugLoc:        { File: src/lib.rs, Line: 12, Column: 5 }
Function:        _ZN4demo6caller17h0123456789abcdefE
Hotness:         42
Args:
  - Callee:          _ZN4demo6callee17hfedcba9876543210E
    DebugLoc:        { File: src/lib.rs, Line: 4, Column: 1 }
  - String:          ' inlined into '
  - Caller:          _ZN4demo6caller17h0123456789abcdefE
...
--- !Missed
Pass:            loop-vectorize
Name:            CantVectorize
Function:        _ZN4demo8unlinked17haaaaaaaaaaaaaaaaE
Args:
  - String:          loop not vectorized
...
"#;

    #[test]
    fn parses_multiple_linked_and_unlinked_records() {
        let parsed = parse_fixture(MULTIPLE_REMARKS, RemarkParseLimits::default()).unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].kind, RemarkKind::Passed);
        assert_eq!(parsed[0].pass_name, "inline");
        assert_eq!(parsed[0].remark_name, "Inlined");
        assert_eq!(parsed[0].function, "_ZN4demo6caller17h0123456789abcdefE");
        assert_eq!(parsed[0].hotness, Some(42));
        assert_eq!(parsed[0].arguments[0].key, "Callee");
        assert_eq!(
            parsed[0].arguments[0].source_location,
            Some(RemarkSourceLocation {
                file: "src/lib.rs".to_owned(),
                line: 4,
                column: 1,
            })
        );
        assert_eq!(
            parsed[0].message,
            "_ZN4demo6callee17hfedcba9876543210E inlined into _ZN4demo6caller17h0123456789abcdefE"
        );
        assert_eq!(parsed[1].kind, RemarkKind::Missed);
        assert_eq!(parsed[1].function, "_ZN4demo8unlinked17haaaaaaaaaaaaaaaaE");
    }

    #[test]
    fn preserves_unknown_document_tags() {
        let yaml = r#"--- !FutureRemark
Pass: future-pass
Name: FutureName
Function: _ZN4demo6future17hbbbbbbbbbbbbbbbbE
Args:
  - String: future message
...
"#;

        let parsed = parse_fixture(yaml, RemarkParseLimits::default()).unwrap();

        assert_eq!(
            parsed[0].kind,
            RemarkKind::Unknown {
                tag: "FutureRemark".to_owned(),
            }
        );
        assert_eq!(parsed[0].function, "_ZN4demo6future17hbbbbbbbbbbbbbbbbE");
        assert_eq!(parsed[0].message, "future message");
    }

    #[test]
    fn classifies_each_llvm_document_tag() {
        let cases = [
            ("Passed", RemarkKind::Passed),
            ("Missed", RemarkKind::Missed),
            ("Analysis", RemarkKind::Analysis),
            ("AnalysisFPCommute", RemarkKind::AnalysisFpCommute),
            ("AnalysisAliasing", RemarkKind::AnalysisAliasing),
            ("Failure", RemarkKind::Failure),
        ];

        for (tag, expected) in cases {
            let yaml = format!("--- !{tag}\nPass: pass\nName: name\nFunction: _Z8functionv\n...\n");

            let parsed = parse_fixture(&yaml, RemarkParseLimits::default()).unwrap();

            assert_eq!(parsed[0].kind, expected);
        }
    }

    #[test]
    fn rejects_malformed_yaml() {
        let yaml = "--- !Passed\nPass: [not closed\n...\n";

        let error = parse_fixture(yaml, RemarkParseLimits::default()).unwrap_err();

        assert!(error.to_string().contains("invalid YAML"));
    }

    #[test]
    fn rejects_oversized_file_document_string_and_record_count() {
        let yaml = r#"--- !Passed
Pass: inline
Name: Inlined
Function: _ZN4demo6caller17h0123456789abcdefE
Args:
  - String: message
...
"#;

        let cases = [
            RemarkParseLimits {
                max_file_bytes: (yaml.len() - 1) as u64,
                ..RemarkParseLimits::default()
            },
            RemarkParseLimits {
                max_document_bytes: yaml.len() - 1,
                ..RemarkParseLimits::default()
            },
            RemarkParseLimits {
                max_string_bytes: "message".len() - 1,
                ..RemarkParseLimits::default()
            },
            RemarkParseLimits {
                max_records: 0,
                ..RemarkParseLimits::default()
            },
        ];

        for limits in cases {
            assert!(parse_fixture(yaml, limits).is_err());
        }
    }

    #[test]
    fn rejects_content_after_document_end() {
        let yaml = r#"--- !Passed
Pass: inline
Name: Inlined
Function: _ZN4demo6caller17h0123456789abcdefE
...
not a document
"#;

        let error = parse_fixture(yaml, RemarkParseLimits::default()).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unexpected content follows a completed document")
        );
    }

    fn parse_fixture(contents: &str, limits: RemarkParseLimits) -> Result<Vec<OptimizationRemark>> {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("crate.opt.opt.yaml");
        fs::write(&path, contents).unwrap();

        parse_optimization_remarks(path, limits)
    }
}
