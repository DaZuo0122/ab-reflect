# A=B^Reflect Interpreter — Implementation Plan

## 1. Design Decisions

| Decision | Choice |
|---|---|
| `\p{...}` | Prints payload + newline to stdout |
| `\g` | Reads stdin until EOF |
| Tape | UTF-8 `String`; `\g` input lossily converted |
| CLI input | File path arg, or stdin if omitted |
| Errors | Custom `Error` enum, `Result` propagation, non-zero exit codes |
| Crate structure | Workspace: `ab-reflect` lib + `abx` bin |
| Pragmas | `@fuel`, `@tape-limit` as source-level defaults |
| Trace format | `[step:N fuel:F] "TAPE_CONTENT"` |
| Testing | Unit tests + `.abx` integration tests |

## 2. Workspace Layout

```
Cargo.toml                 # workspace root
├── crates/
│   ├── ab-reflect/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs          # public API: parse, run
│   │       ├── error.rs        # Error, Result types
│   │       ├── parser.rs       # source → Vec<Rule>
│   │       ├── vm.rs           # execution engine
│   │       ├── tape.rs         # Tape type + safe injection
│   │       ├── primitives.rs   # \p, \g, \r, \c handlers
│   │       └── rule.rs         # Rule, RuleKey, Modifier types
│   └── abx/
│       ├── Cargo.toml
│       └── src/
│           └── main.rs         # CLI, arg parsing, I/O
└── tests/
    └── programs/               # .abx integration test files
```

## 3. Crate: `ab-reflect` (Library)

### 3.1 Core Types (`rule.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuleKey {
    pub lhs: String,
    pub has_start: bool,
    pub has_end: bool,
    pub has_once: bool,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub key: RuleKey,
    pub rhs: String,
    pub rhs_prefix: RhsPrefix,
    pub used: bool,            // for (once) rules
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RhsPrefix {
    Normal,
    Start,
    End,
    Halt,
}
```

### 3.2 Tape (`tape.rs`)

```rust
pub struct Tape {
    content: String,
}
```

- **Safe Injection Protocol**: static methods to escape CLI args and `\g` input.
- **String matching**: standard `String::find` / `String::replace` (UTF-8 safe).

### 3.3 Parser (`parser.rs`)

State machine, NOT `split('=')`:

1. **Line scanner**: read line by line, preserving all characters.
2. **Classifier**: shebang, comment (`#`), pragma (`# @key=value`), blank, or rule.
3. **Rule splitter**: scan for first **unescaped** `=`.
   - Track `\` escaping state.
   - Preserve all whitespace on both sides.
4. **Modifier extractor**: parse `(start)`, `(once)`, `(end)` from LHS; `(start)`, `(end)`, `(halt)` from RHS prefix.
5. **Escape decoder**: apply Complete Escape Table (Section 2.4 of spec).
6. **Block scanner**: for `\p{...}` and `\r{...}`, read until first unescaped `}`.
7. **Pragma parser**: `@fuel=N`, `@tape-limit=N`.

**Error type**: `ParseError` with line number and description.

### 3.4 VM (`vm.rs`)

```rust
pub struct Vm {
    tape: Tape,
    rules: Vec<Rule>,
    fuel: u64,
    tape_limit: usize,     // in bytes
    step_count: u64,
}
```

**Main loop** (exact priority from spec):

```
loop:
  1. Phase 1: scan tape left→right for leftmost active primitive (\p, \g, \r, \c).
     - If found: execute side effect, mutate tape, decrement fuel, restart loop.
  2. Phase 2: scan rules top→bottom for first match.
     - If found: delete matched substring, insert RHS (respecting prefix),
       mark (once) as used, decrement fuel.
     - If rhs_prefix is Halt: output tape and return Ok(Halted).
     - Restart loop.
  3. Phase 3: no match found. Output tape and return Ok(Halted).
```

**Fuel check**: decrement BEFORE or WITH each mutation. If fuel reaches 0, return `Error::FuelExceeded`.

**Tape limit check**: after every mutation, if `tape.len() > tape_limit`, return `Error::TapeOverflow`.

### 3.5 Primitives (`primitives.rs`)

| Primitive | Action |
|---|---|
| `\p{payload}` | Print decoded payload + `\n` to stdout. Replace primitive with empty string in tape. |
| `\g` | Read stdin to EOF, apply Safe Injection, replace `\g` with `<ESCAPED_INPUT>` or `<EOF>`. |
| `\r{rule_string}` | Parse `rule_string` as a single rule (with `(del)` support). Upsert or delete from `rules`. Replace primitive with empty string. |
| `\c` | Serialize current rules (with runtime escaping: active→lazy, `\`→`\\`, `<`→`\<`, `>`→`\>`). Wrap in `< >`. Replace primitive with result. |

### 3.6 Error (`error.rs`)

```rust
#[derive(Debug)]
pub enum Error {
    Parse { line: usize, message: String },
    FuelExceeded { steps: u64 },
    TapeOverflow { len: usize, limit: usize },
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
```

## 4. Crate: `abx` (Binary)

### 4.1 CLI (clap)

```
abx [OPTIONS] [FILE] [ARGS...]

Arguments:
  [FILE]   Source file (omit or use - for stdin)
  [ARGS]   Arguments appended to initial tape

Options:
  --fuel <N>         Max execution steps (overrides pragma)
  --tape-limit <MB>  Max tape size in MB (overrides pragma)
  --no-final         Suppress final tape output
  --trace            Print every mutation to stderr
  -h, --help         Print help
```

### 4.2 Main Flow

1. Parse CLI args with clap.
2. Read source: file or stdin.
3. Parse source with `ab_reflect::parser::parse()`.
4. Extract pragmas (`@fuel`, `@tape-limit`). CLI flags override pragmas.
5. Build initial tape from first rule's LHS text (no modifiers).
6. Append CLI args via Safe Injection Protocol.
7. Create `Vm` with tape, rules, fuel, tape-limit.
8. Run VM loop. If `--trace`, print `[step:N fuel:F] "TAPE"` to stderr after each step.
9. On `Ok(Halted)`: unless `--no-final`, print final tape to stdout.
10. On `Err(e)`: print error to stderr, exit with non-zero code.

## 5. Testing Strategy

### 5.1 Unit Tests

- **Parser**: escape decoding, unescaped `=` splitting, modifier extraction, block scanning, error cases.
- **Tape**: safe injection round-trip.
- **VM**: fuel exhaustion, tape overflow, rule matching with `(start)`/`(end)`/`(once)`.
- **Primitives**: `\r` upsert/delete, `\c` serialization with downgrade.

### 5.2 Integration Tests

Place `.abx` files in `tests/programs/` with expected output files.

Example:

```
tests/programs/
  ├── hello.abx
  ├── hello.out
  ├── cat.abx
  ├── cat.in
  ├── cat.out
  └── ...
```

A test runner (using `std::process::Command`) runs `abx` against each `.abx` and asserts stdout/stderr/exit code.

## 6. Key Invariants

1. **No implicit trimming**: parser preserves every whitespace character.
2. **Deterministic rule order**: `Vec<Rule>` maintains parse order; `\r` appends new rules to end.
3. **RuleKey identity**: `(lhs_text, has_start, has_end, has_once)` uniquely identifies a rule.
4. **Primitive priority**: Phase 1 (active primitives) always runs before Phase 2 (Markov rules).
5. **Safe serialization**: `\c` output can always be safely fed into `\r` without side effects.
6. **Fuel monotonicity**: fuel strictly decreases on every mutation; 0 is fatal.

## 7. Parser Design (Hand-Rolled)

### 7.1 Why Not a Parsing Crate?

We evaluated `nom`, `pest`, `logos`, and `regex`. All are rejected:

| Approach | Rejection Reason |
|---|---|
| `nom` | Combinators excel at recursive grammars; our parser is a **line-oriented scanner** with escape-state-dependent splitting. `nom` would fight the "first unescaped `=`" and zero-trimming semantics. |
| `pest` / `logos` | Generator tools assume clean token/grammar boundaries. Escaped delimiters (`\=`, `\}`) and inline modifier strings like `(once)` are awkward in PEG/regex-based grammars. |
| `regex` | Cannot track escape state; impossible to express "match `=` only if not preceded by odd number of `\`". |

The spec explicitly warns: *"Do not use simple `split('=')`. Implement a state machine."* The parsing is **scanning, not grammar reduction**.

### 7.2 Parser Architecture

The parser is a **two-pass scanner** in `parser.rs`:

**Pass 1 — Line Classifier**
```rust
fn classify_line(line: &str) -> LineKind
```
- Strips trailing `\n` only (line terminator, not data).
- Checks first non-whitespace char:
  - `#!` → Shebang (skip)
  - `#` → Comment (skip), or Pragma if `# @`
  - whitespace-only → Blank (skip)
  - anything else → Raw Rule line

**Pass 2 — Rule Parser**
```rust
fn parse_rule(line: &str, line_no: usize) -> Result<Rule>
```
1. **Modifier extraction** (front of line): greedily match `(start)`, `(once)`, `(end)` from a known set. Duplicate or unknown modifier → `ParseError`.
2. **Unescaped `=` splitter**: scan char-by-char with a boolean `escape` flag.
   - On `\` → toggle escape flag, emit to buffer.
   - On `=` with `escape == false` → this is the delimiter. LHS buffer is complete.
   - Otherwise → emit to LHS or RHS buffer.
3. **RHS prefix check**: after `=`, check for `(start)`, `(end)`, `(halt)` at the very beginning. Only one allowed, mutually exclusive.
4. **Escape decoder**: walk LHS and RHS buffers, replacing escape sequences per the Complete Escape Table. Unknown escape → `ParseError`.
5. **Block validation** (lazy): we do NOT fully validate `\p{...}` or `\r{...}` payloads at parse time; we just ensure braces are balanced at the text layer. Payload decoding happens at **runtime** when the primitive executes (matching the spec's two-stage decoding).

**Pass 3 — Pragma Parser**
```rust
fn parse_pragma(line: &str) -> Result<(String, String)>
```
- Split at first `=` after `# @`.
- Validate keys: only `fuel`, `tape-limit` are recognized.

### 7.3 State Machine for Unescaped `=`

```
state Scanning:
  read char c
  if c == '\\':
    flip escape_next
    append '\\' to buffer
  else if c == '=' and not escape_next:
    return lhs_buffer, remaining_input
  else:
    clear escape_next
    append c to buffer
```

This is ~20 lines of imperative Rust. Adding `nom` or `pest` would be 10× the dependency weight for worse control.

### 7.4 Error Richness

All parse errors carry:
- `line: usize` — 1-based source line number
- `message: String` — human-readable description
- `column: Option<usize>` — byte offset within the line (nice-to-have, not critical for MVP)

Example: `ParseError { line: 7, message: "unknown escape sequence: \\x", column: Some(12) }`

## 8. Implementation Order

1. **Scaffolding**: workspace `Cargo.toml`, crate skeletons.
2. **Types + Error**: `Rule`, `RuleKey`, `Error`.
3. **Tape + Safe Injection**: `Tape` type, escape/unescape utilities.
4. **Parser**: line classification, rule splitting, escape decoding, pragma parsing.
5. **VM core**: main loop, rule matching, fuel/tape checks.
6. **Primitives**: `\p`, `\g`, `\r`, `\c`.
7. **CLI**: clap setup, main flow, I/O.
8. **Tests**: unit tests, then integration `.abx` programs.

---

## Git Commit Workflow

After each atomic step above, the implementation will pause. A commit message will be provided in text only. Wait for review before proceeding to the next step.
