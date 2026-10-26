# Bambuddy Monitor (Tauri)

Linux tray app for [Bambuddy](https://github.com/maziggy/bambuddy). When a print starts, a window pops up with the live camera and print status (progress, layer, time left, temps, warnings) plus a Stop print button. It hides after the print ends. The window (and its webview) only exists while it is shown, so idle cost is the small Rust poller.

Rust + Tauri 2. Config is `~/.config/bambuddy-monitor/config.json`, the same file as the Python prototype (`../bambuddy-monitor`). First run opens Settings: Bambuddy URL + API key (Bambuddy > Settings > API Keys).

Build (needs `webkit2gtk-4.1`, `libayatana-appindicator`, rust): `cd src-tauri && cargo build --release` → `src-tauri/target/release/bambuddy-monitor`.
Icons: `python3 tools/make-icons.py` (PySide6) re-renders them from the SVG.
