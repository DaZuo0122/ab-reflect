//! Virtual machine: execution engine and main loop.

use crate::error::{Error, Result};
use crate::rule::{RhsPrefix, Rule};
use crate::tape::Tape;

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
                self.trace_step();
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
                        self.trace_step();
                        return Ok(());
                    }
                }

                self.check_tape_limit()?;
                self.trace_step();
                continue;
            }

            // Phase 3: Normal halt
            return Ok(());
        }
    }

    fn trace_step(&self) {
        let tape = self.tape.as_str();
        let display = tape
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\t', "\\t");
        let msg = format!("[step:{} fuel:{}] \"{display}\"", self.step_count, self.fuel);
        tracing::info!("{msg}");
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

        if rule.key.has_end {
            if let Some(start) = tape.rfind(lhs) {
                let end = start + lhs.len();
                if end == tape.len() {
                    return Some((start, end));
                }
            }
            return None;
        }

        tape.find(lhs).map(|start| (start, start + lhs.len()))
    }

    // -----------------------------------------------------------------------
    // Phase 1: Primitive Scanning & Execution
    // -----------------------------------------------------------------------

    fn execute_leftmost_primitive(&mut self) -> Result<Option<(usize, usize, String)>> {
        let tape = self.tape.as_str();
        let mut best: Option<(usize, usize, String)> = None;

        if let Some(res) = self.scan_block_primitive(tape, "\\p{") {
            best = Some(res);
        }
        if let Some(res) = self.scan_block_primitive(tape, "\\r{") {
            best = Self::leftmost(best, res);
        }
        if let Some((pos, end)) = Self::scan_simple_primitive(tape, "\\g") {
            let replacement = crate::primitives::execute_g()?;
            best = Self::leftmost(best, (pos, end, replacement));
        }
        if let Some((pos, end)) = Self::scan_simple_primitive(tape, "\\c") {
            let replacement = crate::primitives::execute_c(&self.rules)?;
            best = Self::leftmost(best, (pos, end, replacement));
        }

        if let Some((pos, end, _)) = best {
            if tape[pos..].starts_with("\\p{") {
                let replacement = crate::primitives::execute_p(&self.tape, pos, end)?;
                return Ok(Some((pos, end, replacement)));
            }
            if tape[pos..].starts_with("\\r{") {
                let replacement =
                    crate::primitives::execute_r(&self.tape, pos, end, &mut self.rules)?;
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

    fn scan_block_primitive(
        &self,
        tape: &str,
        prefix: &str,
    ) -> Option<(usize, usize, String)> {
        let mut search_from = 0;
        while let Some(pos) = tape[search_from..].find(prefix) {
            let abs_pos = search_from + pos;
            if abs_pos > 0 && tape.as_bytes()[abs_pos - 1] == b'\\' {
                search_from = abs_pos + 1;
                continue;
            }
            let after_brace = abs_pos + prefix.len();
            if let Some(end) = crate::primitives::find_unescaped_brace(&tape[after_brace..]) {
                let abs_end = after_brace + end + 1;
                return Some((abs_pos, abs_end, String::new()));
            }
            search_from = abs_pos + 1;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::{Rule, RuleKey};

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
        let rules = vec![
            Rule::new(RuleKey::new("world").with_end(), "!"),
            Rule::new(RuleKey::new("hello "), ""),
        ];
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
        let rules = vec![
            Rule::new(RuleKey::new("A"), "B"),
            Rule::new(RuleKey::new("B"), "A"),
        ];
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
}
