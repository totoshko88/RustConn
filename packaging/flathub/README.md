# Publishing RustConn to Flathub

## Prerequisites

1. GitHub account
2. Flathub account (sign in at https://flathub.org with GitHub)

## Steps to Submit

### 1. Fork the Flathub repository

Go to https://github.com/flathub/flathub and fork it.

### 2. Create a new repository for your app

Create a new repository named `org.rustconn.RustConn` in your fork.

### 3. Add the manifest files

Copy these files to your new repository:
- `org.rustconn.RustConn.yml` - Main manifest
- `cargo-sources.json` - Cargo dependencies

### 4. Create a Pull Request

Create a PR from your `org.rustconn.RustConn` repository to `flathub/org.rustconn.RustConn`.

### 5. Wait for Review

Flathub maintainers will review your submission. Common requirements:
- Valid AppStream metadata (`org.rustconn.RustConn.metainfo.xml`)
- Proper desktop file
- Icons in correct sizes
- No network access during build (offline build)

## Updating the Package

When releasing a new version:

1. Update `tag` and `commit` in `org.rustconn.RustConn.yml`
2. Regenerate `cargo-sources.json`:
   ```bash
   flatpak-cargo-generator Cargo.lock -o cargo-sources.json
   ```
3. Create a PR to your Flathub repository

## Testing Locally

```bash
# Install Flatpak and flathub repo
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo

# Install SDK
flatpak install flathub org.gnome.Sdk//47 org.gnome.Platform//47
flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable//24.08
flatpak install flathub org.freedesktop.Sdk.Extension.llvm18//24.08

# Build
flatpak-builder --force-clean build-dir org.rustconn.RustConn.yml

# Test run
flatpak-builder --run build-dir org.rustconn.RustConn.yml rustconn

# Create bundle for testing
flatpak-builder --repo=repo --force-clean build-dir org.rustconn.RustConn.yml
flatpak build-bundle repo RustConn.flatpak org.rustconn.RustConn
```

## Links

- Flathub submission guide: https://docs.flathub.org/docs/for-app-authors/submission
- App requirements: https://docs.flathub.org/docs/for-app-authors/requirements
- Flatpak Rust guide: https://docs.flatpak.org/en/latest/rust.html
