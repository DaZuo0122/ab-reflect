# A=B^Reflect Language Specification

## 1. Overview
**A=B^Reflect** is a deterministic, Turing-complete esoteric programming language based on the Markov Algorithm. It extends classic string rewriting with controlled I/O, lazy evaluation, and dynamic self-modification (reflection), while strictly adhering to a "no implicit magic" philosophy.

## 2. Lexical Structure & Syntax

### 2.1 Strict No-Trim Parsing
The parser splits each line at the **first unescaped `=`** character. 
* **Zero Trimming**: Leading and trailing whitespace in both LHS and RHS are **preserved as literal characters**. Space is data, not formatting.
* **No Inline Comments**: A line is a comment only if it starts with `#` (after ignoring leading whitespace). Any `#` within a rule line is part of the RHS data.

### 2.2 Formal Grammar (EBNF)
```ebnf
program      ::= line*
line         ::= shebang | comment | pragma | rule | blank

shebang      ::= "#!" .* "\n"
comment      ::= "#" .* "\n"
pragma       ::= "# @" key "=" value "\n"
blank        ::= whitespace* "\n"

// Strict rule structure
rule         ::= lhs_prefix* lhs lhs_suffix* "=" rhs_prefix? rhs "\n"

lhs_prefix   ::= "(start)" | "(once)"
lhs_suffix   ::= "(end)"
rhs_prefix   ::= "(start)" | "(end)" | "(halt)"

lhs          ::= char*  // Until the first unescaped '='
rhs          ::= char*  // After the '=' until '\n'

char         ::= escape | any_other_char
```

### 2.3 Modifier Combination Rules
* **LHS Prefixes**: `(start)` and `(once)` can be combined in any order (e.g., `(start)(once)A=B` is valid and equivalent to `(once)(start)A=B`). Duplicate prefixes (e.g., `(once)(once)`) are a **Parse Error**.
* **LHS Suffix**: `(end)` can appear at most once. Combined with `(start)`, it means the Tape must exactly equal the LHS.
* **RHS Prefixes**: `(start)`, `(end)`, and `(halt)` are **mutually exclusive**. Only one can appear, and it must immediately follow the `=`.

### 2.4 Complete Escape Table
Unknown escape sequences result in a **Parse Error**.

| Escape | Meaning in Source/Rule Text | Escape | Meaning in Source/Rule Text |
| :--- | :--- | :--- | :--- |
| `\\` | Literal backslash `\` | `\=` | Literal equals sign `=` |
| `\n` | Line feed (U+000A) | `\t` | Horizontal tab (U+0009) |
| `\{` | Literal `{` | `\}` | Literal `}` |
| `\<` | Literal `<` | `\>` | Literal `>` |
| `\(` | Literal `(` | `\)` | Literal `)` |
| `\p`, `\P` | Literal `\p`, `\P` (Preserved) | `\g`, `\G` | Literal `\g`, `\G` (Preserved) |
| `\r`, `\R` | Literal `\r`, `\R` (Preserved) | `\c`, `\C` | Literal `\c`, `\C` (Preserved) |

---

## 3. Execution Model

The interpreter maintains two states: **Tape** (a mutable string) and **Rule Set** (an ordered list of parsed rules).

### 3.1 Initialization
1. Parse the source code, skipping shebang, comments, and blanks.
2. Extract the **LHS text (without modifiers)** of the **first valid rule** as the initial `Tape`.
3. CLI arguments (if any) are appended to the initial Tape using the Safe Injection Protocol (see Section 5).

### 3.2 The Main Loop
The engine runs an infinite loop with strict priority:

**Phase 1: Primitive Interception (Highest Priority)**
Scan the Tape from left to right for the leftmost occurrence of an **active primitive** (`\p{...}`, `\g`, `\r{...}`, `\c`).
* If found: Execute the primitive's side effect, mutate the Tape, decrement Fuel, and **immediately restart the loop from Phase 1**.

**Phase 2: Markov Rule Application**
If no primitives are found, scan the Rule Set from top to bottom.
* Find the **first** rule whose LHS matches a substring in the Tape (respecting `(start)` and `(end)` anchors).
* If found: 
  1. Delete the matched LHS substring.
  2. Insert the RHS according to `rhs_prefix` (Normal, InsertStart, InsertEnd, or Halt).
  3. If the rule has `(once)`, mark it as `used`.
  4. Decrement Fuel.
  5. If `rhs_prefix` is `(halt)`, output the final Tape and **Halt**.
  6. **Immediately restart the loop from Phase 1**.

**Phase 3: Normal Halt**
If neither Phase 1 nor Phase 2 finds a match, output the final Tape and **Halt**.

---

## 4. Primitives & Reflection

### 4.1 Active vs. Lazy Primitives
To resolve the "code-as-data" paradox, primitives exist in two states:
* **Active (Lowercase)**: `\p{...}`, `\g`, `\r{...}`, `\c`. Intercepted and executed immediately in Phase 1.
* **Lazy (Uppercase)**: `\P{...}`, `\G`, `\R{...}`, `\C`. Treated as **pure literal strings** in the Tape. They are ignored by Phase 1.
* **Downgrade Execution**: To execute lazy code, a rule must explicitly rewrite it to its active form (e.g., `\P=\p`).

### 4.2 Block Primitive Scanning & Payload Decoding
For `\p{...}` and `\r{...}`:
1. **Scanning**: The parser reads from `{` until the **first unescaped `}`**. Nested braces are not supported.
2. **Decoding**: The captured payload undergoes **Escape Decoding** (e.g., `\}` becomes `}`, `\n` becomes a real newline) before being processed.
* *Example*: `\r{A=\p{hi\}}` captures payload `A=\p{hi\}`, decodes it to `A=\p{hi}`, and passes it to the reflection parser.

### 4.3 Reflection (`\r{RULE_STRING}`)
Modifies the Rule Set at runtime. The `RULE_STRING` is parsed as a single rule.
* **RuleKey Identity**: A rule is uniquely identified by `(lhs_text, has_start, has_end, has_once)`. Modifiers are part of the identity.
* **Upsert**: If RuleKey exists, overwrite its RHS and reset its `used` flag. If not, append to the end of the Rule Set.
* **Delete**: If the payload starts with `(del)`, it is a meta-command to **delete** the rule matching the subsequent RuleKey. `(del)` is **not** a valid top-level rule modifier; it is a Parse Error outside `\r{...}`.
* *Example*: `\r{(del)A=}` deletes the rule with LHS "A". `\r{A=}` adds/modifies a rule that replaces "A" with an empty string.

### 4.4 Introspection (`\c`)
Serializes the current Rule Set into a multi-line string, wrapped in `< >`.
* **Safety Protocol**: To prevent side effects when serialized code enters the Tape, `\c` applies **Runtime Escaping**:
  1. All active primitives are downgraded to lazy primitives (`\p` → `\P`, `\g` → `\G`, `\r` → `\R`, `\c` → `\C`).
  2. Literal `\`, `<`, and `>` are escaped as `\\`, `\<`, and `\>`.
* *Example*: A rule `A=\p{hi}` is serialized as `<A=\P{hi}>`.

---

## 5. CLI & Host Environment

The language core is pure; resource management is delegated to the CLI (`abx`).

### 5.1 Safe Injection Protocol
CLI arguments and `\g` input are injected into the Tape using a strict escaping protocol to prevent accidental primitive execution or boundary breaking:
* `\` becomes `\\`
* `<` becomes `\<`
* `>` becomes `\>`
* CLI Args Format: Initial Tape + ` <arg:ESCAPED_ARG1> <arg:ESCAPED_ARG2>...`
* `\g` Input Format: Replaces `\g` with `<ESCAPED_INPUT>` (or `<EOF>`).

### 5.2 Resource Guards
* `--fuel <N>`: Maximum execution steps. Exceeding triggers a `FuelExceeded` panic.
* `--tape-limit <MB>`: Maximum Tape length. Exceeding triggers a `TapeOverflow` panic.
* `--no-final`: Suppresses the final Tape output on Halt (useful for pure side-effect programs).
* `--trace`: Prints every Tape mutation to `stderr` for debugging.

---

## 6. Implementation Notes (Rust Focus)

The complexity of A=B^Reflect lies entirely in the **text-layer semantics (Parser/Serializer)**, not the VM loop.

1. **Rule Parsing**: Do not use simple `split('=')`. Implement a state machine to find the first unescaped `=` and strictly preserve all surrounding whitespace.
2. **RuleKey Hashing**: Use the exact decoded `lhs` string and boolean flags for `has_start`, `has_end`, and `has_once` to ensure deterministic upserts during `\r`.
3. **Block Scanner**: The payload extractor for `\p` and `\r` must correctly handle `\\` (escaped backslash) so that `\}` is recognized as a literal brace, not a terminator.
4. **Serialization**: The `\c` output must rigorously apply the downgrade and escape rules to guarantee that feeding `\c` output back into `\r` is a safe, side-effect-free operation.
