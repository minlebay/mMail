import { writable } from 'svelte/store';

export const contextMenu = writable(null);
export function showContextMenu(event, items) {
  event.preventDefault();
  event.stopPropagation();
  const anchor = event.currentTarget;
  const rect = anchor.getBoundingClientRect();
  contextMenu.set({
    x: event.clientX || rect.left + 16,
    y: event.clientY || rect.top + 16,
    items,
    anchor,
  });
}
export function closeContextMenu() {
  contextMenu.set(null);
}
export function menuPosition(x, y, width, height, viewportWidth, viewportHeight) {
  return {
    x: Math.max(8, Math.min(x, viewportWidth - width - 8)),
    y: Math.max(8, Math.min(y, viewportHeight - height - 8)),
  };
}
