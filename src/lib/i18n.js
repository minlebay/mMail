import { writable, derived } from 'svelte/store';
import messages from './translations.json' with { type: 'json' };

export const LANGUAGES = [
  { value: 'ru', label: 'Русский' },
  { value: 'en', label: 'English' },
  { value: 'sr', label: 'Srpski' },
];
export const language = writable('ru');
export const normalizeLanguage = (value) => LANGUAGES.some((item) => item.value === value) ? value : 'ru';
export function setLanguage(value) {
  const code = normalizeLanguage(value);
  language.set(code);
  if (typeof document !== 'undefined') document.documentElement.lang = code === 'sr' ? 'sr-Latn' : code;
}
const templates = Object.keys(messages).filter((key) => /\{\w+\}/.test(key)).map((key) => {
  const names = [...key.matchAll(/\{(\w+)\}/g)].map((match) => match[1]);
  const parts = key.split(/\{\w+\}/).map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  return { key, names, pattern: new RegExp('^' + parts.join('(.+?)') + '$') };
});
export function translate(code, key, params = {}) {
  if (typeof key !== 'string') return String(key ?? '');
  if (code !== 'ru' && !messages[key]) {
    for (const entry of templates) {
      const match = key.match(entry.pattern);
      if (match) return translate(code, entry.key, Object.fromEntries(entry.names.map((name, i) => [name, match[i + 1]])));
    }
  }
  const template = messages[key]?.[code] ?? key;
  return template.replace(/\{(\w+)\}/g, (match, name) => params[name] ?? match);
}
export const t = derived(language, (code) => (key, params) => translate(code, key, params));

// Status strings from the backend contain an account prefix and may contain several accounts.
export function statusText(t, value) {
  return value.split(' · ').map((part) => {
    const translated = t(part);
    if (translated !== part) return translated;
    const separator = part.indexOf(': ');
    return separator < 0 ? translated : t(part.slice(0, separator)) + ': ' + t(part.slice(separator + 2));
  }).join(' · ');
}
