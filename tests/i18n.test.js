import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { get } from 'svelte/store';
import { parse } from 'svelte/compiler';
import { translate, setLanguage, normalizeLanguage, t, statusText } from '../src/lib/i18n.js';
import messages from '../src/lib/translations.json' with { type: 'json' };

const placeholders = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
test('all translations are complete and preserve parameters', () => {
  for (const [key, translations] of Object.entries(messages)) {
    for (const code of ['en', 'sr']) {
      assert.ok(translations[code]?.trim(), `${code}: ${key}`);
      assert.deepEqual(placeholders(translations[code]), placeholders(key), `${code}: ${key}`);
    }
  }
});
test('language changes notify existing subscribers and unknown preferences fall back', () => {
  const seen = [];
  const unsubscribe = t.subscribe((translate) => seen.push(translate('Настройки')));
  setLanguage('en');
  setLanguage('sr');
  setLanguage('ru');
  unsubscribe();
  assert.deepEqual(seen, ['Настройки', 'Settings', 'Podešavanja', 'Настройки']);
  assert.equal(normalizeLanguage(undefined), 'ru');
  assert.equal(normalizeLanguage('unknown'), 'ru');
  assert.equal(get(t)('Unknown external error'), 'Unknown external error');
});
test('dynamic UI parameters and backend statuses preserve addresses and details', () => {
  assert.equal(translate('en', 'Всегда для {address}', { address: 'test@example.com' }), 'Always for test@example.com');
  assert.equal(translate('sr', 'не удалось отправить: SMTP 550'), 'slanje nije uspelo: SMTP 550');
  assert.equal(statusText((key) => translate('en', key), 'a@example.com: вход… · Хранилище: открываю…'), 'a@example.com: signing in… · Storage: opening…');
  assert.equal(translate('en', 'Домен «пример.рф» содержит не-латинские символы — возможна подмена адреса'), 'The domain “пример.рф” contains non-Latin characters — possible address spoofing');
});
test('components have no untranslated Russian text nodes or missing literal keys', () => {
  const paths = ['src/App.svelte', 'src/ComposeWindow.svelte', 'src/SettingsWindow.svelte', ...fs.readdirSync('src/lib').filter((p) => p.endsWith('.svelte')).map((p) => `src/lib/${p}`)];
  for (const path of paths) {
    const ast = parse(fs.readFileSync(path, 'utf8'));
    const seen = new Set();
    function walk(node) {
      if (!node || typeof node !== 'object' || seen.has(node)) return;
      seen.add(node);
      if (node.type === 'Text') assert.ok(!/[А-Яа-яЁё]/.test(node.data), `${path}: ${node.data}`);
      if (node.type === 'CallExpression' && node.callee?.name === '$t' && node.arguments[0]?.type === 'Literal') {
        assert.ok(messages[node.arguments[0].value], `${path}: missing ${node.arguments[0].value}`);
      }
      for (const value of Object.values(node)) Array.isArray(value) ? value.forEach(walk) : walk(value);
    }
    walk(ast.html);
    walk(ast.instance);
  }
});
