// Форматирование дат на языке интерфейса.
import { derived } from 'svelte/store';
import { language } from '../i18n.js';

const LOCALES = { ru: 'ru-RU', en: 'en-GB', sr: 'sr-Latn-RS' };

export const fmt = derived(language, (code) => {
  const locale = LOCALES[code] ?? 'ru-RU';
  const make = (options) => new Intl.DateTimeFormat(locale, options);
  const time = make({ hour: '2-digit', minute: '2-digit' });
  const weekday = make({ weekday: 'short' });
  const monthYear = make({ month: 'long', year: 'numeric' });
  const dayMonth = make({ day: 'numeric', month: 'long' });
  const full = make({ weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
  const cap = (s) => s.charAt(0).toUpperCase() + s.slice(1);
  return {
    time: (d) => time.format(d),
    weekday: (d) => cap(weekday.format(d)),
    monthYear: (d) => cap(monthYear.format(d)),
    dayMonth: (d) => dayMonth.format(d),
    full: (d) => cap(full.format(d)),
  };
});
