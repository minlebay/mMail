#!/usr/bin/env bash
# Сборка .deb: dist-deb/mMail_<версия>_amd64.deb.
# Упаковщику Tauri нужен pkg-config-файл appindicator (пакет libayatana-appindicator3-dev);
# если его нет — подсовываем временный, указывающий на уже установленную библиотеку.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if ! pkg-config --exists ayatana-appindicator3-0.1 2>/dev/null; then
  pc=$(mktemp -d)
  trap 'rm -rf "$pc"' EXIT
  version=$(dpkg-query -W -f='${Version}' libayatana-appindicator3-1 | cut -d- -f1)
  cat > "$pc/ayatana-appindicator3-0.1.pc" <<PC
libdir=/usr/lib/x86_64-linux-gnu
Name: ayatana-appindicator3
Description: Ayatana AppIndicator
Version: $version
Libs: -L\${libdir} -layatana-appindicator3
PC
  export PKG_CONFIG_PATH="$pc${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi

[[ -f $HOME/.cargo/env ]] && . "$HOME/.cargo/env"
# Старые пакеты в каталоге сборки иначе тоже скопировались бы в dist-deb/.
rm -f src-tauri/target/release/bundle/deb/*.deb
scripts/clean-env.sh npx tauri build --bundles deb
mkdir -p dist-deb
cp src-tauri/target/release/bundle/deb/*.deb dist-deb/
ls -la dist-deb/*.deb
