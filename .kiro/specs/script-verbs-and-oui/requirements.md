# Requirements: RenameTab Verb and Offline OUI Lookup

## Introduction
Provide the two SecureCRT behaviours that cannot be expressed with the
original DSL: renaming the tab from the CLI prompt and looking up a MAC
vendor offline.

## Requirements

### Requirement 1 — Caption verb
WHEN a script executes `caption <var>` THEN the running session's tab SHALL be
renamed to the variable's trimmed value, capped at 80 characters, and the
saved connection name SHALL NOT change.

### Requirement 2 — OUI verb
WHEN a script executes `oui <mac-var> <vendor-var>` THEN the vendor SHALL be
looked up offline from the built-in prefix table; unknown prefixes SHALL store
"Vendor not found".

### Requirement 3 — Unchanged behaviour
WHEN existing scripts use `getprompt` or `log` THEN they SHALL CONTINUE to
behave identically.

## Acceptance Criteria
- [ ] Bundled `example-cisco-rename-tab` and `example-oui-lookup` parse and
      have passing engine tests.
- [ ] In-app reference and USER_GUIDE list both verbs.
