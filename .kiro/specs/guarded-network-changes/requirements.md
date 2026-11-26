# Requirements: Guarded Network-Change Scripts

## Introduction
Port the config-changing family from securecrt-tools as `.rsc` scripts with
check mode as the default. No live changes on the supplied production switch.

## Requirements

### Requirement 1 — Check mode first
WHEN a change script is run without the literal `APPLY` THEN it SHALL print
the proposed commands and send no configuration command.

### Requirement 2 — Validation and confirmation
WHEN a user supplies an interface, description, address or command THEN it
SHALL be validated before use; privileged mode SHALL be required.

### Requirement 3 — Audit and save
WHEN a change is applied THEN an owner-only timestamped transcript SHALL be
written, and saving to startup SHALL require a separate `YES`.

### Requirement 4 — Unchanged behaviour
WHEN existing read-only examples run THEN they SHALL CONTINUE to work.

## Acceptance Criteria
- [ ] Global config, DHCP relay and interface description examples exist.
- [ ] Test proves check mode sends no `configure terminal`.
