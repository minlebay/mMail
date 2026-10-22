// Перетаскивание писем из списка на папку в дереве. Своё, на событиях указателя:
// HTML5 drag-and-drop в WebKitGTK перехватывает приём файлов Tauri.
// Цель броска — элемент с data-drop-folder (и data-drop-account: '*' — «Все ящики»).
import { writable } from 'svelte/store';

const THRESHOLD = 5;

/// Над какой папкой сейчас письмо: { account, folder } или null.
export const dropTarget = writable(null);

let drag = null;

/// Вызывать из pointerdown строки. `canDrop(account, folder)` решает, подсветится ли папка,
/// `ondrop(account, folder)` — бросили на неё.
export function startDrag(event, { label, canDrop, ondrop }) {
  if (event.button !== 0 || drag) return;
  drag = { x: event.clientX, y: event.clientY, label, canDrop, ondrop, ghost: null, target: null };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
  window.addEventListener('pointercancel', cancel);
  window.addEventListener('keydown', key, true);
  // CSS user-select WebKitGTK тут не останавливает: выделение запрещаем событием.
  document.addEventListener('selectstart', noSelect, true);
  window.getSelection()?.removeAllRanges();
}

function noSelect(event) {
  event.preventDefault();
}

function move(event) {
  if (!drag.ghost) {
    if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < THRESHOLD) return;
    drag.ghost = document.createElement('div');
    drag.ghost.className = 'drag-ghost';
    drag.ghost.textContent = drag.label;
    document.body.append(drag.ghost);
    document.body.classList.add('dragging');
    window.getSelection()?.removeAllRanges();
  }
  drag.ghost.style.transform = `translate(${event.clientX + 14}px, ${event.clientY + 10}px)`;
  const el = document.elementFromPoint(event.clientX, event.clientY)?.closest('[data-drop-folder]');
  const account = el ? (el.dataset.dropAccount === '*' ? null : el.dataset.dropAccount) : undefined;
  const target = el && drag.canDrop(account, el.dataset.dropFolder) ? { account, folder: el.dataset.dropFolder } : null;
  if (target?.account !== drag.target?.account || target?.folder !== drag.target?.folder) {
    drag.target = target;
    dropTarget.set(target);
  }
}

function up() {
  const { target, ondrop, ghost } = drag;
  // Строка под указателем получит click после отпускания — после переноса он не нужен.
  if (ghost) {
    window.addEventListener('click', swallow, true);
    setTimeout(() => window.removeEventListener('click', swallow, true));
  }
  cancel();
  if (target) ondrop(target.account, target.folder);
}

function swallow(event) {
  event.stopPropagation();
  event.preventDefault();
}

function key(event) {
  if (event.key !== 'Escape' || !drag?.ghost) return;
  event.stopPropagation();
  cancel();
}

function cancel() {
  window.removeEventListener('pointermove', move);
  window.removeEventListener('pointerup', up);
  window.removeEventListener('pointercancel', cancel);
  window.removeEventListener('keydown', key, true);
  document.removeEventListener('selectstart', noSelect, true);
  drag?.ghost?.remove();
  document.body.classList.remove('dragging');
  dropTarget.set(null);
  drag = null;
}
