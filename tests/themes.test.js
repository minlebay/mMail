import test from 'node:test';
import assert from 'node:assert/strict';
import { applyTheme, THEMES } from '../src/lib/themes.js';

test('light and dark themes preserve the selected glass opacity', () => {
  const previous = globalThis.document;
  const values = new Map();
  globalThis.document = { documentElement: { dataset: {}, style: { setProperty: (key, value) => values.set(key, value) } } };
  try {
    for (const dark of [true, false]) {
      for (const theme of THEMES) {
        for (const opacity of [0.15, 0.42, 0.85]) {
          applyTheme(theme.id, opacity, dark);
          assert.ok(values.get('--window-tint').endsWith(`, ${opacity})`));
          assert.equal(document.documentElement.dataset.scheme, dark ? 'dark' : 'light');
        }
      }
    }
  } finally {
    globalThis.document = previous;
  }
});

test('disabling blur makes windows and menus opaque and restores the saved opacity', () => {
  const previous = globalThis.document;
  const values = new Map();
  globalThis.document = { documentElement: { dataset: {}, style: { setProperty: (key, value) => values.set(key, value) } } };
  try {
    for (const dark of [true, false]) {
      applyTheme('graphite', 0.27, dark, false);
      for (const key of ['--window-tint', '--surface', '--glass-menu', '--native-menu']) {
        assert.ok(values.get(key).endsWith(', 1)'), key);
      }
      applyTheme('graphite', 0.27, dark, true);
      assert.ok(values.get('--window-tint').endsWith(', 0.27)'));
    }
  } finally { globalThis.document = previous; }
});
