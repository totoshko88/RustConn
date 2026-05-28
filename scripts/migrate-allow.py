#!/usr/bin/env python3
"""Migrate #[allow(...)] to #[expect(..., reason = "...")].

Replaces well-known patterns with appropriate reasons. Anything
unrecognised is left alone for manual review.
"""

import re
import sys
from pathlib import Path

# Map: tuple of sorted lint names -> default reason for that combination.
REASONS = {
    ("clippy::too_many_lines",): "long match/dispatch over many enum variants; \
splitting per variant only relocates the boilerplate",
    ("clippy::too_many_arguments",): "function parameters mirror upstream API or \
struct fields 1:1; bundling into a struct only restates the field list",
    ("clippy::too_many_arguments", "clippy::too_many_lines"): "long match dispatch \
with many flat parameters; restructuring would only move the parameter list elsewhere",
    ("clippy::struct_excessive_bools",): "settings/flags struct mirrors persisted \
config 1:1; bools represent independent toggles, not a state machine",
    ("clippy::cast_possible_truncation",): "value range fits the target type \
by construction in this code path",
    ("clippy::cast_possible_wrap",): "value range fits the target signed type \
by construction in this code path",
    ("clippy::cast_sign_loss",): "value is non-negative by construction in this \
code path",
    ("clippy::cast_possible_truncation", "clippy::cast_sign_loss"): "value range \
fits the target type and is non-negative by construction in this code path",
    ("clippy::unused_self",): "method is part of a uniform helper API where most \
operations need &self; keeping &self preserves the consistent signature",
    ("clippy::unnecessary_wraps",): "function returns Result for trait/API \
uniformity even though this branch never fails",
    ("clippy::needless_pass_by_ref_mut",): "&mut required by the trait/upstream \
contract even when this branch does not mutate",
    ("clippy::needless_pass_by_value",): "value is consumed by trait/API contract; \
borrowing would force callers to clone before passing",
    ("clippy::future_not_send",): "future borrows GTK/Path types that are pinned \
to the calling thread; never moved to another runtime",
    ("clippy::implicit_hasher",): "wrapper takes any HashMap; an explicit S: \
BuildHasher generic would push one more parameter onto every caller",
    ("clippy::similar_names",): "names follow a deliberate naming scheme for \
paired inputs/outputs; renaming hurts readability",
    ("clippy::module_inception",): "internal `mod foo` inside `foo.rs` keeps the \
file private and re-exports curated items",
    ("clippy::trivially_copy_pass_by_ref",): "&self chosen for API consistency \
with sibling impls that need the borrow",
    ("clippy::option_if_let_else",): "if-let form reads more naturally than \
map_or_else for the side-effecting else branch",
    ("clippy::format_push_string",): "incremental format! into String is clearer \
than write! macro chaining for this report builder",
    ("clippy::case_sensitive_file_extension_comparisons",): "extension comes from \
a controlled allow-list, not user input - case is already normalised upstream",
    ("clippy::derive_partial_eq_without_eq",): "type may grow non-Eq fields (f32 \
colour components) in future; deriving Eq would break that path",
    ("clippy::missing_panics_doc", "clippy::significant_drop_tightening"): "Mutex \
guard is intentionally held across the operation; panic only on poisoned lock",
    ("clippy::significant_drop_tightening",): "guard is intentionally held across \
the operation to keep the critical section atomic",
    ("clippy::assertions_on_constants",): "compile-time invariant checked alongside \
runtime asserts to surface regressions on every test run",
    ("clippy::literal_string_with_formatting_args",): "test fixture string contains \
Rust format placeholders verbatim - they are part of the expected payload",
    ("clippy::match_same_arms",): "arms differ only in attached doc comment; \
collapsing would lose the inline documentation",
    ("clippy::unnecessary_literal_bound",): "explicit 'static bound documents that \
the trait object outlives all callers, even when the type system can infer it",
    ("clippy::type_complexity",): "internal helper signature documents the exact \
tuple layout used by the caller; aliasing would obscure the data flow",
    ("clippy::cast_precision_loss",): "f64 conversion is intentional for display/UI \
arithmetic where sub-integer precision is irrelevant",
    ("clippy::items_after_statements",): "local helper introduced inline next to \
its only call site; hoisting would scatter related logic",
    ("clippy::many_single_char_names",): "matrix/vector arithmetic uses canonical \
short names from the linear algebra literature",
    ("clippy::needless_collect",): "intermediate Vec is required because the \
following loop borrows the source mutably",
    ("clippy::unwrap_used",): "value is statically proven non-None at this site \
(see preceding insert/init); panicking would indicate a programmer bug",
    ("clippy::cast_possible_truncation", "clippy::cast_possible_wrap"): "value range \
fits both signed and unsigned target types by construction in this code path",
    ("clippy::cast_possible_wrap", "clippy::cast_possible_truncation"): "value range \
fits both signed and unsigned target types by construction in this code path",
    ("clippy::fn_params_excessive_bools", "clippy::too_many_arguments"): "function \
parameters mirror Clap-derived flags 1:1; bundling would only restate them",
    ("clippy::similar_names", "clippy::too_many_lines", "clippy::type_complexity"): \
"long match dispatch with paired naming; aliasing or splitting would only relocate \
the boilerplate",
    ("clippy::similar_names", "clippy::too_many_arguments"): "function parameters \
follow paired naming for inputs/outputs; bundling would only restate the field list",
    ("clippy::needless_pass_by_value", "clippy::too_many_lines"): "value is consumed \
by trait/API contract and the body dispatches over many variants; restructuring \
would scatter related logic",
    ("clippy::similar_names", "clippy::too_many_lines"): "long match dispatch with \
paired naming; splitting would only relocate the boilerplate",
    ("clippy::too_many_lines", "clippy::type_complexity"): "long match dispatch \
returning a documented tuple; aliasing would obscure the data flow",
    ("clippy::similar_names", "clippy::type_complexity"): "internal helper signature \
documents the tuple layout with paired naming; aliasing would obscure the data flow",
    ("clippy::similar_names", "clippy::too_many_lines", "clippy::type_complexity"): \
"long match dispatch returning a documented tuple with paired naming; restructuring \
would only relocate the boilerplate",
    ("clippy::similar_names", "clippy::too_many_arguments", "clippy::too_many_lines"): \
"long builder takes paired widgets and dispatches over many protocol kinds; \
restructuring would only relocate the parameter list",
    ("clippy::cast_lossless", "clippy::cast_possible_truncation", "clippy::cast_sign_loss"): \
"value range fits the target type and is non-negative by construction; explicit \
`as` cast is intentional alongside .round()",
}

# Pattern matches `#[allow(clippy::foo)]`,
# `#[allow(clippy::foo, clippy::bar)]`, or the multi-line form
# `#[allow(\n    clippy::foo,\n    clippy::bar,\n)]`.
ALLOW_RE = re.compile(r"#\[allow\(\s*([^)]+?)\s*\)\]", re.DOTALL)


def parse_lints(arg: str) -> tuple[str, ...]:
    return tuple(sorted(s.strip() for s in arg.split(",") if s.strip()))


def replace_one(match: re.Match[str]) -> str:
    lints = parse_lints(match.group(1))
    # dead_code allows are kept as-is — reasons usually live in inline comments
    # next to the field; expect() does not accept those positionally.
    if lints == ("dead_code",):
        return match.group(0)
    reason = REASONS.get(lints)
    if reason is None:
        return match.group(0)
    lints_str = ",\n    ".join(lints)
    return f"#[expect(\n    {lints_str},\n    reason = \"{reason}\"\n)]"


def migrate_file(path: Path) -> int:
    text = path.read_text()
    new_text, count = ALLOW_RE.subn(replace_one, text)
    if count and new_text != text:
        path.write_text(new_text)
    return count


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: migrate-allow.py FILE [FILE ...]", file=sys.stderr)
        return 1
    total = 0
    for arg in sys.argv[1:]:
        path = Path(arg)
        if not path.is_file():
            continue
        n = migrate_file(path)
        if n:
            print(f"  {path}: rewrote {n} attribute(s)")
        total += n
    print(f"Total rewritten: {total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
