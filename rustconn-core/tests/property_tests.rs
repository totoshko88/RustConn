//! Property-based tests for `RustConn` core library
//!
//! This module contains property-based tests that validate correctness properties
//! defined in the design document.

// Allow common test patterns that Clippy warns about
#![allow(
    clippy::redundant_clone,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::similar_names,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::unreadable_literal,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::items_after_statements,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::too_many_lines,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::cognitive_complexity,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::field_reassign_with_default,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::single_component_path_imports,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::use_self,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::needless_pass_by_value,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::redundant_closure,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::redundant_closure_for_method_calls,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::expect_fun_call,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::unwrap_or_default,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::map_unwrap_or,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::single_char_pattern,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::cast_sign_loss,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::default_trait_access,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::useless_format,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::match_same_arms,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::uninlined_format_args,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::significant_drop_tightening,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::collection_is_never_read,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::used_underscore_binding,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::stable_sort_primitive,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::implicit_clone,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::or_fun_call,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::len_zero,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::literal_string_with_formatting_args,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::collapsible_str_replace,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::needless_collect,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::manual_let_else,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::branches_sharing_code,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::format_push_string,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::clone_on_copy,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::to_string_trait_impl,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::cast_lossless,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::unnecessary_to_owned,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::option_if_let_else,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::manual_string_new,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::if_not_else,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::inefficient_to_string,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::suboptimal_flops,
    reason = "module-wide override for legacy code; refactored case by case"
)]
// New lints from IronRDP integration — suppress for existing test helpers
#![allow(
    unreachable_pub,
    reason = "test helpers use pub for cross-module visibility"
)]
#![allow(clippy::unwrap_used, reason = "tests use unwrap/expect for brevity")]
#![allow(
    clippy::wildcard_imports,
    reason = "test modules use prelude-style imports"
)]

mod fixtures;
mod properties;
