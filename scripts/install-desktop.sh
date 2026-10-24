#!/bin/sh
# Ярлык и значок mMail для KDE (панель задач, меню). На Wayland KWin ищет значок окна
# по app_id ("mmail") через одноимённый .desktop-файл — без него рисуется значок Wayland.
# Ставится только для текущего пользователя, в ~/.local; повторный запуск обновляет.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# PNG из SVG: размытие в исходнике (feGaussianBlur) Qt SVG не рисует, поэтому SVG не ставим.
(cd "$root" && npx tauri icon src-tauri/icons/source.svg -o "$tmp" -p 16,22,24,32,48,64,128,256 >/dev/null)
for size in 16 22 24 32 48 64 128 256; do
  install -Dm644 "$tmp/${size}x${size}.png" "$data/icons/hicolor/${size}x${size}/apps/mmail.png"
done

# Пока нет релизной сборки, ярлык запускает dev-режим (через scripts/mmail → launch.sh).
# MimeType: mMail в списке программ для ссылок mailto: (выбрать по умолчанию — в «Параметрах системы»).
install -d "$data/applications"
cat > "$data/applications/mmail.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=mMail
GenericName=Почтовый клиент
Comment=Почта Яндекса и Gmail
Icon=mmail
Exec="$root/scripts/mmail" %u
Terminal=false
Categories=Network;Email;
StartupWMClass=mmail
MimeType=x-scheme-handler/mailto;
DESKTOP

touch "$data/icons/hicolor"
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" || true
command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 >/dev/null 2>&1 || true
echo "mMail: ярлык и значок установлены в $data"
