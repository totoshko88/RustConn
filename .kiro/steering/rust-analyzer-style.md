---
inclusion: fileMatch
fileMatchPattern: "**/*.rs"
---

# rust-analyzer Style — RustConn Adaptation

Adaptation of the [rust-analyzer style guide](https://rust-analyzer.github.io/book/contributing/style.html)
for RustConn. Supplements `rust-pragmatic-guidelines.md` and `project-rules.md`,
does not replace them. Only the rules that are portable *and* not already covered
by another steering file are kept; rust-analyzer's project-specific rules are
listed at the bottom under "Do NOT adopt" so nobody copies them by reflex.

The upstream guide's own framing applies here too: **style does not block a
change.** Flag a nit, or send a follow-up cleanup — do not hold correct work
hostage to formatting. These are defaults for new code, not a mandate to churn
existing code that already reads fine.

## Types on the boundary — prefer the general one

Prefer the borrowed, more general type in a parameter position:

```
// GOOD      BAD
&[T]         &Vec<T>
&str         &String
Option<&T>   &Option<T>
&Path        &PathBuf
```

Rationale: the left column is strictly more general and reveals less about
internal representation. This pairs with the Microsoft M-guidelines already in
`rust-pragmatic-guidelines.md`; it does not conflict with "push allocations to the
call site" — when the callee genuinely needs to *own* the value, take `String` /
`PathBuf` by value instead of taking `&str` and cloning inside.

## Preconditions live in types, checked where they are used

Force the caller to establish a precondition rather than re-checking it in the
callee, and never split the check from its use across two functions.

```rust
// GOOD — the caller cannot call this without a Walrus
fn frobnicate(walrus: Walrus) { ... }

// BAD — the "no walrus" branch is invisible at the call site
fn frobnicate(walrus: Option<Walrus>) {
    let Some(walrus) = walrus else { return };
    ...
}
```

```rust
// GOOD — check and use in the same block, precondition encoded on return
if let Some(contents) = string_literal_contents(s) { ... }

fn string_literal_contents(s: &str) -> Option<&str> {
    (s.starts_with('"') && s.ends_with('"')).then(|| &s[1..s.len() - 1])
}

// BAD — is_string_literal proves the bounds, main() relies on them
if is_string_literal(s) { let contents = &s[1..s.len() - 1]; }
```

Rationale: non-local properties degrade under change. Note this is the one place
the upstream guide's own `Option::filter`/`bool::then` advice bends — a `then`
that returns the sliced content in one expression is clearer than an `if` here;
elsewhere prefer control flow (see below).

## Config struct over a pile of bool / Option parameters

If a function grows several `bool` or `Option` parameters, pack them into a config
struct. If a `bool`/`Option` parameter is *always* passed a literal, split the
function in two instead.

```rust
// GOOD
pub fn export(conns: &[Connection], cfg: ExportConfig) -> Result<String, ExportError> { ... }

// GOOD — literal-only flag becomes two functions
fn connect(target: &Target) { ... }
fn connect_via_jump(target: &Target, jump: &JumpHost) { ... }

// BAD
pub fn export(conns: &[Connection], encrypt: bool, include_notes: bool, pretty: bool) -> ... { ... }
```

Do **not** implement `Default` for such a config struct and do not stash it in
long-lived state — the caller has the context to choose, and passing it
explicitly keeps that choice visible. (This is narrower than the M-guidelines'
general encouragement of `Default`; the difference is deliberate and upstream.)

## Invariants → private field + borrowing getter, no setters

- A field with no invariant: make it `pub`.
- A field with an invariant: document the invariant, enforce it in the
  constructor, keep the field private, expose a getter that returns **borrowed**
  data. Never write a setter.

```rust
impl Connection {
    fn name(&self) -> &str { &self.name }               // GOOD
    fn tags(&self) -> Option<&[String]> { self.tags.as_deref() } // GOOD
}
```

Rationale: privacy makes the invariant local; borrowed getters hide the storage
type. Aligns with Rust API Guidelines C-GETTER.

## File layout reads top-down as API documentation

Optimise for a reader seeing the file for the first time.

- If everything but one item is private, put the public item first.
- With a mix, public items first.
- Structs and enums before impls and free functions; declare types top-down
  (parent before child).

Rationale: with bodies folded, the file should read as the module's public API.

## Control flow, not clever combinators

- Use early returns; do not invert a function into one nested `if`.
- Prefer `match` over `if let ... { } else { }` — the `else` arm gets a precise
  pattern (`None`, `Err(_)`) instead of `_`.
- Use `map` / `and_then` / `?` when they are the natural choice; when a combinator
  chain creates friction, drop back to `for` / `if` / `match`. Mostly avoid
  `bool::then` and `Option::filter` (the string-literal case above is the
  sanctioned exception).
- `return Err(e)` to raise an error, not `Err(e)?` — `return` is `!`, so the
  compiler can still flag dead code after it.
- In comparisons use `<` / `<=`, avoid `>` / `>=`: `lo <= x && x <= hi` reads as
  the number line.

Rust 2024 note: prefer `let ... else { return }` and let-chains (the repo is on
the 2024 edition) over the older `match { Some(it) => it, None => return }` shape
where they read cleaner.

## Type ascription over turbofish, and no bare `_`

```rust
let mutable: Vec<Row> = old.into_iter().map(build).collect();   // GOOD
let mutable: Vec<_>   = old.into_iter().map(build).collect();   // BAD (bare _)
let mutable = old.into_iter().map(build).collect::<Vec<Row>>(); // BAD (turbofish)
```

Rationale: the result type up front helps the reader follow the iterator chain; if
the compiler needs the hint, so does the human.

## Blocks over single-use helper functions; helper variables freely

- Do not extract a helper function that is called once — a block gives the same
  delineation with full access to context, and single-use functions accrete
  parameters under change. Exception: when you want `return` or `?` inside.
- When you do keep a nested helper, put it at the **end** of the enclosing
  function (via `return`) and nest at most one level deep.
- Introduce named helper variables liberally, especially to name a multiline
  condition — they are cheap, aid debugging, and format better than a giant `if`.

## Comments are sentences

Inline comments start with a capital and end with a full stop. Writing a sentence
(sometimes a paragraph) captures the context you were holding in your head; a
lowercase fragment does not. For `.md` files use one sentence per line, do not wrap
— it keeps diffs readable. (The `changelog-format.md` prose already follows this.)

## Do NOT adopt (rust-analyzer-internal, wrong for RustConn)

These are correct for rust-analyzer and wrong here — do not copy them:

- **`FxHashMap` / `FxHashSet` everywhere.** RustConn is not hashing-hot; `std`
  collections stay the default (YAGNI, rung 3). Reach for `rustc-hash` only behind
  a measured hot path, and that is a new dependency decision (`cargo deny`).
- **`anyhow::Result` as the default `Result`.** In `rustconn-core` errors are
  `thiserror` enums so GUI/CLI can match variants (M-ERRORS-CANONICAL-STRUCTS);
  `anyhow` is allowed only in the `rustconn` / `rustconn-cli` binaries
  (M-APP-ERROR). Never blanket-alias `Result` to `anyhow::Result`.
- **Mangled keyword names** (`krate`, `strukt`, `func`, `ty`). Use plain
  descriptive names; RustConn has no keyword-collision problem to solve.
- **`stdx` crate / "add reusable bits to stdx".** No such crate here, and adding
  one contradicts the fewest-crates rule. Shared helpers live in `rustconn-core`.
- **Ban on `#[should_panic]` and `#[ignore]`.** RustConn's escape-hatch table in
  `project-rules.md` explicitly sanctions `#[ignore = "flaky: issue #NNN"]`;
  prefer asserting `Err`/`None` over `#[should_panic]`, but the outright ban does
  not apply.
- **`stdx::never!` / assert-liberally style.** Follow M-PANIC-ON-BUG instead:
  panic means "stop now", recoverable state is a `Result`. `debug_assert!` for
  programming bugs is fine; do not import a custom assertion vocabulary.
- **`T![foo]` token macros, `hir`/`ast` qualification.** Compiler-internal, no
  analogue in this codebase.

## References

- Upstream guide: <https://rust-analyzer.github.io/book/contributing/style.html>
- Complements: `rust-pragmatic-guidelines.md` (Microsoft M-guidelines),
  Rust API Guidelines <https://rust-lang.github.io/api-guidelines/>,
  checklist <https://rust-lang.github.io/api-guidelines/checklist.html>
