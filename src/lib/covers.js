import { writable } from 'svelte/store';

// Тело письма — нативный WebKit поверх страницы (mailview.rs): HTML-диалог под ним не виден.
// Пока открыт хоть один диалог с `use:covers`, Reader прячет письмо.
export const covered = writable(0);

export function covers() {
  covered.update((n) => n + 1);
  return { destroy: () => covered.update((n) => n - 1) };
}
