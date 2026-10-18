// Защиты, которые живут не в коде ядра, а в конфигурации и разметке: их легко случайно
// ослабить, добавляя фичу. Логика ядра проверяется в cargo test.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';

const read = (path) => fs.readFileSync(path, 'utf8');
const json = (path) => JSON.parse(read(path));

test('app CSP allows no remote scripts, frames or network', () => {
  const { app } = json('src-tauri/tauri.conf.json');
  assert.equal(app.withGlobalTauri, false, 'IPC не должен быть доступен любому скрипту через window.__TAURI__');
  assert.equal(app.security.freezePrototype, true);
  for (const csp of [app.security.csp, app.security.devCsp]) {
    const rules = Object.fromEntries(csp.split(';').map((r) => r.trim().split(/\s+/)).map(([name, ...v]) => [name, v]));
    assert.deepEqual(rules['script-src'], ["'self'"], csp);
    assert.deepEqual(rules['frame-src'], ["'none'"], csp);
    assert.deepEqual(rules['object-src'], ["'none'"], csp);
    assert.deepEqual(rules['base-uri'], ["'none'"], csp);
    assert.deepEqual(rules['form-action'], ["'none'"], csp);
    assert.ok(!rules['img-src'].some((s) => /^https?:/.test(s)), 'картинки из сети — только через ядро как data:');
    assert.ok(!rules['connect-src'].some((s) => /^https?:\/\/(?!ipc\.localhost)/.test(s)), csp);
  }
});

test('only the main window can read mail', () => {
  const reading = ['allow-get-message', 'allow-list-messages', 'allow-list-folders', 'allow-save-attachment', 'allow-translate-message', 'allow-open-link', 'allow-calendar-events'];
  for (const name of ['compose', 'settings']) {
    const { permissions, windows } = json(`src-tauri/capabilities/${name}.json`);
    assert.ok(!windows.includes('main'), name);
    for (const p of reading) assert.ok(!permissions.includes(p), `${name}: ${p}`);
  }
  // Сырые возможности Tauri (файлы, shell, http, opener) ни одному окну не нужны.
  for (const name of ['default', 'compose', 'settings']) {
    const { permissions } = json(`src-tauri/capabilities/${name}.json`);
    for (const p of permissions) assert.ok(!/^(fs|shell|http|opener|process):/.test(p), `${name}: ${p}`);
  }
});

test('message frames are sandboxed without scripts or same origin', () => {
  const reader = read('src/lib/Reader.svelte');
  const frames = [...reader.matchAll(/<iframe\b[^>]*>/g)].map((m) => m[0]);
  assert.ok(frames.length >= 2, 'письмо и перевод');
  for (const frame of frames) {
    const sandbox = frame.match(/sandbox="([^"]*)"/);
    assert.ok(sandbox, `iframe без sandbox: ${frame}`);
    assert.ok(!/allow-(scripts|same-origin|forms|top-navigation|modals|downloads)/.test(sandbox[1]), frame);
    assert.match(frame, /referrerpolicy="no-referrer"/);
    assert.match(frame, /srcdoc=/, 'содержимое — из ядра, не по адресу');
  }
});

test('quoted mail in the compose editor cannot cover the window', () => {
  const style = read('src/ComposeWindow.svelte').match(/\.editor\s*\{([^}]*)\}/);
  assert.ok(style, '.editor');
  assert.match(style[1], /contain:\s*paint/, 'position:fixed из цитаты должен оставаться внутри редактора');
});

test('no secrets in tracked files', () => {
  const patterns = [
    [/AIza[0-9A-Za-z_-]{35}/, 'ключ Google API'],
    [/GOCSPX-[0-9A-Za-z_-]{20,}/, 'секрет OAuth-клиента Google'],
    [/\b1\/\/0[0-9A-Za-z_-]{30,}/, 'refresh token Google'],
    [/ya29\.[0-9A-Za-z_-]{20,}/, 'access token Google'],
    [/gh[pousr]_[0-9A-Za-z]{30,}/, 'токен GitHub'],
    [/sk-(ant-)?[0-9A-Za-z_-]{30,}/, 'ключ API'],
    [/-----BEGIN [A-Z ]*PRIVATE KEY-----/, 'приватный ключ'],
    [/[0-9]+-[0-9a-z]{32}\.apps\.googleusercontent\.com/, 'Client ID Google'],
  ];
  const files = execFileSync('git', ['ls-files', '-z'], { encoding: 'utf8' }).split('\0').filter(Boolean);
  const text = files.filter((f) => !/\.(png|jpg|ico|svg)$/.test(f) && fs.existsSync(f));
  assert.ok(text.length > 50);
  for (const file of text) {
    const content = read(file);
    for (const [pattern, what] of patterns) assert.ok(!pattern.test(content), `${file}: похоже на ${what}`);
  }
});
