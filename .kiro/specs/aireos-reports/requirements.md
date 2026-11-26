# Requirements: AireOS WLC Collectors

## Introduction
Six AireOS collectors from securecrt-tools, restated as CLI report kinds over
WLC `show … summary` tables.

## Requirements

### Requirement 1 — Six kinds
WHEN `report aireos-*` is invoked THEN the CLI SHALL run the corresponding
read-only `show` command and emit rows with the documented headers.

### Requirement 2 — Unknown output is ignored
WHEN output does not contain recognised table rows THEN the report SHALL emit
no rows rather than guessing fields.

### Requirement 3 — Unchanged behaviour
WHEN an existing report kind runs THEN its output SHALL CONTINUE unchanged.

## Acceptance Criteria
- [ ] Six kinds registered; generic table parser tested with sample output;
      every parser row matches its header width.
- [ ] Documentation states they are unverified against a real WLC.
