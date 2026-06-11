//! Virtual machine: execution engine and main loop.

use crate::error::{Error, Result};
use crate::rule::{RhsPrefix, Rule, RuleKey};
use crate::tape::{escape_for_runtime, format_g_input, Tape};
use std::io::{self, Read};

/// Execution engine for A=B^Reflect.
pub struct Vm {
    pub tape: Tape,
    pub rules: Vec<Rule>,
    fuel: u64,
    tape_limit: usize,
    step_count: u64,
}

impl Vm {
    pub fn new(tape: Tape, rules: Vec<Rule>, fuel: u64, tape_limit: usize) -> Self {
        Self {
            tape,
            rules,
            fuel,
            tape_limit,
            step_count: 0,
        }
    }

    /// Run the VM until halt or error.
    pub fn run(&mut self) -> Result<()> {
        loop {
            self.step_count += 1;

            if self.fuel == 0 {
                return Err(Error::FuelExceeded {
                    steps: self.step_count,
                });
            }
            self.fuel -= 1;

            // Phase 1: Primitive interception (highest priority)
            if let Some((start, end, replacement)) = self.execute_leftmost_primitive()? {
                self.tape.replace_range(start, end, &replacement);
                self.check_tape_limit()?;
                continue;
            }

            // Phase 2: Markov rule application
            if let Some((rule_idx, match_start, match_end)) = self.find_first_rule_match() {
                let rule = &mut self.rules[rule_idx];
                let rhs = rule.rhs.clone();
                let prefix = rule.rhs_prefix;

                if rule.key.has_once {
                    rule.used = true;
                }

                match prefix {
                    RhsPrefix::Normal => {
                        self.tape.replace_range(match_start, match_end, &rhs);
                    }
                    RhsPrefix::Start => {
                        self.tape.replace_range(match_start, match_end, "");
                        self.tape.insert_start(&rhs);
                    }
                    RhsPrefix::End => {
                        self.tape.replace_range(match_start, match_end, "");
                        self.tape.insert_end(&rhs);
                    }
                    RhsPrefix::Halt => {
                        self.tape.replace_range(match_start, match_end, &rhs);
                        return Ok(());
                    }
                }

                self.check_tape_limit()?;
                continue;
            }

            // Phase 3: Normal halt
            return Ok(());
        }
    }

    fn check_tape_limit(&self) -> Result<()> {
        let len = self.tape.len();
        if len > self.tape_limit {
            Err(Error::TapeOverflow {
                len,
                limit: self.tape_limit,
            })
        } else {
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Phase 2: Rule Matching
    // -----------------------------------------------------------------------

    fn find_first_rule_match(&self) -> Option<(usize, usize, usize)> {
        for (idx, rule) in self.rules.iter().enumerate() {
            if rule.key.has_once && rule.used {
                continue;
            }

            if let Some((start, end)) = self.match_rule(rule) {
                return Some((idx, start, end));
            }
        }
        None
    }

    fn match_rule(&self, rule: &Rule) -> Option<(usize, usize)> {
        let tape = self.tape.as_str();
        let lhs = &rule.key.lhs;

        // (start): must match at position 0
        if rule.key.has_start {
            if tape.starts_with(lhs) {
                let end = lhs.len();
                if rule.key.has_end {
                    if end == tape.len() {
                        return Some((0, end));
                    }
                } else {
                    return Some((0, end));
                }
            }
            return None;
        }

        // (end): must match at end of tape
        if rule.key.has_end {
            if let Some(start) = tape.rfind(lhs) {
                let end = start + lhs.len();
                if end == tape.len() {
                    return Some((start, end));
                }
            }
            return None;
        }

        // Normal: first match anywhere
        tape.find(lhs).map(|start| (start, start + lhs.len()))
    }

    // -----------------------------------------------------------------------
    // Phase 1: Primitive Scanning & Execution
    // -----------------------------------------------------------------------

    fn execute_leftmost_primitive(&mut self) -> Result<Option<(usize, usize, String)>> {
        let tape = self.tape.as_str();
        let mut best: Option<(usize, usize, String)> = None;

        // Scan for all four primitive families and keep the leftmost.
        if let Some(res) = self.scan_block_primitive(tape, "\\p{") {
            best = Some(res);
        }
        if let Some(res) = self.scan_block_primitive(tape, "\\r{") {
            best = Self::leftmost(best, res);
        }
        if let Some((pos, end)) = Self::scan_simple_primitive(tape, "\\g") {
            let replacement = self.exec_g()?;
            best = Self::leftmost(best, (pos, end, replacement));
        }
        if let Some((pos, end)) = Self::scan_simple_primitive(tape, "\\c") {
            let replacement = self.exec_c()?;
            best = Self::leftmost(best, (pos, end, replacement));
        }

        // If we found \p or \r, execute them now that we know they're leftmost.
        if let Some((pos, end, _)) = best {
            if tape[pos..].starts_with("\\p{") {
                let replacement = self.exec_p(pos, end)?;
                return Ok(Some((pos, end, replacement)));
            }
            if tape[pos..].starts_with("\\r{") {
                let replacement = self.exec_r(pos, end)?;
                return Ok(Some((pos, end, replacement)));
            }
        }

        Ok(best)
    }

    fn leftmost(
        a: Option<(usize, usize, String)>,
        b: (usize, usize, String),
    ) -> Option<(usize, usize, String)> {
        match a {
            Some((pa, ..)) if pa <= b.0 => a,
            _ => Some(b),
        }
    }

    /// Scan for a block primitive (`\p{...}` or `\r{...}`).
    /// Returns `(start, end, placeholder)` where the caller will execute.
    fn scan_block_primitive(
        &self,
        tape: &str,
        prefix: &str,
    ) -> Option<(usize, usize, String)> {
        let mut search_from = 0;
        while let Some(pos) = tape[search_from..].find(prefix) {
            let abs_pos = search_from + pos;
            // Ensure the `\` is not itself escaped.
            if abs_pos > 0 && tape.as_bytes()[abs_pos - 1] == b'\\' {
                search_from = abs_pos + 1;
                continue;
            }
            let after_brace = abs_pos + prefix.len();
            if let Some(end) = Self::find_unescaped_brace(&tape[after_brace..]) {
                let abs_end = after_brace + end + 1; // +1 to include `}`
                return Some((abs_pos, abs_end, String::new()));
            }
            search_from = abs_pos + 1;
        }
        None
    }

    fn find_unescaped_brace(text: &str) -> Option<usize> {
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

    fn scan_simple_primitive(tape: &str, pattern: &str) -> Option<(usize, usize)> {
        let mut search_from = 0;
        while let Some(pos) = tape[search_from..].find(pattern) {
            let abs_pos = search_from + pos;
            if abs_pos > 0 && tape.as_bytes()[abs_pos - 1] == b'\\' {
                search_from = abs_pos + 1;
                continue;
            }
            return Some((abs_pos, abs_pos + pattern.len()));
        }
        None
    }

    // -----------------------------------------------------------------------
    // Primitive Implementations
    // -----------------------------------------------------------------------

    fn exec_p(&self, start: usize, end: usize) -> Result<String> {
        let tape = self.tape.as_str();
        let prefix_len = "\\p{".len();
        let payload = &tape[start + prefix_len..end - 1]; // exclude '}'

        // Decode payload escapes before printing
        let decoded = crate::tape::decode_escapes(payload)?;
        println!("{decoded}");

        Ok(String::new())
    }

    fn exec_g(&self) -> Result<String> {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(format_g_input(Some(&buffer)))
    }

    fn exec_r(&mut self, start: usize, end: usize) -> Result<String> {
        let tape = self.tape.as_str();
        let prefix_len = "\\r{".len();
        let payload = &tape[start + prefix_len..end - 1];

        let decoded = crate::tape::decode_escapes(payload)?;

        if decoded.starts_with("(del)") {
            let key_text = &decoded[5..];
            let key = parse_rule_key(key_text)?;
            self.rules.retain(|r| r.key != key);
        } else {
            let new_rule = crate::parser::parse_rule(&decoded, 0)?;
            // Upsert: overwrite existing or append
            if let Some(existing) = self.rules.iter_mut().find(|r| r.key == new_rule.key) {
                existing.rhs = new_rule.rhs;
                existing.rhs_prefix = new_rule.rhs_prefix;
                existing.used = false;
            } else {
                self.rules.push(new_rule);
            }
        }

        Ok(String::new())
    }

    fn exec_c(&self) -> Result<String> {
        let mut lines = Vec::new();
        for rule in &self.rules {
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_rule_key(text: &str) -> Result<RuleKey> {
    let dummy = format!("{text}=");
    let rule = crate::parser::parse_rule(&dummy, 0)?;
    Ok(rule.key)
}

/// Scan text for active primitive patterns and downgrade them to lazy.
fn downgrade_primitives(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some((pos, ch)) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        // Check if this `\` is escaped by another `\`
        if pos > 0 {
            let prev = text.as_bytes()[pos - 1];
            if prev == b'\\' {
                out.push('\\');
                continue;
            }
        }

        // Look ahead for primitive patterns
        let remaining = &text[pos..];
        if remaining.starts_with("\\p{") {
            if let Some(end) = Vm::find_unescaped_brace(&remaining[3..]) {
                let block_end = 3 + end + 1;
                out.push_str("\\P{");
                out.push_str(&remaining[3..block_end - 1]);
                out.push('}');
                // Skip consumed chars
                for _ in 0..block_end - 1 {
                    chars.next();
                }
                continue;
            }
        }
        if remaining.starts_with("\\r{") {
            if let Some(end) = Vm::find_unescaped_brace(&remaining[3..]) {
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
            chars.next(); // consume 'g'
            continue;
        }
        if remaining.starts_with("\\c") {
            out.push_str("\\C");
            chars.next(); // consume 'c'
            continue;
        }

        // Not a primitive we downgrade
        out.push('\\');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_simple_rule_replacement() {
        let tape = Tape::new("hello");
        let rules = vec![Rule::new(RuleKey::new("hello"), "world")];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "world");
    }

    #[test]
    fn vm_start_modifier() {
        let tape = Tape::new("hello world");
        let rules = vec![Rule::new(RuleKey::new("hello").with_start(), "hi")];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "hi world");
    }

    #[test]
    fn vm_end_modifier() {
        let tape = Tape::new("hello world");
        let rules = vec![Rule::new(RuleKey::new("world").with_end(), "!"),
                         Rule::new(RuleKey::new("hello "), "")];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "!");
    }

    #[test]
    fn vm_once_modifier() {
        let tape = Tape::new("AA");
        let rules = vec![Rule::new(RuleKey::new("A").with_once(), "B")];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        // Only first A replaced, second A remains because rule is used
        assert_eq!(vm.tape.as_str(), "BA");
    }

    #[test]
    fn vm_rhs_start_prefix() {
        let tape = Tape::new("X");
        let rules = vec![Rule::new(RuleKey::new("X"), "Y").with_rhs_prefix(RhsPrefix::Start)];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "Y");
    }

    #[test]
    fn vm_rhs_end_prefix() {
        let tape = Tape::new("X");
        let rules = vec![Rule::new(RuleKey::new("X"), "Y").with_rhs_prefix(RhsPrefix::End)];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "Y");
    }

    #[test]
    fn vm_fuel_exhaustion() {
        let tape = Tape::new("A");
        let rules = vec![Rule::new(RuleKey::new("A"), "B"),
                         Rule::new(RuleKey::new("B"), "A")];
        let mut vm = Vm::new(tape, rules, 5, 1024 * 1024);
        assert!(matches!(vm.run(), Err(Error::FuelExceeded { .. })));
    }

    #[test]
    fn vm_tape_overflow() {
        let tape = Tape::new("A");
        let rules = vec![Rule::new(RuleKey::new("A"), "BBBB")];
        let mut vm = Vm::new(tape, rules, 100, 3);
        assert!(matches!(vm.run(), Err(Error::TapeOverflow { .. })));
    }

    #[test]
    fn downgrade_primitives_basic() {
        let input = r"\p{hi}\g\r{A=B}\c";
        let out = downgrade_primitives(input);
        assert_eq!(out, r"\P{hi}\G\R{A=B}\C");
    }

    #[test]
    fn downgrade_skips_escaped() {
        // `\\p{hi}` is literal backslash then `p{hi}` (not a primitive)
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
    fn find_unescaped_brace() {
        assert_eq!(Vm::find_unescaped_brace("hi}"), Some(2));
        assert_eq!(Vm::find_unescaped_brace("hi\\}"), None);
        assert_eq!(Vm::find_unescaped_brace("hi\\}foo}"), Some(7));
    }
}
