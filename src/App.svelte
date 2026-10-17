<script>
  import TitleBar from './lib/TitleBar.svelte';
  import Sidebar from './lib/Sidebar.svelte';
  import MessageList from './lib/MessageList.svelte';
  import Reader from './lib/Reader.svelte';
  import LinkDialog from './lib/LinkDialog.svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import { listFolders, syncStatus } from './lib/api.js';

  const win = getCurrentWindow();
  let maximized = $state(false);
  const syncMaximized = () => win.isMaximized().then((m) => (maximized = m));
  syncMaximized();
  $effect(() => {
    window.addEventListener('resize', syncMaximized);
    return () => window.removeEventListener('resize', syncMaximized);
  });

  let folders = $state([]);
  let folder = $state('inbox');
  let selectedId = $state(null);
  // Растёт, когда ядро сообщает о новых данных: список и счётчики перечитываются.
  let version = $state(0);
  let sync = $state({ state: 'busy', message: 'Подключение…' });

  const refreshFolders = () => listFolders().then((f) => (folders = f));
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

  function pickFolder(id) {
    folder = id;
    selectedId = null;
  }
</script>

<div class="window" class:maximized>
  <TitleBar {sync} />

  <main class="layout">
    <Sidebar {folders} active={folder} onpick={pickFolder} />
    <MessageList {folder} {version} {selectedId} onselect={(id) => (selectedId = id)} />
    <Reader id={selectedId} />
  </main>

  <LinkDialog />
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
    grid-template-columns: 190px minmax(260px, 320px) 1fr;
    gap: 12px;
    padding: 4px 16px 16px;
  }
</style>
