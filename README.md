# Bambuddy Monitor

A small Linux tray app for [Bambuddy](https://github.com/maziggy/bambuddy). When a print starts, a window pops up with the **live camera** and **print status** (progress, layer, time left, temperatures, printer warnings), with a **Stop print** button. It hides itself after the print ends.

Unofficial companion; not affiliated with Bambuddy or Bambu Lab.

- Lightweight: Rust + [Tauri 2](https://tauri.app). The window (and its webview) exists only while shown; idle cost is a small poller.
- Talks only to your Bambuddy server (REST API key), never to the printer directly. Several viewers can watch at once because Bambuddy fans the camera out.
- Desktop notification when a print finishes or fails.

## Install

Grab a package from [Releases](../../releases) (`.deb`, `.rpm`, AppImage), or on Arch build the PKGBUILD:

    git clone https://github.com/shatteredeagle/bambuddy-monitor && cd bambuddy-monitor && makepkg -si

Runtime needs: `webkit2gtk-4.1`, `libayatana-appindicator`, `gtk3`. Your desktop must show tray icons (KDE does; GNOME needs the AppIndicator extension).

## Set up

First run opens Settings: your Bambuddy URL and an API key (Bambuddy > Settings > API Keys), and optionally one printer to watch. Config lives in `~/.config/bambuddy-monitor/config.json`. Right-click the tray icon for Show / Settings / Quit. To start at login, copy the `.desktop` file to `~/.config/autostart/`.

## Build

Needs rust, `webkit2gtk-4.1` and `libayatana-appindicator`.

    cd src-tauri && cargo build --release        # binary at src-tauri/target/release/bambuddy-monitor
    cargo install tauri-cli --locked && cargo tauri build   # .deb / .rpm / AppImage

`tools/make-icons.py` re-renders the icons from the SVG (PySide6).

## Licence

AGPL-3.0-or-later, see [LICENSE](LICENSE).
