# Running RustConn on Windows (WSL2)

RustConn is a Linux GTK4 application and has no native Windows port (GTK4 +
VTE on Windows is not in a usable state). The supported way to run RustConn
on a Windows machine is **WSL2 with WSLg**, which renders Linux GUI apps as
regular Windows windows — including Start menu integration, Alt-Tab, and
shared clipboard.

This guide follows Microsoft's
[Run Linux GUI apps with WSL](https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps)
tutorial and adds the RustConn-specific setup that is easy to miss
(systemd, D-Bus, and secret storage).

## Prerequisites

- **Windows 10 Build 19044+ or Windows 11.** WSLg (the GUI layer) ships with
  WSL 0.47.1+ on these builds.
- **WSL 2 only.** Linux GUI apps do not work in WSL 1. Convert an existing
  distro with `wsl --set-version <distro> 2` if needed.
- **GPU driver with vGPU support** for hardware-accelerated OpenGL
  (recommended, not strictly required):
  [Intel](https://www.intel.com/content/www/us/en/download-center/home.html),
  [AMD](https://www.amd.com/en/support/download/drivers.html),
  [NVIDIA](https://www.nvidia.com/drivers).

## Step 1 — Install or update WSL

Fresh install (admin PowerShell, then reboot):

```powershell
wsl --install
```

After the reboot, Ubuntu finishes installing and asks for a Linux username
and password.

Existing WSL install — make sure WSLg is present:

```powershell
wsl --update
wsl --shutdown   # restart WSL so the update takes effect
```

Verify with `wsl --version` — the output should list a `WSLg` version.

## Step 2 — Enable systemd (required)

RustConn relies on a D-Bus session bus for the Secret Service (saved
passwords via GNOME Keyring), desktop portals (the SFTP file-manager
integration), and notifications. The easiest way to get a working session
bus in WSL is to enable systemd.

Inside the distro, create or edit `/etc/wsl.conf`:

```ini
[boot]
systemd=true
```

Then restart WSL from PowerShell:

```powershell
wsl --shutdown
```

This step is the most common reason RustConn "doesn't work" under WSL —
without it the app starts, but saving passwords and opening SFTP in a file
manager fail.

## Step 3 — Install RustConn

Inside Ubuntu (24.04 LTS shown; for other releases see
[INSTALL.md](INSTALL.md)):

```bash
echo 'deb http://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_24.04/ /' \
  | sudo tee /etc/apt/sources.list.d/rustconn.list
curl -fsSL https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_24.04/Release.key \
  | gpg --dearmor | sudo tee /etc/apt/trusted.gpg.d/rustconn.gpg > /dev/null
sudo apt update
sudo apt install rustconn
```

Or install the `.deb` from
[GitHub Releases](https://github.com/totoshko88/RustConn/releases):

```bash
sudo apt install ./rustconn_*_amd64.deb
```

Both paths pull in the GTK4/libadwaita/VTE runtime automatically. Optional
external clients (`freerdp3-x11`, `tigervnc-viewer`, `virt-viewer`,
`picocom`) are Recommends — install the ones you need.

## Step 4 — Secret storage

For the default GNOME Keyring backend:

```bash
sudo apt install gnome-keyring
```

On first password save, the keyring prompts you to set (or confirm) a keyring
password. If you prefer not to run a keyring daemon under WSL, pick a CLI
backend instead in **Settings → Secrets** (KeePassXC, Bitwarden, 1Password,
pass) — see [USER_GUIDE.md](USER_GUIDE.md).

## Step 5 — Launch

From the Ubuntu terminal:

```bash
rustconn
```

or from the Windows **Start menu → Ubuntu → RustConn**. The window behaves
like a native Windows app (Alt-Tab, taskbar pinning, clipboard sharing).

## Known limitations under WSLg

- **No system tray.** WSLg has no StatusNotifier host, so the tray icon and
  minimize-to-tray are unavailable; the window close button quits the app.
- **Emoji icons may not render** — WSLg ships a limited set of fonts and may
  lack full color emoji coverage. Smart Folder and connection icons set to
  emoji may appear as blank boxes. Use GTK icon names (e.g.
  `network-server-symbolic`) as an alternative, or install a Noto Color Emoji
  font: `sudo apt install fonts-noto-color-emoji && fc-cache -f`.
- **Embedded RDP may fail with "decode error"** — the built-in IronRDP
  client does not yet support all RDP extensions. If the embedded client
  fails, RustConn automatically falls back to FreeRDP. Install it:
  `sudo apt install freerdp3-x11` (or `freerdp2-x11` on older distros).
  Alternatively, use an external RDP session (right-click → Connect External).
- **Embedded RDP/VNC rendering is slower than on native Linux** — WSLg adds
  a compositing hop (RDP-over-vsock). For heavy remote-desktop work prefer
  the Windows native clients, or accept the overhead.
- **libEGL / MESA warnings in the log** — WSLg uses software rendering via
  `d3d12` or `llvmpipe` when no vGPU driver is installed. These warnings are
  cosmetic and do not affect functionality; install a
  [vGPU driver](https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps#install-support-for-linux-gui-apps)
  to silence them and get hardware acceleration.
- **Audio** (RDP/SPICE audio redirection) goes through WSLg's PulseAudio
  server; it works but adds latency.
- **Serial ports** require attaching USB devices to WSL via
  [usbipd-win](https://learn.microsoft.com/en-us/windows/wsl/connect-usb).
- **Wake-on-LAN** packets originate from the WSL NAT network; in the default
  networking mode they may not reach your LAN. Use `networkingMode=mirrored`
  (Windows 11 22H2+) in `.wslconfig` if you need WoL.

## Troubleshooting

- **"cannot open display" / blank window** — see the official WSLg guide:
  [Diagnosing "cannot open display" issues](https://github.com/microsoft/wslg/wiki/Diagnosing-%22cannot-open-display%22-type-issues-with-WSLg).
  Check that `echo $DISPLAY` prints `:0` and `echo $WAYLAND_DISPLAY` prints
  `wayland-0` inside the distro.
- **GUI apps don't start at all** — confirm WSL 2 (`wsl -l -v`) and that
  `wsl --version` shows a WSLg version; update Windows if the build is older
  than 19044.
- **Passwords are not saved / Secret Service errors** — systemd is not
  enabled (Step 2) or `gnome-keyring` is missing (Step 4). Verify with
  `systemctl is-system-running` and `busctl --user list | grep secrets`.
- **SFTP "Open in file manager" does nothing** — install a file manager
  (`sudo apt install nautilus`) and the GVFS SFTP backend
  (`sudo apt install gvfs-backends`); requires the session bus from Step 2.
- **Fonts look wrong / tofu glyphs** — `sudo apt install fonts-dejavu
  fonts-noto-color-emoji` inside the distro.
