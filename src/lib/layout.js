// Widths are CSS pixels: WebView zoom is already reflected in innerWidth.
export function wideLayoutEnabled(enabled, width, sidebar, list) {
  // Outer padding/borders + three dividers + reader + calendar.
  const required = sidebar + list + 34 + 36 + 360 + 560;
  return Boolean(enabled) && width >= Math.max(2560, required);
}
