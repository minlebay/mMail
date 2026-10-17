#!/usr/bin/env bash
# VS Code из snap протаскивает в дочерние процессы свои GTK/GIO-пути,
# и WebKitGTK падает на чужом libpthread. Запускаем команду с чистым окружением.
set -euo pipefail

[[ -n "${XDG_DATA_DIRS_VSCODE_SNAP_ORIG:-}" ]] && export XDG_DATA_DIRS="$XDG_DATA_DIRS_VSCODE_SNAP_ORIG"
[[ -n "${XDG_CONFIG_DIRS_VSCODE_SNAP_ORIG:-}" ]] && export XDG_CONFIG_DIRS="$XDG_CONFIG_DIRS_VSCODE_SNAP_ORIG"

unset GDK_PIXBUF_MODULEDIR GDK_PIXBUF_MODULE_FILE GIO_MODULE_DIR GSETTINGS_SCHEMA_DIR \
  GTK_EXE_PREFIX GTK_IM_MODULE_FILE GTK_PATH LOCPATH
while IFS='=' read -r name _; do
  [[ "$name" == SNAP* ]] && unset "$name"
done < <(env)

exec "$@"
