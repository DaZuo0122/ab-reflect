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
            // Phase 1: Primitive interception (highest priority)
            if let Some((start, end, replacement)) = self.execute_leftmost_primitive()? {
                self.consume_fuel()?;
                self.tape.replace_range(start, end, &replacement);
                self.check_tape_limit()?;
                self.trace_step();
                continue;
            }

            // Phase 2: Markov rule application
            if let Some((rule_idx, match_start, match_end)) = self.find_first_rule_match() {
                self.consume_fuel()?;
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
                        self.check_tape_limit()?;
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
        let boundaries = Self::escape_boundaries(self.tape.as_str());
        for (idx, rule) in self.rules.iter().enumerate() {
            if rule.key.has_once && rule.used {
                continue;
            }
            if let Some((start, end)) = self.match_rule(rule, &boundaries) {
                return Some((idx, start, end));
            }
        }
        None
    }

    fn match_rule(&self, rule: &Rule, boundaries: &[bool]) -> Option<(usize, usize)> {
        let tape = self.tape.as_str();
        let lhs = &rule.key.lhs;

        if rule.key.has_start {
            if tape.starts_with(lhs) {
                let end = lhs.len();
                if boundaries[end] && (!rule.key.has_end || end == tape.len()) {
                    return Some((0, end));
                }
            }
            return None;
        }

        if rule.key.has_end {
            if let Some(start) = tape.rfind(lhs) {
                let end = start + lhs.len();
                if end == tape.len() && boundaries[start] {
                    return Some((start, end));
                }
            }
            return None;
        }

        // Plain match: the match must start and end on escape-token boundaries
        // so that rules cannot match inside \p, \r, \\, etc.
        let mut search_from = 0;
        while let Some(start) = tape[search_from..].find(lhs) {
            let abs_start = search_from + start;
            let end = abs_start + lhs.len();
            if boundaries[abs_start] && boundaries[end] {
                return Some((abs_start, end));
            }
            search_from = Self::next_char_boundary(tape, abs_start);
        }
        None
    }

    fn next_char_boundary(text: &str, pos: usize) -> usize {
        match text[pos..].chars().next() {
            Some(ch) => pos + ch.len_utf8(),
            None => text.len(),
        }
    }

    fn consume_fuel(&mut self) -> Result<()> {
        if self.fuel == 0 {
            return Err(Error::FuelExceeded {
                steps: self.step_count,
            });
        }

        self.fuel -= 1;
        self.step_count += 1;
        Ok(())
    }

    /// Compute the set of byte positions in `tape` that are the start of an
    /// indivisible escaped-source token.  A token is either:
    /// - a normal character,
    /// - a two-character escape sequence `\X` (including `\\`), or
    /// - a `<...>` capsule produced by `\c` or the safe-injection protocol.
    ///
    /// Rule matches must begin and end on these boundaries so that rules cannot
    /// match inside primitive escape sequences such as `\C` or `\r{...}`, nor
    /// inside serialized/introspection capsules.
    fn escape_boundaries(tape: &str) -> Vec<bool> {
        let mut boundaries = vec![false; tape.len() + 1];
        boundaries[0] = true;

        let mut chars = tape.char_indices().peekable();
        while let Some((pos, ch)) = chars.next() {
            if !boundaries[pos] {
                // We are at a char index that is not a token boundary (should
                // not happen when iterating char_indices, but stay defensive).
                continue;
            }
            if ch == '\\' {
                if let Some(&(next_pos, next_ch)) = chars.peek() {
                    let next_end = next_pos + next_ch.len_utf8();
                    boundaries[next_end] = true;
                    chars.next();
                } else {
                    // Trailing backslash: treat it as a single-byte token.
                    boundaries[tape.len()] = true;
                }
            } else if ch == '<' {
                // A `<...>` capsule is a single indivisible token.
                if let Some(gt_pos) = Self::find_unescaped_gt(tape, pos + 1) {
                    let end = gt_pos + 1;
                    boundaries[end] = true;
                    // Skip over the capsule contents.
                    let skip = end;
                    while let Some(&(p, _)) = chars.peek() {
                        if p >= skip {
                            break;
                        }
                        chars.next();
                    }
                } else {
                    boundaries[pos + ch.len_utf8()] = true;
                }
            } else {
                boundaries[pos + ch.len_utf8()] = true;
            }
        }

        boundaries[tape.len()] = true;
        boundaries
    }

    /// Find the next unescaped `>` in `tape` starting at `start`.
    fn find_unescaped_gt(tape: &str, start: usize) -> Option<usize> {
        let bytes = tape.as_bytes();
        let mut escaped = false;
        for idx in start..bytes.len() {
            if escaped {
                escaped = false;
                continue;
            }
            let b = bytes[idx];
            if b == b'\\' {
                escaped = true;
            } else if b == b'>' {
                return Some(idx);
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Phase 1: Primitive Scanning & Execution
    // -----------------------------------------------------------------------

    fn execute_leftmost_primitive(&mut self) -> Result<Option<(usize, usize, String)>> {
        #[derive(Clone, Copy, Debug)]
        enum Kind {
            P,
            R,
            G,
            C,
        }

        let tape = self.tape.as_str();
        let mut best: Option<(usize, usize, Kind)> = None;

        if let Some((pos, end)) = self.scan_block_primitive(tape, "\\p{") {
            best = Some((pos, end, Kind::P));
        }
        if let Some((pos, end)) = self.scan_block_primitive(tape, "\\r{") {
            best = Self::leftmost_kind(best, (pos, end, Kind::R));
        }
        if let Some((pos, end)) = Self::scan_simple_primitive(tape, "\\g") {
            best = Self::leftmost_kind(best, (pos, end, Kind::G));
        }
        if let Some((pos, end)) = Self::scan_simple_primitive(tape, "\\c") {
            best = Self::leftmost_kind(best, (pos, end, Kind::C));
        }

        if let Some((pos, end, kind)) = best {
            let replacement = match kind {
                Kind::P => crate::primitives::execute_p(&self.tape, pos, end)?,
                Kind::R => crate::primitives::execute_r(&self.tape, pos, end, &mut self.rules)?,
                Kind::G => crate::primitives::execute_g()?,
                Kind::C => crate::primitives::execute_c(&self.rules)?,
            };
            return Ok(Some((pos, end, replacement)));
        }

        Ok(None)
    }

    fn leftmost_kind<T>(
        a: Option<(usize, usize, T)>,
        b: (usize, usize, T),
    ) -> Option<(usize, usize, T)> {
        match a {
            Some((pa, .., _)) if pa <= b.0 => a,
            _ => Some(b),
        }
    }

    fn scan_block_primitive(&self, tape: &str, prefix: &str) -> Option<(usize, usize)> {
        let mut search_from = 0;
        while let Some(pos) = tape[search_from..].find(prefix) {
            let abs_pos = search_from + pos;
            if !Self::is_active_escape_start(tape, abs_pos) {
                search_from = abs_pos + 1;
                continue;
            }
            let after_brace = abs_pos + prefix.len();
            if let Some(end) = crate::primitives::find_unescaped_brace(&tape[after_brace..]) {
                let abs_end = after_brace + end + 1;
                return Some((abs_pos, abs_end));
            }
            search_from = abs_pos + 1;
        }
        None
    }

    fn scan_simple_primitive(tape: &str, pattern: &str) -> Option<(usize, usize)> {
        let mut search_from = 0;
        while let Some(pos) = tape[search_from..].find(pattern) {
            let abs_pos = search_from + pos;
            if !Self::is_active_escape_start(tape, abs_pos) {
                search_from = abs_pos + 1;
                continue;
            }
            return Some((abs_pos, abs_pos + pattern.len()));
        }
        None
    }

    fn is_active_escape_start(tape: &str, pos: usize) -> bool {
        let bytes = tape.as_bytes();
        let mut count = 0;
        let mut idx = pos;
        while idx > 0 && bytes[idx - 1] == b'\\' {
            count += 1;
            idx -= 1;
        }
        count % 2 == 0
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
    fn vm_normal_halt_does_not_consume_fuel() {
        let tape = Tape::new("A");
        let rules = vec![Rule::new(RuleKey::new("B"), "C")];
        let mut vm = Vm::new(tape, rules, 0, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "A");
    }

    #[test]
    fn vm_one_mutation_can_halt_with_one_fuel() {
        let tape = Tape::new("A");
        let rules = vec![Rule::new(RuleKey::new("A").with_once(), "B")];
        let mut vm = Vm::new(tape, rules, 1, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "B");
    }

    #[test]
    fn vm_tape_overflow() {
        let tape = Tape::new("A");
        let rules = vec![Rule::new(RuleKey::new("A"), "BBBB")];
        let mut vm = Vm::new(tape, rules, 100, 3);
        assert!(matches!(vm.run(), Err(Error::TapeOverflow { .. })));
    }

    #[test]
    fn vm_halt_replacement_checks_tape_limit() {
        let tape = Tape::new("A");
        let rules = vec![Rule::new(RuleKey::new("A"), "BBBB").with_rhs_prefix(RhsPrefix::Halt)];
        let mut vm = Vm::new(tape, rules, 100, 3);
        assert!(matches!(vm.run(), Err(Error::TapeOverflow { .. })));
    }

    #[test]
    fn vm_end_modifier_does_not_split_escape() {
        // The C inside the lazy primitive \C must not be matched by C(end)=Z.
        let tape = Tape::new(r"\C");
        let rules = vec![Rule::new(RuleKey::new("C").with_end(), "Z")];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), r"\C");
    }

    #[test]
    fn vm_lazy_primitive_matched_as_whole_token() {
        // A rule whose LHS is the whole lazy primitive \C should match.
        let tape = Tape::new(r"\C");
        let rules = vec![Rule::new(RuleKey::new(r"\C"), "REPLACED")];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "REPLACED");
    }

    #[test]
    fn vm_reflection_then_plain_match() {
        // \r{A=B} adds the rule A=B; the trailing A is a separate token and matches.
        let tape = Tape::new(r"\r{A=B}A");
        let rules = vec![];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "B");
    }

    #[test]
    fn vm_rule_does_not_match_inside_capsule() {
        // A <...> capsule (e.g. \c output or injected input) is atomic for rule
        // matching, so the GO inside <GO=...> must not match the GO rule.
        let tape = Tape::new("<GO=hello>");
        let rules = vec![Rule::new(RuleKey::new("GO"), "REPLACED")];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "<GO=hello>");
    }

    #[test]
    fn vm_rule_miss_inside_utf8_capsule_does_not_panic() {
        let tape = Tape::new("<é>");
        let rules = vec![
            Rule::new(RuleKey::new("<é>").with_once(), "<é>"),
            Rule::new(RuleKey::new("é"), "REPLACED"),
        ];
        let mut vm = Vm::new(tape, rules, 100, 1024 * 1024);
        vm.run().unwrap();
        assert_eq!(vm.tape.as_str(), "<é>");
    }

    #[test]
    fn vm_scans_primitive_after_escaped_backslash_pair() {
        let vm = Vm::new(Tape::new(r"\\\p{hi}"), vec![], 100, 1024 * 1024);
        assert_eq!(vm.scan_block_primitive(vm.tape.as_str(), r"\p{"), Some((2, 8)));
    }

    #[test]
    fn vm_does_not_scan_primitive_after_single_escape_backslash() {
        let vm = Vm::new(Tape::new(r"\\p{hi}"), vec![], 100, 1024 * 1024);
        assert_eq!(vm.scan_block_primitive(vm.tape.as_str(), r"\p{"), None);
    }

    #[test]
    fn vm_scans_simple_primitive_after_escaped_backslash_pair() {
        assert_eq!(Vm::scan_simple_primitive(r"\\\c", r"\c"), Some((2, 4)));
        assert_eq!(Vm::scan_simple_primitive(r"\\c", r"\c"), None);
    }
}
