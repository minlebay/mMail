#!/usr/bin/env -S zsh -i
# Запуск из меню KDE: интерактивный шелл подхватывает nvm (node) из ~/.zshrc, cargo — отсюда.
. "$HOME/.cargo/env"
cd "$(dirname "$0")/.." && exec npm run app
