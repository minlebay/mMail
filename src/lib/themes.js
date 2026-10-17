// Цветовые темы: акцент и оттенок стекла. Прозрачность окна настраивается отдельно.
export const THEMES = [
  { id: 'amethyst', name: 'Аметист', accent: '#8b7bff', tint: [22, 17, 34] },
  { id: 'ocean', name: 'Океан', accent: '#4fb3ff', tint: [10, 22, 38] },
  { id: 'emerald', name: 'Изумруд', accent: '#3ddc97', tint: [10, 28, 24] },
  { id: 'sunset', name: 'Закат', accent: '#ff8a5c', tint: [36, 18, 20] },
  { id: 'rose', name: 'Роза', accent: '#ff5c95', tint: [34, 14, 26] },
  { id: 'graphite', name: 'Графит', accent: '#b4b8c2', tint: [18, 18, 22] },
];

export const DEFAULT_THEME = 'amethyst';
export const DEFAULT_OPACITY = 0.42;

export function themeById(id) {
  return THEMES.find((t) => t.id === id) ?? THEMES[0];
}

function hexToRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function applyTheme(id, opacity = DEFAULT_OPACITY) {
  const theme = themeById(id);
  const [r, g, b] = hexToRgb(theme.accent);
  const root = document.documentElement.style;
  root.setProperty('--accent', theme.accent);
  root.setProperty('--accent-soft', `rgba(${r}, ${g}, ${b}, 0.28)`);
  root.setProperty('--window-tint', `rgba(${theme.tint.join(', ')}, ${opacity})`);
  // Фон диалогов: оттенок темы, чуть светлее и почти непрозрачный.
  const surface = theme.tint.map((c) => Math.min(255, c + 14));
  root.setProperty('--surface', `rgba(${surface.join(', ')}, 0.97)`);
  root.setProperty('--glass-menu', `rgba(${surface.join(', ')}, 0.94)`);
  root.setProperty('--native-menu', `rgba(${surface.join(', ')}, 0.62)`);
}
