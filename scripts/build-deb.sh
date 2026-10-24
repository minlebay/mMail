#!/usr/bin/env bash
# Сборка .deb: dist-deb/mMail_<версия>_amd64.deb.
# Упаковщику Tauri нужен pkg-config-файл appindicator (пакет libayatana-appindicator3-dev);
# если его нет — подсовываем временный, указывающий на уже установленную библиотеку.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if ! pkg-config --exists ayatana-appindicator3-0.1 2>/dev/null; then
  mkdir -p "$tmp/pc"
  version=$(dpkg-query -W -f='${Version}' libayatana-appindicator3-1 | cut -d- -f1)
  cat > "$tmp/pc/ayatana-appindicator3-0.1.pc" <<PC
libdir=/usr/lib/x86_64-linux-gnu
Name: ayatana-appindicator3
Description: Ayatana AppIndicator
Version: $version
Libs: -L\${libdir} -layatana-appindicator3
PC
  export PKG_CONFIG_PATH="$tmp/pc${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi

[[ -f $HOME/.cargo/env ]] && . "$HOME/.cargo/env"
# Старые пакеты в каталоге сборки иначе тоже скопировались бы в dist-deb/.
rm -f src-tauri/target/release/bundle/deb/*.deb
scripts/clean-env.sh npx tauri build --bundles deb

# Tauri называет .desktop-файл по productName (mMail.desktop), а Wayland app_id берётся из
# имени бинарника (mmail). KWin ищет значок окна по app_id в одноимённом .desktop-файле,
# без него в Alt+Tab рисуется стандартный значок Wayland. Переименовываем файл в пакете
# и пересчитываем md5sums (там пути к файлам).
deb=$(find src-tauri/target/release/bundle/deb -maxdepth 1 -name '*.deb' -print -quit)
dpkg-deb -R "$deb" "$tmp/deb"
mv "$tmp/deb/usr/share/applications/mMail.desktop" "$tmp/deb/usr/share/applications/mmail.desktop"
(cd "$tmp/deb" && find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums)
dpkg-deb --root-owner-group -b "$tmp/deb" "$tmp/mmail.deb" >/dev/null
mv "$tmp/mmail.deb" "$deb"

mkdir -p dist-deb
cp src-tauri/target/release/bundle/deb/*.deb dist-deb/
ls -la dist-deb/*.deb
