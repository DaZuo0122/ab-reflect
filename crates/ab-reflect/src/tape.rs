//! Tape representation and safe injection protocol.

use crate::error::{Error, Result};

/// The mutable string state of the interpreter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tape {
    content: String,
}

impl Tape {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.content
    }

    pub fn len(&self) -> usize {
        self.content.len()
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Find the first occurrence of `needle` in the tape.
    pub fn find(&self, needle: &str) -> Option<usize> {
        self.content.find(needle)
    }

    /// Replace the bytes `[start, end)` with `replacement`.
    pub fn replace_range(&mut self, start: usize, end: usize, replacement: &str) {
        self.content.replace_range(start..end, replacement);
    }

    /// Insert `text` at the start of the tape.
    pub fn insert_start(&mut self, text: &str) {
        self.content.insert_str(0, text);
    }

    /// Insert `text` at the end of the tape.
    pub fn insert_end(&mut self, text: &str) {
        self.content.push_str(text);
    }

    /// Append text without escaping (used internally when text is already safe).
    pub fn push_str(&mut self, text: &str) {
        self.content.push_str(text);
    }
}

impl From<String> for Tape {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

// ---------------------------------------------------------------------------
// Safe Injection Protocol
// ---------------------------------------------------------------------------

/// Escape a string so it can be safely injected into the Tape without
/// accidentally activating primitives or breaking boundary markers.
///
/// Escapes: `\` → `\\`, `<` → `\<`, `>` → `\>`.
pub fn safe_inject(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\<"),
            '>' => out.push_str("\\>"),
            _ => out.push(ch),
        }
    }
    out
}

/// Format a CLI argument for injection: ` <arg:ESCAPED>`.
pub fn format_arg(arg: &str) -> String {
    format!(" <arg:{}>", safe_inject(arg))
}

/// Format `\g` input for injection: `<ESCAPED>` or `<EOF>`.
pub fn format_g_input(input: Option<&str>) -> String {
    match input {
        Some(text) => format!("<{}>", safe_inject(text)),
        None => "<EOF>".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Escape Decoder (used by Parser and \r primitive)
// ---------------------------------------------------------------------------

/// Decode escape sequences per the Complete Escape Table.
/// Unknown escapes result in a ParseError.
pub fn decode_escapes(input: &str) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        let next = chars.next().ok_or_else(|| Error::Parse {
            line: 0,
            message: "trailing backslash at end of input".to_string(),
            column: None,
        })?;

        let decoded = match next {
            '\\' => '\\',
            '=' => '=',
            'n' => '\n',
            't' => '\t',
            '{' => '{',
            '}' => '}',
            '<' => '<',
            '>' => '>',
            '(' => '(',
            ')' => ')',
            'p' => '\x00', // sentinel for \p — preserved literally
            'P' => '\x01', // sentinel for \P — preserved literally
            'g' => '\x02', // sentinel for \g — preserved literally
            'G' => '\x03', // sentinel for \G — preserved literally
            'r' => '\x04', // sentinel for \r — preserved literally
            'R' => '\x05', // sentinel for \R — preserved literally
            'c' => '\x06', // sentinel for \c — preserved literally
            'C' => '\x07', // sentinel for \C — preserved literally
            _ => {
                return Err(Error::Parse {
                    line: 0,
                    message: format!("unknown escape sequence: \\{next}"),
                    column: None,
                })
            }
        };

        // For sentinel values, emit the literal two-char sequence instead.
        match decoded {
            '\x00' => out.push_str("\\p"),
            '\x01' => out.push_str("\\P"),
            '\x02' => out.push_str("\\g"),
            '\x03' => out.push_str("\\G"),
            '\x04' => out.push_str("\\r"),
            '\x05' => out.push_str("\\R"),
            '\x06' => out.push_str("\\c"),
            '\x07' => out.push_str("\\C"),
            c => out.push(c),
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Runtime Escape Encoder (used by \c primitive after downgrade)
// ---------------------------------------------------------------------------

/// Escape a string so it can be safely embedded in serialized rule text.
///
/// Escapes: `\` → `\\`, `<` → `\<`, `>` → `\>`.
///
/// **Note:** Primitive downgrading (`\p` → `\P`, etc.) is performed by the
/// `\c` handler using pattern-aware scanning *before* calling this function.
/// This encoder is intentionally simple and only handles literal escaping,
/// making it robust against arbitrary backslash runs.
pub fn escape_for_runtime(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\<"),
            '>' => out.push_str("\\>"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_inject_escapes_special_chars() {
        assert_eq!(safe_inject("a\\b<c>d"), "a\\\\b\\<c\\>d");
    }

    #[test]
    fn decode_basic_escapes() {
        let decoded = decode_escapes("hello\\nworld\\t!").unwrap();
        assert_eq!(decoded, "hello\nworld\t!");
    }

    #[test]
    fn decode_preserve_primitives() {
        let decoded = decode_escapes("\\p{hi}\\g\\r{A=B}\\c").unwrap();
        assert_eq!(decoded, "\\p{hi}\\g\\r{A=B}\\c");
    }

    #[test]
    fn decode_unknown_escape_fails() {
        assert!(decode_escapes("\\x").is_err());
    }

    #[test]
    fn decode_trailing_backslash_fails() {
        assert!(decode_escapes("foo\\").is_err());
    }

    #[test]
    fn escape_for_runtime_escapes_special_chars() {
        assert_eq!(escape_for_runtime(r"\<\>"), r"\\\<\\\>");
    }

    #[test]
    fn escape_for_runtime_handles_backslash_runs() {
        // Round-trip invariants for decoded tape content.
        assert_eq!(escape_for_runtime(r"\"), r"\\");        // 1 → 2
        assert_eq!(escape_for_runtime(r"\\"), r"\\\\");   // 2 → 4
        assert_eq!(escape_for_runtime(r"\\\"), r"\\\\\\"); // 3 → 6
    }

    #[test]
    fn escape_round_trips_with_decode() {
        let original = r"\<foo\>\\bar";
        let encoded = escape_for_runtime(original);
        let decoded = decode_escapes(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn tape_replace_range() {
        let mut tape = Tape::new("hello world");
        tape.replace_range(6, 11, "tape");
        assert_eq!(tape.as_str(), "hello tape");
    }

    #[test]
    fn tape_insert_start_and_end() {
        let mut tape = Tape::new("mid");
        tape.insert_start("[");
        tape.insert_end("]");
        assert_eq!(tape.as_str(), "[mid]");
    }
}
