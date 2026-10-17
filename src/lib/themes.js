import { writable } from 'svelte/store';

// Цветовые темы: акцент и оттенок стекла. Прозрачность окна настраивается отдельно.
export const THEMES = [
  { id: 'amethyst', name: 'Аметист', accent: '#8b7bff', tint: [22, 17, 34] },
  { id: 'ocean', name: 'Океан', accent: '#4fb3ff', tint: [10, 22, 38] },
  { id: 'emerald', name: 'Изумруд', accent: '#3ddc97', tint: [10, 28, 24] },
  { id: 'sunset', name: 'Закат', accent: '#ff8a5c', tint: [36, 18, 20] },
  { id: 'rose', name: 'Роза', accent: '#ff5c95', tint: [34, 14, 26] },
  { id: 'graphite', name: 'Графит', accent: '#b4b8c2', tint: [18, 18, 22] },
];

// Текущая схема окна — для документов во iframe (письмо, перевод), у которых свои стили.
export const darkScheme = writable(true);

export const DEFAULT_THEME = 'amethyst';
export const DEFAULT_OPACITY = 0.42;

export function themeById(id) {
  return THEMES.find((t) => t.id === id) ?? THEMES[0];
}

function hexToRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

// Белый текст на кнопке должен читаться: затемняем акцент, пока контраст с белым не станет
// не ниже 3.5:1 (у светлых акцентов вроде «Графита» без этого текст сливается с фоном).
const luminance = (rgb) => {
  const [r, g, b] = rgb.map((c) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
export function strongAccent(hex) {
  let rgb = hexToRgb(hex);
  for (let k = 0; k < 40 && 1.05 / (luminance(rgb) + 0.05) < 3.5; k++) {
    rgb = rgb.map((c) => Math.round(c * 0.96));
  }
  return `rgb(${rgb.join(', ')})`;
}

// Тёмная ли схема для этих настроек: «Как в системе» берёт текущую схему KDE от ядра.
export function isDark(a) {
  const scheme = a?.scheme ?? 'system';
  return scheme === 'system' ? a?.systemDark !== false : scheme === 'dark';
}

// Смесь цвета с белым: светлое стекло с оттенком акцента темы.
const towardWhite = (rgb, amount) => rgb.map((c) => Math.round(255 - (255 - c) * amount));

export function applyTheme(id, opacity = DEFAULT_OPACITY, dark = true, blurEnabled = true) {
  const theme = themeById(id);
  const windowOpacity = blurEnabled ? opacity : 1;
  const alpha = (value) => blurEnabled ? value : 1;
  const el = document.documentElement;
  el.dataset.scheme = dark ? 'dark' : 'light';
  darkScheme.set(dark);
  const root = el.style;
  // На светлом фоне светлые акценты (Графит, Изумруд) не читаются — берём затемнённый.
  const strong = strongAccent(theme.accent);
  const accent = dark ? theme.accent : strong;
  const [r, g, b] = dark ? hexToRgb(theme.accent) : strong.match(/\d+/g).map(Number);
  root.setProperty('--accent', accent);
  root.setProperty('--accent-strong', strong);
  root.setProperty('--accent-soft', `rgba(${r}, ${g}, ${b}, ${dark ? 0.28 : 0.2})`);
  if (dark) {
    root.setProperty('--window-tint', `rgba(${theme.tint.join(', ')}, ${windowOpacity})`);
    // Фон диалогов: оттенок темы, чуть светлее и почти непрозрачный.
    const surface = theme.tint.map((c) => Math.min(255, c + 14));
    root.setProperty('--surface', `rgba(${surface.join(', ')}, ${alpha(0.97)})`);
    root.setProperty('--glass-menu', `rgba(${surface.join(', ')}, ${alpha(0.94)})`);
    root.setProperty('--native-menu', `rgba(${surface.join(', ')}, ${alpha(0.62)})`);
  } else {
    // Та же прозрачность, что у тёмной темы: плотность стекла выбирает пользователь.
    const tint = towardWhite(hexToRgb(theme.accent), 0.1);
    root.setProperty('--window-tint', `rgba(${tint.join(', ')}, ${windowOpacity})`);
    const surface = towardWhite(hexToRgb(theme.accent), 0.04);
    root.setProperty('--surface', `rgba(${surface.join(', ')}, ${alpha(0.98)})`);
    root.setProperty('--glass-menu', `rgba(${surface.join(', ')}, ${alpha(0.96)})`);
    root.setProperty('--native-menu', `rgba(${surface.join(', ')}, ${alpha(0.8)})`);
  }
}
