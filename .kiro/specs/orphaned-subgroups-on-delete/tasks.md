# Tasks: Orphaned Subgroups on Delete

## Task 1: GUI Fix — Use cascade delete for empty groups (rustconn)

- [x] 1.1 In `rustconn/src/window/operations.rs`, in the `delete_selected_connection()` function's simple confirmation branch (where `is_group && connection_count == 0`), change `state_mut.delete_group(id)` to `state_mut.delete_group_cascade(id)`

## Task 2: CLI Fix — Delegate to ConnectionManager (rustconn-cli)

- [x] 2.1 In `rustconn-cli/src/commands/group.rs`, add `use rustconn_core::ConnectionManager;` to imports
- [x] 2.2 Rewrite `cmd_group_delete()` to construct a `ConnectionManager` from the `ConfigManager` and call `delete_group(id)` instead of manual `groups.retain()` + connection clearing logic
- [x] 2.3 Handle the Tokio runtime requirement for `ConnectionManager::new()` (wrap in `tokio::runtime::Runtime::new()` block if CLI is not already async)

## Task 3: Property-Based Tests (rustconn-core)

- [x] 3.1 Create `rustconn-core/tests/properties/group_delete_tests.rs` test file with proptest imports and `ConnectionManager` test helper
- [x] 3.2 Register the new module in `rustconn-core/tests/properties/mod.rs`
- [x] 3.3 [PBT: Property 1] Write property test: cascade delete on random group trees with empty subgroups removes all descendants and leaves no dangling `parent_id` references
- [x] 3.4 [PBT: Property 2] Write property test: `delete_group()` on a group with child subgroups properly reparents all direct children to the deleted group's parent and ungroups direct connections
- [x] 3.5 [PBT: Property 3] Write property test: deleting a leaf group (no subgroups) produces identical results whether using `delete_group()` or `delete_group_cascade()` — group removed, connections ungrouped, no other groups affected
