<script>
  import TitleBar from './lib/TitleBar.svelte';
  import Sidebar from './lib/Sidebar.svelte';
  import MessageList from './lib/MessageList.svelte';
  import Reader from './lib/Reader.svelte';
  import LinkDialog from './lib/LinkDialog.svelte';
  import Settings from './lib/Settings.svelte';
  import Splitter from './lib/Splitter.svelte';
  import { applyTheme, DEFAULT_THEME, DEFAULT_OPACITY } from './lib/themes.js';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import { listFolders, syncStatus, getAppearance } from './lib/api.js';

  const win = getCurrentWindow();
  let maximized = $state(false);
  const syncMaximized = () => win.isMaximized().then((m) => (maximized = m));
  syncMaximized();
  $effect(() => {
    window.addEventListener('resize', syncMaximized);
    return () => window.removeEventListener('resize', syncMaximized);
  });

  let appearance = $state({ theme: DEFAULT_THEME, opacity: DEFAULT_OPACITY, middleClickMinimize: true });
  let settingsOpen = $state(false);
  getAppearance().then((a) => {
    appearance = { theme: a.theme || DEFAULT_THEME, opacity: a.opacity ?? DEFAULT_OPACITY, middleClickMinimize: a.middleClickMinimize };
    applyTheme(appearance.theme, appearance.opacity);
  });

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

  function pickFolder(account, folder) {
    active = { account, folder };
    selectedId = null;
  }
</script>

<div class="window" class:maximized>
  <TitleBar {sync} middleClickMinimize={appearance.middleClickMinimize} onsettings={() => (settingsOpen = true)} />

  <main class="layout" style:grid-template-columns="{sizes.sidebar}px 12px {sizes.list}px 12px minmax(320px, 1fr)">
    <Sidebar {groups} {active} onpick={pickFolder} oncompose={() => {}} />
    <Splitter value={sizes.sidebar} min={150} max={380} initial={SIZES.sidebar} onchange={(px) => resize('sidebar', px)} />
    <MessageList account={active.account} folder={active.folder} {version} {selectedId} onselect={(id) => (selectedId = id)} />
    <Splitter value={sizes.list} min={220} max={640} initial={SIZES.list} onchange={(px) => resize('list', px)} />
    <Reader id={selectedId} />
  </main>

  <LinkDialog />
  {#if settingsOpen}
    <Settings {appearance} onclose={() => (settingsOpen = false)} />
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
