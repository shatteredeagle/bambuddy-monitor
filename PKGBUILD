# Maintainer: jesse
# Build from a checkout: `makepkg -si` in this directory.
pkgname=bambuddy-monitor
pkgver=0.2.0
pkgrel=1
pkgdesc='Tray app that pops up a live camera and status window when a Bambuddy print starts'
arch=(x86_64 aarch64)
license=(custom)
depends=(webkit2gtk-4.1 libayatana-appindicator gtk3)
makedepends=(rust)
options=(!lto)

prepare() {
  rm -rf "$srcdir/build" && mkdir "$srcdir/build"
  tar -C "$startdir" --exclude=target -cf - dist src-tauri bambuddy-monitor.svg | tar -C "$srcdir/build" -xf -
}
build() { cd "$srcdir/build/src-tauri" && cargo build --release --locked; }
package() {
  cd "$srcdir/build"
  install -Dm755 src-tauri/target/release/bambuddy-monitor "$pkgdir/usr/bin/bambuddy-monitor"
  install -Dm644 "$startdir/packaging/bambuddy-monitor.desktop" "$pkgdir/usr/share/applications/bambuddy-monitor.desktop"
  install -Dm644 bambuddy-monitor.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/bambuddy-monitor.svg"
}
