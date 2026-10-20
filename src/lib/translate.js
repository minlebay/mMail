// Запрос «перевести письмо» из списка: Reader подхватывает его, когда письмо откроется.
import { writable } from 'svelte/store';

export const translateRequest = writable(null);
export const requestTranslation = (id) => translateRequest.set({ id, at: Date.now() });

// Языки перевода — те же коды, что в translation::LANGUAGES (ядро принимает только их).
// Названия на самих языках: так свой язык легко найти при любом языке интерфейса.
export const TARGET_LANGUAGES = [
  { value: 'ru', label: 'Русский' },
  { value: 'en', label: 'English' },
  { value: 'sr', label: 'Српски' },
  { value: 'uk', label: 'Українська' },
  { value: 'de', label: 'Deutsch' },
  { value: 'fr', label: 'Français' },
  { value: 'es', label: 'Español' },
  { value: 'it', label: 'Italiano' },
  { value: 'pt', label: 'Português' },
  { value: 'pl', label: 'Polski' },
  { value: 'nl', label: 'Nederlands' },
  { value: 'cs', label: 'Čeština' },
  { value: 'tr', label: 'Türkçe' },
  { value: 'zh-CN', label: '中文' },
  { value: 'ja', label: '日本語' },
  { value: 'ko', label: '한국어' },
];
