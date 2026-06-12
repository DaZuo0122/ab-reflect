//! Source code parser: converts raw text into a Vec<Rule>.

use crate::error::{Error, Result};
use crate::rule::{RhsPrefix, Rule, RuleKey};

/// The result of parsing a source file.
#[derive(Clone, Debug, Default)]
pub struct ParsedProgram {
    pub rules: Vec<Rule>,
    pub fuel: Option<u64>,
    pub tape_limit: Option<usize>,
}

impl ParsedProgram {
    /// Returns the LHS text of the first rule, which becomes the initial tape.
    pub fn initial_tape(&self) -> Option<&str> {
        self.rules.first().map(|r| r.key.lhs.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineKind<'a> {
    Shebang,
    Comment,
    Pragma(&'a str),
    Blank,
    Rule(&'a str),
}

/// Parse a complete source file.
pub fn parse(source: &str) -> Result<ParsedProgram> {
    let mut program = ParsedProgram::default();

    for (line_no, line) in source.lines().enumerate() {
        let line_no = line_no + 1; // 1-based
        match classify_line(line) {
            LineKind::Shebang | LineKind::Comment | LineKind::Blank => continue,
            LineKind::Pragma(text) => {
                let (key, value) = parse_pragma(text, line_no)?;
                apply_pragma(&mut program, &key, &value, line_no)?;
            }
            LineKind::Rule(text) => {
                program.rules.push(parse_rule(text, line_no)?);
            }
        }
    }

    Ok(program)
}

fn classify_line(line: &str) -> LineKind<'_> {
    let trimmed = trim_leading_whitespace(line);

    if trimmed.starts_with("#!") {
        LineKind::Shebang
    } else if let Some(rest) = trimmed.strip_prefix("# @") {
        LineKind::Pragma(rest)
    } else if trimmed.starts_with('#') {
        LineKind::Comment
    } else if trimmed.is_empty() {
        LineKind::Blank
    } else {
        LineKind::Rule(line)
    }
}

fn trim_leading_whitespace(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_whitespace())
}

fn parse_pragma(text: &str, line_no: usize) -> Result<(String, String)> {
    let trimmed = trim_leading_whitespace(text);
    let Some(eq_pos) = trimmed.find('=') else {
        return Err(Error::Parse {
            line: line_no,
            message: "pragma missing '=' separator".to_string(),
            column: None,
        });
    };

    let key = trim_leading_whitespace(&trimmed[..eq_pos]).trim_end().to_string();
    let value = &trimmed[eq_pos + 1..];
    Ok((key, value.to_string()))
}

fn apply_pragma(program: &mut ParsedProgram, key: &str, value: &str, line_no: usize) -> Result<()> {
    match key {
        "fuel" => {
            let n = parse_u64(value, line_no, "fuel")?;
            program.fuel = Some(n);
        }
        "tape-limit" => {
            let n = parse_u64(value, line_no, "tape-limit")?;
            program.tape_limit = Some(n as usize);
        }
        _ => {
            return Err(Error::Parse {
                line: line_no,
                message: format!("unknown pragma key: {key}"),
                column: None,
            })
        }
    }
    Ok(())
}

fn parse_u64(value: &str, line_no: usize, name: &str) -> Result<u64> {
    value.trim().parse().map_err(|_| Error::Parse {
        line: line_no,
        message: format!("invalid value for @{name}: expected non-negative integer"),
        column: None,
    })
}

pub fn parse_rule(line: &str, line_no: usize) -> Result<Rule> {
    let (has_start, has_once, rest) = extract_lhs_prefixes(line, line_no)?;

    let Some((lhs_raw, rhs_raw)) = split_unescaped_equal(rest) else {
        return Err(Error::Parse {
            line: line_no,
            message: "rule has no unescaped '='".to_string(),
            column: None,
        });
    };

    let (has_end, lhs_text) = extract_lhs_suffix(lhs_raw, line_no)?;
    let (rhs_prefix, rhs_text) = extract_rhs_prefix(rhs_raw, line_no)?;

    validate_escapes(lhs_text, line_no)?;
    validate_escapes(rhs_text, line_no)?;

    // The rule is stored in escaped source representation, matching the Tape.
    let key = RuleKey {
        lhs: lhs_text.to_string(),
        has_start,
        has_end,
        has_once,
    };

    Ok(Rule {
        key,
        rhs: rhs_text.to_string(),
        rhs_prefix,
        used: false,
    })
}

fn extract_lhs_prefixes(line: &str, line_no: usize) -> Result<(bool, bool, &str)> {
    let mut has_start = false;
    let mut has_once = false;
    let mut rest = line;

    loop {
        if let Some(tail) = rest.strip_prefix("(start)") {
            if has_start {
                return Err(Error::Parse {
                    line: line_no,
                    message: "duplicate (start) prefix".to_string(),
                    column: None,
                });
            }
            has_start = true;
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("(once)") {
            if has_once {
                return Err(Error::Parse {
                    line: line_no,
                    message: "duplicate (once) prefix".to_string(),
                    column: None,
                });
            }
            has_once = true;
            rest = tail;
        } else {
            break;
        }
    }

    // After known prefixes, a remaining '(' indicates an unknown modifier.
    if rest.starts_with('(') {
        return Err(Error::Parse {
            line: line_no,
            message: "unknown or misplaced modifier at start of LHS".to_string(),
            column: None,
        });
    }

    Ok((has_start, has_once, rest))
}

fn extract_lhs_suffix(lhs_raw: &str, line_no: usize) -> Result<(bool, &str)> {
    if let Some(lhs) = lhs_raw.strip_suffix("(end)") {
        if lhs.ends_with("(end)") {
            return Err(Error::Parse {
                line: line_no,
                message: "duplicate (end) suffix".to_string(),
                column: None,
            });
        }
        Ok((true, lhs))
    } else {
        Ok((false, lhs_raw))
    }
}

fn extract_rhs_prefix(rhs_raw: &str, _line_no: usize) -> Result<(RhsPrefix, &str)> {
    if let Some(tail) = rhs_raw.strip_prefix("(start)") {
        Ok((RhsPrefix::Start, tail))
    } else if let Some(tail) = rhs_raw.strip_prefix("(end)") {
        Ok((RhsPrefix::End, tail))
    } else if let Some(tail) = rhs_raw.strip_prefix("(halt)") {
        Ok((RhsPrefix::Halt, tail))
    } else {
        Ok((RhsPrefix::Normal, rhs_raw))
    }
}

/// Split a string at the first unescaped '=' character.
/// Returns `None` if no unescaped '=' exists.
fn split_unescaped_equal(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
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
        if b == b'=' {
            return Some((&s[..idx], &s[idx + 1..]));
        }
    }

    None
}

/// Validate that all escape sequences in the text are known.
/// Does not mutate the text — rules are stored escaped.
fn validate_escapes(text: &str, line_no: usize) -> Result<()> {
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            continue;
        }
        let next = chars.next().ok_or_else(|| Error::Parse {
            line: line_no,
            message: "trailing backslash in rule text".to_string(),
            column: None,
        })?;
        match next {
            '\\' | '=' | 'n' | 't' | '{' | '}' | '<' | '>' | '(' | ')' | 'p' | 'P' | 'g'
            | 'G' | 'r' | 'R' | 'c' | 'C' => {}
            _ => {
                return Err(Error::Parse {
                    line: line_no,
                    message: format!("unknown escape sequence: \\{next}"),
                    column: None,
                })
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_rule() {
        let program = parse("A=B\n").unwrap();
        assert_eq!(program.rules.len(), 1);
        let rule = &program.rules[0];
        assert_eq!(rule.key.lhs, "A");
        assert_eq!(rule.rhs, "B");
        assert_eq!(rule.rhs_prefix, RhsPrefix::Normal);
    }

    #[test]
    fn parse_preserves_whitespace() {
        let program = parse("  A  =  B  \n").unwrap();
        let rule = &program.rules[0];
        assert_eq!(rule.key.lhs, "  A  ");
        assert_eq!(rule.rhs, "  B  ");
    }

    #[test]
    fn parse_unescaped_equal() {
        let program = parse(r"A\=B=C").unwrap();
        let rule = &program.rules[0];
        assert_eq!(rule.key.lhs, r"A\=B");
        assert_eq!(rule.rhs, "C");
    }

    #[test]
    fn parse_modifiers() {
        let program = parse("(start)(once)A(end)=(halt)B").unwrap();
        let rule = &program.rules[0];
        assert!(rule.key.has_start);
        assert!(rule.key.has_once);
        assert!(rule.key.has_end);
        assert_eq!(rule.rhs_prefix, RhsPrefix::Halt);
        assert_eq!(rule.key.lhs, "A");
        assert_eq!(rule.rhs, "B");
    }

    #[test]
    fn parse_rhs_prefixes_mutually_exclusive() {
        // Multiple RHS prefixes would be parsed sequentially; the second is kept as text.
        // The spec says they are mutually exclusive, but the syntax itself makes the first
        // prefix consume the token. We intentionally do not error here because `(start)(end)`
        // as text after a prefix is valid data.
        let program = parse("A=(start)(end)B").unwrap();
        let rule = &program.rules[0];
        assert_eq!(rule.rhs_prefix, RhsPrefix::Start);
        assert_eq!(rule.rhs, "(end)B");
    }

    #[test]
    fn parse_pragma() {
        let program = parse("# @fuel=1000\n# @tape-limit=64\nA=B\n").unwrap();
        assert_eq!(program.fuel, Some(1000));
        assert_eq!(program.tape_limit, Some(64));
        assert_eq!(program.rules.len(), 1);
    }

    #[test]
    fn parse_skips_comments_and_blanks() {
        let program = parse("# comment\n\nA=B\n#!shebang\n").unwrap();
        assert_eq!(program.rules.len(), 1);
        assert_eq!(program.initial_tape(), Some("A"));
    }

    #[test]
    fn parse_duplicate_prefix_errors() {
        assert!(parse("(once)(once)A=B").is_err());
    }

    #[test]
    fn parse_duplicate_end_suffix_errors() {
        assert!(parse("A(end)(end)=B").is_err());
    }

    #[test]
    fn parse_unknown_escape_errors() {
        assert!(parse("A=\\x").is_err());
    }

    #[test]
    fn parse_no_equal_errors() {
        assert!(parse("AB").is_err());
    }
}
