# Publishing RustConn to Flathub

## Automated Updates

After the initial submission is accepted, updates are automated via GitHub Actions:

1. Create a new release/tag (e.g., `v0.5.0`)
2. The `flathub-update.yml` workflow automatically:
   - Generates new `cargo-sources.json`
   - Updates the Flathub repo manifest
   - Pushes changes to trigger a new build

### Required Setup

1. Create a GitHub Personal Access Token with `repo` scope
2. Add it as `FLATHUB_TOKEN` secret in repository settings

## Manual Submission (First Time)

### Prerequisites

1. GitHub account
2. Flathub account (sign in at https://flathub.org with GitHub)

### Steps to Submit

1. Fork https://github.com/flathub/flathub
2. Create branch `io.github.totoshko88.RustConn`
3. Add these files:
   - `io.github.totoshko88.RustConn.yml` - Main manifest
   - `flathub.json` - Linter exceptions
   - `cargo-sources.json` - Cargo dependencies (from workflow artifact)
4. Create PR to `flathub/flathub`

### Generate cargo-sources.json locally

```bash
pip install aiohttp toml
wget https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
python3 flatpak-cargo-generator.py Cargo.lock -o cargo-sources.json
```

## Testing Locally

```bash
# Install Flatpak and flathub repo
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo

# Install SDK
flatpak install flathub org.gnome.Sdk//49 org.gnome.Platform//49
flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable//24.08

# Build
flatpak-builder --force-clean build-dir io.github.totoshko88.RustConn.yml

# Test run
flatpak-builder --run build-dir io.github.totoshko88.RustConn.yml rustconn

# Create bundle for testing
flatpak-builder --repo=repo --force-clean build-dir io.github.totoshko88.RustConn.yml
flatpak build-bundle repo RustConn.flatpak io.github.totoshko88.RustConn
```

## Links

- Flathub submission guide: https://docs.flathub.org/docs/for-app-authors/submission
- App requirements: https://docs.flathub.org/docs/for-app-authors/requirements
- Flatpak Rust guide: https://docs.flatpak.org/en/latest/rust.html
