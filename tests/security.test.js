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

// Письмо не попадает в страницу приложения: его показывает отдельный WebKit (mailview.rs)
// со своим эфемерным контекстом, без JavaScript и без навигации — ссылки только через подтверждение.
test('mail body lives in an isolated native view without scripts', () => {
  const reader = read('src/lib/Reader.svelte');
  assert.ok(!/<iframe\b/.test(reader), 'письмо не встраивается в страницу приложения');
  assert.ok(!/\{@html/.test(reader), 'и не вставляется как разметка');
  const view = read('src-tauri/src/mailview.rs');
  assert.match(view, /set_enable_javascript\(false\)/);
  assert.match(view, /WebContext::new_ephemeral\(\)/);
  assert.match(view, /connect_decide_policy/);
  assert.match(view, /decision\.ignore\(\)/);
  assert.ok(!/allow-scripts|nonce/.test(read('src-tauri/src/sanitize.rs')), 'в документе письма скриптов нет');
});

test('quoted mail in the compose editor cannot cover the window', () => {
  const style = read('src/ComposeWindow.svelte').match(/\.editor\s*\{([^}]*)\}/);
  assert.ok(style, '.editor');
  assert.match(style[1], /contain:\s*paint/, 'position:fixed из цитаты должен оставаться внутри редактора');
});

test('pasted and dropped HTML in compose goes through the core sanitizer', () => {
  const compose = read('src/ComposeWindow.svelte');
  assert.match(compose, /onpaste=\{onPaste\}/, 'paste должен перехватываться');
  assert.match(compose, /ondrop=\{onDrop\}/, 'drop должен перехватываться');
  assert.match(compose, /insertSanitized\(html,/, 'HTML должен идти через общий sanitizer');
  assert.match(compose, /cleanComposeHtml\(html\)/, 'ядро чистит HTML тем же фильтром, что и письма');
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
