//! Core types: Rule, RuleKey, and modifiers.

/// Uniquely identifies a rule by its normalized identity.
/// Modifiers are part of the key — `(start)A=B` and `A=B` are different rules.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuleKey {
    /// Decoded LHS text (without modifiers).
    pub lhs: String,
    /// `(start)` prefix present.
    pub has_start: bool,
    /// `(end)` suffix present.
    pub has_end: bool,
    /// `(once)` prefix present.
    pub has_once: bool,
}

impl RuleKey {
    pub fn new(lhs: impl Into<String>) -> Self {
        Self {
            lhs: lhs.into(),
            has_start: false,
            has_end: false,
            has_once: false,
        }
    }

    pub fn with_start(mut self) -> Self {
        self.has_start = true;
        self
    }

    pub fn with_end(mut self) -> Self {
        self.has_end = true;
        self
    }

    pub fn with_once(mut self) -> Self {
        self.has_once = true;
        self
    }
}

/// A single rewrite rule.
#[derive(Clone, Debug)]
pub struct Rule {
    pub key: RuleKey,
    /// Decoded RHS text (without prefix).
    pub rhs: String,
    /// RHS prefix modifier.
    pub rhs_prefix: RhsPrefix,
    /// Tracks whether this `(once)` rule has already fired.
    pub used: bool,
}

impl Rule {
    pub fn new(key: RuleKey, rhs: impl Into<String>) -> Self {
        Self {
            key,
            rhs: rhs.into(),
            rhs_prefix: RhsPrefix::Normal,
            used: false,
        }
    }

    pub fn with_rhs_prefix(mut self, prefix: RhsPrefix) -> Self {
        self.rhs_prefix = prefix;
        self
    }
}

/// Prefix that controls where the RHS is inserted on match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RhsPrefix {
    /// Insert RHS where the matched LHS was (default).
    Normal,
    /// Insert RHS at the start of the tape.
    Start,
    /// Insert RHS at the end of the tape.
    End,
    /// Insert RHS and then halt execution.
    Halt,
}
