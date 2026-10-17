// Контекстное меню — родное (GTK + размытие KWin), см. src-tauri/src/context_menu.rs.
// Страница по правому клику сообщает ядру пункты, ядро показывает меню и присылает выбор.
// preventDefault не вызываем: ядру нужно само событие мыши от WebKit.
import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { t } from './i18n.js';
import { toNative } from './menuItems.js';

let token = 0;
let runs = [];
let claimed = false;
let onError = console.error;

// Значение токена как готовый цвет: токены бывают составными (rgba(var(--ink), …)),
// а GTK понимает только конечный цвет — поэтому пропускаем через вычисленный стиль.
function cssVar(name, fallback) {
  const probe = document.createElement('span');
  probe.style.display = 'none';
  probe.style.color = `var(${name})`;
  document.body.append(probe);
  const color = getComputedStyle(probe).color;
  probe.remove();
  return color || fallback;
}

function send(items) {
  token += 1;
  const { native, runs: actions } = toNative(items, get(t));
  runs = actions;
  const colors = {
    surface: cssVar('--native-menu', 'rgba(36, 31, 48, 0.62)'),
    text: cssVar('--text', '#f1eef8'),
    accent: cssVar('--accent-soft', 'rgba(139, 123, 255, 0.28)'),
    danger: cssVar('--danger', '#ff6b8b'),
    border: cssVar('--glass-border', 'rgba(255, 255, 255, 0.14)'),
  };
  invoke('context_menu_items', { menu: { token, items: native, colors } }).catch((e) => onError(String(e)));
}

/// Один раз на окно: если правый клик никто не обработал — пустой список, и ядро
/// покажет только «Копировать/Вставить» без лишнего ожидания.
export function installContextMenu(errorHandler) {
  if (errorHandler) onError = errorHandler;
  window.addEventListener('contextmenu', () => (claimed = false), true);
  window.addEventListener('contextmenu', () => {
    if (!claimed) send([]);
  });
  listen('context-menu-pick', ({ payload }) => {
    if (payload.token !== token) return;
    const run = runs[payload.id];
    if (run) Promise.resolve().then(run).catch((e) => onError(String(e)));
  });
}

export function setMenuErrorHandler(handler) {
  onError = handler;
}

export function showContextMenu(event, items) {
  if (!items?.length) return;
  claimed = true;
  // Вложенные обработчики (вложение внутри письма) — меню только самого вложенного.
  event.stopPropagation();
  send(items);
}
