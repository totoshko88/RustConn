# Orphaned Subgroups on Delete — Bugfix Design

## Overview

When deleting a group that contains only empty subgroups (0 total connections), the GUI calls `delete_group()` which reparents children instead of removing them. The CLI `cmd_group_delete()` manually removes only the target group via `groups.retain()`, leaving child groups with dangling `parent_id` references. The fix is minimal: use `delete_group_cascade()` in the GUI's empty-group branch, and rewrite the CLI to delegate to `ConnectionManager` instead of reimplementing deletion logic.

## Glossary

- **Bug_Condition (C)**: A group deletion where the target group has descendant subgroups — the GUI path uses the wrong delete method, and the CLI bypasses `ConnectionManager` entirely
- **Property (P)**: After deletion, no group references the deleted group's ID as `parent_id`, and all empty descendant subgroups are removed
- **Preservation**: Groups with connections still show the "Keep Connections" / "Delete All" dialog; single connection deletion is unaffected; groups with no subgroups delete cleanly
- **`delete_group()`**: Method on `ConnectionManager` in `rustconn-core/src/connection/manager.rs` that removes a group, reparents child groups to the deleted group's parent, and ungroups direct connections
- **`delete_group_cascade()`**: Method on `ConnectionManager` that removes a group and all its descendants (groups + connections) recursively
- **`count_connections_in_group()`**: Method on `ConnectionManager` that counts connections across the entire descendant hierarchy of a group
- **`collect_descendant_groups()`**: Internal method that BFS-collects all descendant group IDs for a given group

## Bug Details

### Bug Condition

The bug manifests in two paths:

1. **GUI** (`operations.rs`): When `is_group && connection_count == 0`, the code enters the "simple confirmation" branch and calls `state_mut.delete_group(id)`. This is correct for leaf groups but wrong for groups with empty subgroups. Since `count_connections_in_group()` already counts the full hierarchy, `connection_count == 0` guarantees cascade is safe (no data loss). The fix is changing `delete_group(id)` → `delete_group_cascade(id)`.

2. **CLI** (`group.rs`): `cmd_group_delete()` manually calls `groups.retain(|g| g.id != id)` and only clears `group_id` on direct connections. It never updates child groups' `parent_id` references, and it doesn't use `ConnectionManager` at all. The fix is constructing a `ConnectionManager` and delegating to `delete_group()` (which properly reparents children and ungroups connections).

**Formal Specification:**
```
FUNCTION isBugCondition(input)
  INPUT: input of type GroupDeleteRequest { group_id: Uuid, path: GUI | CLI }
  OUTPUT: boolean

  LET group = findGroup(input.group_id)
  LET descendants = collectDescendantGroups(input.group_id)
  LET has_subgroups = |descendants| > 1  // includes self

  IF input.path == GUI THEN
    LET conn_count = countConnectionsInGroup(input.group_id)
    RETURN has_subgroups AND conn_count == 0
  ELSE IF input.path == CLI THEN
    RETURN has_subgroups
  END IF
END FUNCTION
```

### Examples

- **GUI: Parent with one empty child** — User deletes group "Servers" which contains subgroup "Dev" (no connections anywhere). `connection_count == 0`, so code calls `delete_group()`. "Dev" gets reparented to root instead of being deleted. After fix: both "Servers" and "Dev" are removed.
- **GUI: Three-level nesting** — "Region" → "DC1" → "Rack1", all empty. Deleting "Region" reparents "DC1" and "Rack1" to root. After fix: all three are cascade-deleted.
- **CLI: Parent with child containing connections** — `rustconn group delete "Servers"` removes "Servers" but leaves child group "Prod" with `parent_id` pointing to the now-deleted "Servers" UUID. Connections in "Prod" are untouched. After fix: "Prod" is reparented to root (or cascade-deleted), connections handled properly.
- **Edge case: Group with no subgroups** — Both GUI and CLI work correctly today; no change needed.

## Expected Behavior

### Preservation Requirements

**Unchanged Behaviors:**
- Groups with `connection_count > 0` deleted via GUI still show the "Keep Connections" / "Delete All" dialog
- Choosing "Keep Connections" still calls `delete_group()` (reparent children, ungroup connections)
- Choosing "Delete All" still calls `delete_group_cascade()`
- Deleting a single connection (not a group) is unaffected
- Deleting a group with no subgroups and no connections works as before
- CLI deletion of a group with no subgroups still removes only the target group and ungroups its direct connections

**Scope:**
All inputs that do NOT involve deleting a group with descendant subgroups should be completely unaffected by this fix. This includes:
- Single connection deletions
- Group deletions where the group has no children
- Groups with connections (GUI dialog path is unchanged)

## Hypothesized Root Cause

Based on the bug description and code analysis:

1. **GUI: Wrong method call for empty groups with subgroups** — In `operations.rs` line ~148, the `else` branch (simple confirmation for `connection_count == 0`) calls `state_mut.delete_group(id)`. This is correct for leaf groups but wrong for groups with empty subgroups. Since `count_connections_in_group()` already counts the full hierarchy, `connection_count == 0` guarantees cascade is safe (no data loss). The fix is changing `delete_group(id)` → `delete_group_cascade(id)`.

2. **CLI: Manual deletion bypasses ConnectionManager** — `cmd_group_delete()` reimplements deletion with `groups.retain()` + manual `group_id` clearing. It never updates child groups' `parent_id` references, and it doesn't use `ConnectionManager` at all. The fix is constructing a `ConnectionManager` and delegating to `delete_group()` (which properly reparents children and ungroups connections).

## Correctness Properties

Property 1: Bug Condition — Cascade delete removes all empty descendants (GUI)

_For any_ group hierarchy where the root group has descendant subgroups and zero total connections, calling `delete_group_cascade()` SHALL remove the root group and all its descendant subgroups, leaving no group with a `parent_id` referencing any deleted group ID.

**Validates: Requirements 2.1, 2.2**

Property 2: Bug Condition — CLI delete properly handles child groups

_For any_ group with descendant subgroups deleted via the CLI path (using `ConnectionManager::delete_group()`), the operation SHALL reparent all direct child groups to the deleted group's parent and ungroup all direct connections, leaving no dangling `parent_id` references.

**Validates: Requirements 2.3, 2.4**

Property 3: Preservation — Non-subgroup deletions unchanged

_For any_ group deletion where the target group has no descendant subgroups, the fixed code SHALL produce the same result as the original code: the group is removed, direct connections are ungrouped, and no other groups are affected.

**Validates: Requirements 3.4, 3.5, 3.6**

Property 4: Preservation — Groups with connections still trigger dialog

_For any_ group with `connection_count > 0`, the GUI deletion path SHALL continue to present the "Keep Connections" / "Delete All" dialog and call the corresponding `delete_group()` or `delete_group_cascade()` method based on user choice.

**Validates: Requirements 3.1, 3.2, 3.3**

## Fix Implementation

### Changes Required

Assuming our root cause analysis is correct:

**File**: `rustconn/src/window/operations.rs`

**Function**: `delete_selected_connection()`

**Specific Changes**:
1. **Change `delete_group` to `delete_group_cascade` in the empty-group branch** — In the `else` block (simple confirmation, `is_group && connection_count == 0`), change `state_mut.delete_group(id)` to `state_mut.delete_group_cascade(id)`. This is safe because `connection_count == 0` means the entire subtree has no connections, so cascade only removes empty groups.

**File**: `rustconn-cli/src/commands/group.rs`

**Function**: `cmd_group_delete()`

**Specific Changes**:
2. **Replace manual deletion with `ConnectionManager` delegation** — Remove the manual `groups.retain()` + connection clearing logic. Instead, construct a `ConnectionManager` from the `ConfigManager`, call `delete_group(id)` on it (which properly reparents child groups and ungroups connections), and let the manager handle persistence.
3. **Add `ConnectionManager` import** — Add `use rustconn_core::ConnectionManager;` to the imports.
4. **Handle async runtime** — `ConnectionManager::new()` requires a Tokio runtime (it spawns persistence tasks). The CLI may need a `tokio::runtime::Runtime::new()` block or use an existing runtime context.

## Testing Strategy

### Validation Approach

The testing strategy follows a two-phase approach: first, surface counterexamples that demonstrate the bug on unfixed code, then verify the fix works correctly and preserves existing behavior.

### Exploratory Bug Condition Checking

**Goal**: Surface counterexamples that demonstrate the bug BEFORE implementing the fix. Confirm or refute the root cause analysis.

**Test Plan**: Write property tests that create group hierarchies with empty subgroups, call `delete_group()` (the wrong method), and assert that orphaned groups remain — confirming the bug exists.

**Test Cases**:
1. **Empty subgroup orphan test**: Create parent → child (both empty), call `delete_group(parent)`, assert child still exists with reparented `parent_id` (demonstrates the bug)
2. **Deep nesting orphan test**: Create 3-level hierarchy (all empty), call `delete_group(root)`, assert descendants are reparented (demonstrates the bug)
3. **CLI-style manual delete test**: Simulate `groups.retain()` without updating child `parent_id`, assert dangling references exist

**Expected Counterexamples**:
- After `delete_group()` on a parent with empty children, the children still exist (reparented) instead of being deleted
- After manual `groups.retain()`, child groups have `parent_id` pointing to a non-existent group

### Fix Checking

**Goal**: Verify that for all inputs where the bug condition holds, the fixed function produces the expected behavior.

**Pseudocode:**
```
FOR ALL input WHERE isBugCondition(input) DO
  result := delete_group_cascade(input.group_id)  // GUI fix
  ASSERT no group has parent_id referencing any deleted group
  ASSERT all descendant groups are removed
END FOR
```

### Preservation Checking

**Goal**: Verify that for all inputs where the bug condition does NOT hold, the fixed function produces the same result as the original function.

**Pseudocode:**
```
FOR ALL input WHERE NOT isBugCondition(input) DO
  ASSERT delete_group(input.group_id) behaves identically before and after fix
  ASSERT connections are ungrouped, no other groups affected
END FOR
```

**Testing Approach**: Property-based testing with `proptest` is recommended because:
- It generates many random group hierarchies automatically
- It catches edge cases in nesting depth and group topology
- It provides strong guarantees that non-buggy paths are unchanged

**Test Plan**: Use `ConnectionManager` in tests to create random group hierarchies, then verify cascade delete removes all descendants and non-cascade delete properly reparents.

**Test Cases**:
1. **Cascade removes all descendants**: Generate random tree of empty groups, cascade-delete root, verify all are gone
2. **No dangling parent_id after cascade**: Generate random tree, cascade-delete root, verify no remaining group references a deleted ID
3. **Non-cascade reparents correctly**: Generate random tree with connections, non-cascade delete, verify children reparented and connections ungrouped
4. **Leaf group deletion unchanged**: Generate group with no children, delete, verify only that group removed

### Unit Tests

- Test `delete_group_cascade()` on a parent with one empty child
- Test `delete_group_cascade()` on a 3-level empty hierarchy
- Test `delete_group()` still reparents children when connections exist
- Test CLI path with `ConnectionManager::delete_group()` properly reparents

### Property-Based Tests

- Generate random group trees (varying depth/breadth), cascade-delete root, assert no orphans and no dangling references
- Generate random group trees with connections, non-cascade delete, assert children reparented and connections ungrouped
- Generate random leaf groups, delete, assert identical behavior to original

### Integration Tests

- Test full CLI `group delete` command with nested groups
- Test that GUI empty-group deletion path removes all descendants
- Test undo/trash behavior after cascade delete of empty hierarchy
