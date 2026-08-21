//! Renders structured command output and compiler text for a terminal.
//!
//! [`Terminal`] applies semantic styles to interface text. It applies syntax styles to compiler
//! evidence. Callers disable all styles for JSON, pipes, and terminals that request plain text.

use std::sync::OnceLock;

use anstyle::{AnsiColor, Style};
use two_face::re_exports::syntect::easy::HighlightLines;
use two_face::re_exports::syntect::highlighting::Theme;
use two_face::re_exports::syntect::parsing::{SyntaxReference, SyntaxSet};
use two_face::re_exports::syntect::util::{LinesWithEndings, as_24_bit_terminal_escaped};
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static THEMES: OnceLock<EmbeddedLazyThemeSet> = OnceLock::new();

#[derive(Clone, Copy)]
pub(crate) enum CodeSyntax {
    Rust,
    Llvm,
}

pub(crate) struct CodeHighlighter {
    state: Option<HighlightState>,
}

struct HighlightState {
    highlighter: HighlightLines<'static>,

    has_input: bool,

    pending_line: String,
}

pub(crate) struct Terminal {
    color: bool,
}

impl Terminal {
    pub(crate) const fn new(color: bool) -> Self {
        Self { color }
    }

    pub(crate) fn heading(&self, text: &str) -> String {
        self.paint(AnsiColor::BrightCyan.on_default().bold(), text)
    }

    pub(crate) fn label(&self, text: &str) -> String {
        self.paint(AnsiColor::BrightBlack.on_default(), text)
    }

    pub(crate) fn identifier(&self, text: &str, unique_prefix_length: usize) -> String {
        if !self.color {
            return text.to_owned();
        }

        let unique_prefix_length = unique_prefix_length.min(text.len());
        let (unique_prefix, remainder) = text.split_at(unique_prefix_length);
        let unique_prefix = self.paint(AnsiColor::BrightYellow.on_default().bold(), unique_prefix);

        if remainder.is_empty() {
            unique_prefix
        } else {
            format!(
                "{unique_prefix}{}",
                self.paint(AnsiColor::BrightBlack.on_default(), remainder)
            )
        }
    }

    pub(crate) fn function(&self, text: &str) -> String {
        self.paint(Style::new().bold(), text)
    }

    pub(crate) fn positive(&self, text: &str) -> String {
        self.paint(AnsiColor::BrightGreen.on_default(), text)
    }

    pub(crate) fn warning(&self, text: &str) -> String {
        self.paint(AnsiColor::BrightYellow.on_default(), text)
    }

    pub(crate) fn command(&self, text: &str) -> String {
        self.paint(AnsiColor::BrightCyan.on_default(), text)
    }

    pub(crate) fn command_with_identifier(
        &self,
        before: &str,
        identifier: &str,
        unique_prefix_length: usize,
        after: &str,
    ) -> String {
        let after = if after.is_empty() {
            String::new()
        } else {
            self.command(after)
        };

        format!(
            "{}{}{}",
            self.command(before),
            self.identifier(identifier, unique_prefix_length),
            after,
        )
    }

    pub(crate) fn code(&self, text: &str, syntax: CodeSyntax) -> String {
        let mut highlighter = self.code_highlighter(syntax);
        let mut output = highlighter.push(text);
        output.push_str(&highlighter.finish());

        output
    }

    pub(crate) fn code_highlighter(&self, syntax: CodeSyntax) -> CodeHighlighter {
        let state = if self.color {
            let syntaxes = syntaxes();
            find_syntax(syntaxes, syntax).map(|syntax| HighlightState {
                highlighter: HighlightLines::new(syntax, theme()),
                has_input: false,
                pending_line: String::new(),
            })
        } else {
            None
        };

        CodeHighlighter { state }
    }

    fn paint(&self, style: Style, text: &str) -> String {
        if self.color {
            format!("{style}{text}{style:#}")
        } else {
            text.to_owned()
        }
    }
}

impl CodeHighlighter {
    pub(crate) fn push(&mut self, text: &str) -> String {
        let Some(state) = &mut self.state else {
            return text.to_owned();
        };
        if text.is_empty() {
            return String::new();
        }
        state.has_input = true;
        state.pending_line.push_str(text);
        let Some(last_newline) = state.pending_line.rfind('\n') else {
            return String::new();
        };
        let complete = state
            .pending_line
            .drain(..=last_newline)
            .collect::<String>();

        state.highlight(&complete).unwrap_or(complete)
    }

    pub(crate) fn finish(&mut self) -> String {
        let Some(state) = &mut self.state else {
            return String::new();
        };
        if !state.has_input {
            return String::new();
        }
        state.has_input = false;
        let pending = std::mem::take(&mut state.pending_line);
        let mut output = state.highlight(&pending).unwrap_or(pending);
        output.push_str("\x1b[0m");

        output
    }
}

impl HighlightState {
    fn highlight(&mut self, text: &str) -> Option<String> {
        let syntaxes = syntaxes();
        let mut output = String::with_capacity(text.len());

        for line in LinesWithEndings::from(text) {
            let regions = self.highlighter.highlight_line(line, syntaxes).ok()?;
            output.push_str(&as_24_bit_terminal_escaped(&regions, false));
        }

        Some(output)
    }
}

fn syntaxes() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(two_face::syntax::extra_newlines)
}

fn find_syntax(syntaxes: &SyntaxSet, syntax: CodeSyntax) -> Option<&SyntaxReference> {
    match syntax {
        CodeSyntax::Rust => syntaxes.find_syntax_by_extension("rs"),
        CodeSyntax::Llvm => syntaxes.find_syntax_by_extension("ll"),
    }
}

fn theme() -> &'static Theme {
    THEMES
        .get_or_init(two_face::theme::extra)
        .get(EmbeddedThemeName::Base16OceanDark)
}

#[cfg(test)]
mod tests {
    use super::{CodeSyntax, SYNTAXES, Terminal, find_syntax};

    #[test]
    fn includes_rust_and_llvm_syntaxes() {
        let syntaxes = SYNTAXES.get_or_init(two_face::syntax::extra_newlines);

        assert!(find_syntax(syntaxes, CodeSyntax::Rust).is_some());
        assert!(find_syntax(syntaxes, CodeSyntax::Llvm).is_some());
    }

    #[test]
    fn highlights_only_the_unique_identifier_prefix() {
        let terminal = Terminal::new(true);
        let identifier = terminal.identifier("ins_853d3c84a9f7", 7);

        assert_eq!(
            identifier,
            "\x1b[1m\x1b[93mins_853\x1b[0m\x1b[90md3c84a9f7\x1b[0m"
        );
    }

    #[test]
    fn preserves_highlighting_across_partial_lines() {
        let terminal = Terminal::new(true);
        let text = "fn example() {\n    /* split\n       comment */\n}\n";
        let expected = terminal.code(text, CodeSyntax::Rust);
        let mut highlighter = terminal.code_highlighter(CodeSyntax::Rust);
        let mut actual = String::new();

        for chunk in [
            "fn example() {\n    /* sp",
            "lit\n       com",
            "ment */\n}\n",
        ] {
            actual.push_str(&highlighter.push(chunk));
        }
        actual.push_str(&highlighter.finish());

        assert_eq!(actual, expected);
    }
}
