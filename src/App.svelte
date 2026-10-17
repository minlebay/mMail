<script>
  import TitleBar from './lib/TitleBar.svelte';
  import Sidebar from './lib/Sidebar.svelte';
  import MessageList from './lib/MessageList.svelte';
  import Reader from './lib/Reader.svelte';
  import LinkDialog from './lib/LinkDialog.svelte';
  import Settings from './lib/Settings.svelte';
  import Splitter from './lib/Splitter.svelte';
  import Compose from './lib/Compose.svelte';
  import { applyTheme, DEFAULT_THEME, DEFAULT_OPACITY } from './lib/themes.js';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import { listFolders, syncStatus, getAppearance, setZoom, listAccounts } from './lib/api.js';

  const win = getCurrentWindow();
  let maximized = $state(false);
  const syncMaximized = () => win.isMaximized().then((m) => (maximized = m));
  syncMaximized();
  $effect(() => {
    window.addEventListener('resize', syncMaximized);
    return () => window.removeEventListener('resize', syncMaximized);
  });

  let appearance = $state({ theme: DEFAULT_THEME, opacity: DEFAULT_OPACITY, middleClickMinimize: true, zoom: 1, pollSeconds: 10 });
  let settingsOpen = $state(false);
  // Текущий масштаб; при запуске ядро уже выставило масштаб по умолчанию из настроек.
  let zoom = $state(1);
  getAppearance().then((a) => {
    appearance = {
      theme: a.theme || DEFAULT_THEME,
      opacity: a.opacity ?? DEFAULT_OPACITY,
      middleClickMinimize: a.middleClickMinimize,
      zoom: a.zoom,
      pollSeconds: a.pollSeconds,
    };
    zoom = a.zoom;
    applyTheme(appearance.theme, appearance.opacity);
  });

  function changeZoom(value) {
    zoom = Math.round(Math.min(2, Math.max(0.5, value)) * 100) / 100;
    setZoom(zoom);
  }

  // Ctrl + / Ctrl − / Ctrl 0 (сброс к масштабу по умолчанию).
  function onkeydown(e) {
    if (!e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === '=' || e.key === '+') changeZoom(zoom + 0.1);
    else if (e.key === '-') changeZoom(zoom - 0.1);
    else if (e.key === '0') changeZoom(appearance.zoom);
    else return;
    e.preventDefault();
  }

  // Ширины панелей: удобство этого компьютера, поэтому localStorage.
  const SIZES = { sidebar: 210, list: 300 };
  let sizes = $state(readSizes());

  function readSizes() {
    try {
      return { ...SIZES, ...JSON.parse(localStorage.getItem('mmail.sizes') ?? '{}') };
    } catch {
      return { ...SIZES };
    }
  }

  function resize(key, px) {
    sizes[key] = px;
    try {
      localStorage.setItem('mmail.sizes', JSON.stringify(sizes));
    } catch {}
  }

  let groups = $state([]);
  // account: null — все ящики, '' — демо, иначе адрес ящика.
  let active = $state({ account: null, folder: 'inbox' });
  let selectedId = $state(null);
  // Растёт, когда ядро сообщает о новых данных: список и счётчики перечитываются.
  let version = $state(0);
  let sync = $state({ state: 'busy', message: 'Подключение…' });

  const refreshFolders = () =>
    listFolders().then((g) => {
      groups = g;
      // Выбранный узел пропал (например, «Все ящики» при одном ящике) — встаём на первый.
      if (g.length && !g.some((x) => x.account === active.account)) active = { account: g[0].account, folder: active.folder };
    });
  refreshFolders();

  $effect(() => {
    const stops = [
      listen('mailbox-changed', () => {
        version++;
        refreshFolders();
      }),
      listen('sync-status', (e) => (sync = e.payload)),
    ];
    // Всё, что случилось до подписки, забираем явно.
    Promise.all(stops).then(() => {
      syncStatus().then((s) => (sync = s));
      version++;
      refreshFolders();
    });
    return () => stops.forEach((p) => p.then((stop) => stop()));
  });

  // Написание письма: отправитель по умолчанию — ящик выбранной слева папки.
  let compose = $state(null);
  let composeAccounts = $state([]);
  let toast = $state('');

  async function openCompose(prefill = {}) {
    composeAccounts = await listAccounts();
    const current = active.account && composeAccounts.includes(active.account) ? active.account : composeAccounts[0];
    const from = prefill.from && composeAccounts.includes(prefill.from) ? prefill.from : current ?? '';
    compose = { from, to: '', cc: '', subject: '', body: '', ...prefill, from };
  }

  function replyTo(msg, kind) {
    const prefix = kind === 'reply' ? 'Re: ' : 'Fwd: ';
    const subject = /^(re|fwd?):/i.test(msg.subject) ? msg.subject : prefix + msg.subject;
    openCompose({ from: msg.account, to: kind === 'reply' ? msg.fromAddr : '', subject });
  }

  function sent() {
    compose = null;
    toast = 'Письмо отправлено';
    setTimeout(() => (toast = ''), 3000);
  }

  function pickFolder(account, folder) {
    active = { account, folder };
    selectedId = null;
  }
</script>

<svelte:window {onkeydown} />

<div class="window" class:maximized>
  <TitleBar
    {sync}
    {zoom}
    onzoom={changeZoom}
    middleClickMinimize={appearance.middleClickMinimize}
    onsettings={() => (settingsOpen = true)}
  />

  <main class="layout" style:grid-template-columns="minmax(150px, {sizes.sidebar}px) 12px minmax(220px, {sizes.list}px) 12px minmax(280px, 1fr)">
    <Sidebar {groups} {active} onpick={pickFolder} oncompose={() => openCompose()} />
    <Splitter value={sizes.sidebar} min={150} max={380} initial={SIZES.sidebar} onchange={(px) => resize('sidebar', px)} />
    <MessageList account={active.account} folder={active.folder} {version} {selectedId} onselect={(id) => (selectedId = id)} />
    <Splitter value={sizes.list} min={220} max={640} initial={SIZES.list} onchange={(px) => resize('list', px)} />
    <Reader id={selectedId} onreply={replyTo} />
  </main>

  <LinkDialog />
  {#if compose}
    <Compose draft={compose} accounts={composeAccounts} onclose={() => (compose = null)} onsent={sent} />
  {/if}
  {#if toast}<div class="toast">{toast}</div>{/if}
  {#if settingsOpen}
    <Settings {appearance} onzoom={changeZoom} onclose={() => (settingsOpen = false)} />
  {/if}
</div>

<style>
  /* Окно без системной рамки и полупрозрачное: то, что под ним, размывает KWin (src-tauri/src/blur.rs).
     Радиус углов должен совпадать с CORNER_RADIUS в main.rs. */
  .window {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    background: var(--window-tint);
  }
  .toast {
    position: absolute;
    left: 50%;
    bottom: 28px;
    z-index: 30;
    transform: translateX(-50%);
    padding: 10px 18px;
    border-radius: 20px;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35);
  }

  .window.maximized {
    border-radius: 0;
    border: 0;
  }

  .layout {
    position: relative;
    flex: 1;
    min-height: 0;
    display: grid;
    padding: 4px 16px 16px;
  }
</style>
