//! Primitive handlers for \p, \g, \r, and \c.

use crate::error::Result;
use crate::parser;
use crate::rule::{RhsPrefix, Rule, RuleKey};
use crate::tape::{decode_escapes, escape_for_runtime, format_g_input, Tape};
use std::io::{self, Read};

// ---------------------------------------------------------------------------
// Block scanner helper
// ---------------------------------------------------------------------------

/// Find the first unescaped `}` in the text.
pub fn find_unescaped_brace(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut escaped = false;
    for (idx, &b) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' {
            escaped = true;
            continue;
        }
        if b == b'}' {
            return Some(idx);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Individual primitive executors
// ---------------------------------------------------------------------------

/// `\p{payload}` — print decoded payload + newline, return empty replacement.
pub fn execute_p(tape: &Tape, start: usize, end: usize) -> Result<String> {
    let text = tape.as_str();
    let prefix_len = "\\p{".len();
    let payload = &text[start + prefix_len..end - 1]; // exclude '}'
    let decoded = decode_escapes(payload)?;
    println!("{decoded}");
    Ok(String::new())
}

/// `\g` — read stdin to EOF, return formatted replacement.
pub fn execute_g() -> Result<String> {
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer)?;
    Ok(format_g_input(Some(&buffer)))
}

/// `\r{rule_string}` — parse decoded payload as a rule and upsert/delete.
pub fn execute_r(tape: &Tape, start: usize, end: usize, rules: &mut Vec<Rule>) -> Result<String> {
    let text = tape.as_str();
    let prefix_len = "\\r{".len();
    let payload = &text[start + prefix_len..end - 1];
    let decoded = decode_escapes(payload)?;

    if decoded.starts_with("(del)") {
        let key_text = &decoded[5..];
        let key = parse_rule_key(key_text)?;
        rules.retain(|r| r.key != key);
    } else {
        let new_rule = parser::parse_rule(&decoded, 0)?;
        if let Some(existing) = rules.iter_mut().find(|r| r.key == new_rule.key) {
            existing.rhs = new_rule.rhs;
            existing.rhs_prefix = new_rule.rhs_prefix;
            existing.used = false;
        } else {
            rules.push(new_rule);
        }
    }

    Ok(String::new())
}

/// `\c` — serialize current rules into a safe `<...>` string.
pub fn execute_c(rules: &[Rule]) -> Result<String> {
    let mut lines = Vec::new();
    for rule in rules {
        let mut line = String::new();
        if rule.key.has_start {
            line.push_str("(start)");
        }
        if rule.key.has_once {
            line.push_str("(once)");
        }
        line.push_str(&rule.key.lhs);
        if rule.key.has_end {
            line.push_str("(end)");
        }
        line.push('=');
        match rule.rhs_prefix {
            RhsPrefix::Start => line.push_str("(start)"),
            RhsPrefix::End => line.push_str("(end)"),
            RhsPrefix::Halt => line.push_str("(halt)"),
            RhsPrefix::Normal => {}
        }
        let downgraded = downgrade_primitives(&rule.rhs);
        let escaped = escape_for_runtime(&downgraded);
        line.push_str(&escaped);
        lines.push(line);
    }

    let inner = lines.join("\n");
    Ok(format!("<{inner}>"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_rule_key(text: &str) -> Result<RuleKey> {
    let dummy = format!("{text}=");
    let rule = parser::parse_rule(&dummy, 0)?;
    Ok(rule.key)
}

/// Scan text for active primitive patterns and downgrade them to lazy.
/// Treats `\` followed by `p`/`g`/`r`/`c` as active only when the `\`
/// itself is not escaped by another `\`.
pub fn downgrade_primitives(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some((pos, ch)) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        // Check if this `\` is escaped by another `\`
        if pos > 0 && text.as_bytes()[pos - 1] == b'\\' {
            out.push('\\');
            continue;
        }

        let remaining = &text[pos..];
        if remaining.starts_with("\\p{") {
            if let Some(end) = find_unescaped_brace(&remaining[3..]) {
                let block_end = 3 + end + 1;
                out.push_str("\\P{");
                out.push_str(&remaining[3..block_end - 1]);
                out.push('}');
                for _ in 0..block_end - 1 {
                    chars.next();
                }
                continue;
            }
        }
        if remaining.starts_with("\\r{") {
            if let Some(end) = find_unescaped_brace(&remaining[3..]) {
                let block_end = 3 + end + 1;
                out.push_str("\\R{");
                out.push_str(&remaining[3..block_end - 1]);
                out.push('}');
                for _ in 0..block_end - 1 {
                    chars.next();
                }
                continue;
            }
        }
        if remaining.starts_with("\\g") {
            out.push_str("\\G");
            chars.next();
            continue;
        }
        if remaining.starts_with("\\c") {
            out.push_str("\\C");
            chars.next();
            continue;
        }

        out.push('\\');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downgrade_primitives_basic() {
        let input = r"\p{hi}\g\r{A=B}\c";
        let out = downgrade_primitives(input);
        assert_eq!(out, r"\P{hi}\G\R{A=B}\C");
    }

    #[test]
    fn downgrade_skips_escaped() {
        let input = r"\\p{hi}";
        let out = downgrade_primitives(input);
        assert_eq!(out, r"\\p{hi}");
    }

    #[test]
    fn downgrade_preserves_literal_backslash() {
        let input = r"\x";
        let out = downgrade_primitives(input);
        assert_eq!(out, r"\x");
    }

    #[test]
    fn find_unescaped_brace_cases() {
        assert_eq!(find_unescaped_brace("hi}"), Some(2));
        assert_eq!(find_unescaped_brace("hi\\}"), None);
        assert_eq!(find_unescaped_brace("hi\\}foo}"), Some(7));
    }
}
