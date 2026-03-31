# Bugfix Requirements Document

## Introduction

When a user deletes a group that contains subgroups but no connections (or only empty subgroups), the subgroups are orphaned instead of being deleted. This happens because the GUI deletion path in `operations.rs` calls `delete_group()` (which reparents children) instead of `delete_group_cascade()` when `connection_count == 0`. The CLI `cmd_group_delete()` has the same issue — it only removes the target group via `groups.retain()`, leaving child groups with dangling `parent_id` references. The user expects all empty subgroups to be removed along with the parent group since there is no data to preserve.

## Bug Analysis

### Current Behavior (Defect)

1.1 WHEN a group with empty subgroups and 0 total connections is deleted via the GUI THEN the system calls `delete_group()` which reparents the subgroups to the deleted group's parent instead of removing them

1.2 WHEN a group with nested empty subgroups (multiple levels deep) and 0 total connections is deleted via the GUI THEN the system reparents all descendant subgroups to the deleted group's parent, leaving them visible in the sidebar

1.3 WHEN a group with subgroups is deleted via the CLI (`cmd_group_delete`) THEN the system only removes the target group with `groups.retain()`, leaving child groups with dangling `parent_id` references pointing to the now-deleted group

1.4 WHEN a group with subgroups containing connections is deleted via the CLI THEN the system only ungroups direct connections of the target group but does not handle connections nested in child subgroups

### Expected Behavior (Correct)

2.1 WHEN a group with empty subgroups and 0 total connections is deleted via the GUI THEN the system SHALL call `delete_group_cascade()` to remove the group and all its descendant subgroups

2.2 WHEN a group with nested empty subgroups (multiple levels deep) and 0 total connections is deleted via the GUI THEN the system SHALL recursively delete all descendant subgroups at every nesting level

2.3 WHEN a group with subgroups is deleted via the CLI (`cmd_group_delete`) THEN the system SHALL either cascade-delete all descendant subgroups and their connections, or properly reparent all descendant subgroups to the deleted group's parent (consistent with the chosen strategy)

2.4 WHEN a group with subgroups containing connections is deleted via the CLI THEN the system SHALL handle all connections in descendant subgroups (either moving them to ungrouped or deleting them, consistent with the chosen strategy)

### Unchanged Behavior (Regression Prevention)

3.1 WHEN a group with connections (connection_count > 0) is deleted via the GUI THEN the system SHALL CONTINUE TO show the dialog offering "Keep Connections" / "Delete All" options

3.2 WHEN a group with connections is deleted via the GUI and the user chooses "Keep Connections" THEN the system SHALL CONTINUE TO call `delete_group()` to reparent children and move connections to ungrouped

3.3 WHEN a group with connections is deleted via the GUI and the user chooses "Delete All" THEN the system SHALL CONTINUE TO call `delete_group_cascade()` to remove the group, all descendants, and all connections

3.4 WHEN a single connection (not a group) is deleted via the GUI THEN the system SHALL CONTINUE TO delete only that connection without affecting groups

3.5 WHEN a group with no subgroups and no connections is deleted via the GUI THEN the system SHALL CONTINUE TO delete the group cleanly

3.6 WHEN a group is deleted via the CLI and the group has no subgroups THEN the system SHALL CONTINUE TO delete only the target group and ungroup its direct connections