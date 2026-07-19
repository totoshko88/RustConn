---
inclusion: auto
---
# Shell Environment

## Terminal Profile

The workspace uses `bash --noprofile --norc` with explicit PATH injection via the VS Code terminal profile. This means:

- No `.bashrc`, `.profile`, or `/etc/profile` is sourced in agent terminals
- `cargo`, `rustfmt`, `clippy` are available at `~/.cargo/bin/` (injected via profile `env.PATH`)
- `~/.local/bin/` is also in PATH (for `uv`, `pipx`, user scripts)
- `direnv` is NOT active in agent shells (requires hook in `.bashrc`)

## Multiline Text in Shell Commands

**Never pass multiline text inline** in bash command arguments (e.g. `--body '...'` with newlines). The bash tool cannot reliably handle unmatched quotes and heredocs across multiple lines.

Instead:
1. Write multiline content to a temp file using `fs_write`
2. Pass the file to the command (e.g. `gh issue comment --body-file /tmp/comment.md`)
3. Delete the temp file after use

## Available Tools

| Tool | Path | Notes |
|------|------|-------|
| cargo | ~/.cargo/bin/cargo | Rust toolchain via rustup |
| gh | gh (system) | GitHub CLI, authenticated |
| flatpak-builder | flatpak-builder (system) | Flatpak builds |
| kirograph | ~/.nvm/.../bin/kirograph | Code graph (when .kirograph/ exists) |

## Cargo Commands

Always use the full path if PATH issues arise: `/home/totoshko88/.cargo/bin/cargo`

Common verification sequence:
```bash
cargo fmt --check
cargo clippy --all-targets
cargo test --package rustconn-core --test property_tests
```
