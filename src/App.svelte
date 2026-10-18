<script>
  import { wideLayoutEnabled } from './lib/layout.js';
  import { t } from './lib/i18n.js';
  import TitleBar from './lib/TitleBar.svelte';
  import Sidebar from './lib/Sidebar.svelte';
  import MessageList from './lib/MessageList.svelte';
  import Reader from './lib/Reader.svelte';
  import LinkDialog from './lib/LinkDialog.svelte';
  import TranslationDialog from './lib/TranslationDialog.svelte';
  import Splitter from './lib/Splitter.svelte';
  import CalendarSidebar from './lib/calendar/CalendarSidebar.svelte';
  import CalendarView from './lib/calendar/CalendarView.svelte';
  import EventDialog from './lib/calendar/EventDialog.svelte';
  import { visibleRange, toRfc3339, fromDateKey } from './lib/calendar/dates.js';
  import { applyTheme, isDark, DEFAULT_THEME, DEFAULT_OPACITY } from './lib/themes.js';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import { listFolders, syncStatus, getAppearance, setZoom, openSettings, openCompose as openComposeWindow } from './lib/api.js';
  import { googleStatus, calendarList, calendarSetVisible, calendarEvents, calendarSaveEvent, calendarDeleteEvent } from './lib/api.js';
  import { setMenuErrorHandler } from './lib/contextMenu.js';

  const win = getCurrentWindow();
  let maximized = $state(false);
  const syncMaximized = () => win.isMaximized().then((m) => (maximized = m));
  syncMaximized();
  $effect(() => {
    window.addEventListener('resize', syncMaximized);
    return () => window.removeEventListener('resize', syncMaximized);
  });

  let appearance = $state({ theme: DEFAULT_THEME, opacity: DEFAULT_OPACITY, middleClickMinimize: true, zoom: 1, pollSeconds: 10 });
  // Текущий масштаб; при запуске ядро уже выставило масштаб по умолчанию из настроек.
  let zoom = $state(1);
  getAppearance().then((a) => {
    appearance = {
      theme: a.theme || DEFAULT_THEME,
      opacity: a.opacity ?? DEFAULT_OPACITY,
      scheme: a.scheme,
      blurEnabled: a.blurEnabled ?? true,
      wideLayout: a.wideLayout ?? false,
      systemDark: a.systemDark,
      middleClickMinimize: a.middleClickMinimize,
      zoom: a.zoom,
      pollSeconds: a.pollSeconds,
      calendarView: a.calendarView,
      calendarFullTitles: a.calendarFullTitles ?? false,
    };
    zoom = a.zoom;
    calView = a.calendarView ?? 'week';
    applyTheme(appearance.theme, appearance.opacity, isDark(appearance), appearance.blurEnabled ?? true);
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
  const SIZES = { sidebar: 210, list: 300, calendar: 900 };
  let sizes = $state(readSizes());
  let viewportWidth = $state(window.innerWidth);
  const wide = $derived(wideLayoutEnabled(appearance.wideLayout, viewportWidth, sizes.sidebar, sizes.list));

  const calendarMax = $derived(Math.max(560, viewportWidth - sizes.sidebar - sizes.list - 70 - 360));
  const calendarWidth = $derived(Math.min(calendarMax, Math.max(560, sizes.calendar)));

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
      // Настройки живут в своём окне: тему и масштаб по умолчанию применяем, когда их там меняют.
      listen('appearance-changed', (e) => {
        const a = e.payload;
        if (a.zoom !== appearance.zoom) changeZoom(a.zoom);
        // Вид по умолчанию поменяли в настройках — показываем его сразу.
        if (a.calendarView && a.calendarView !== appearance.calendarView) calView = a.calendarView;
        appearance = { ...a, theme: a.theme || DEFAULT_THEME, opacity: a.opacity ?? DEFAULT_OPACITY };
        applyTheme(appearance.theme, appearance.opacity, isDark(appearance), appearance.blurEnabled ?? true);
      }),
      listen('message-sent', () => showToast('Письмо отправлено')),
      listen('google-changed', () => refreshGoogle()),
      // Клик по уведомлению: новое письмо (или несколько — тогда «Входящие» ящика) и день события.
      listen('open-message', ({ payload }) => {
        setMode('mail');
        if (groups.some((g) => g.account === payload.account)) active = { account: payload.account, folder: 'inbox' };
        if (payload.id != null) selectedId = payload.id;
      }),
      listen('open-date', ({ payload }) => {
        setMode('calendar');
        calDate = fromDateKey(payload);
        calView = 'day';
      }),
      listen('operation-error', (e) => showToast(String(e.payload))),
      // Удалённое письмо больше не показываем в просмотре.
      listen('message-trashed', (e) => {
        if (selectedId === e.payload) selectedId = null;
      }),
    ];
    // Всё, что случилось до подписки, забираем явно.
    Promise.all(stops).then(() => {
      syncStatus().then((s) => (sync = s));
      version++;
      refreshFolders();
    });
    return () => stops.forEach((p) => p.then((stop) => stop()));
  });

  // Написание письма — отдельное окно; отправитель по умолчанию — ящик выбранной слева папки.
  let toast = $state('');

  function openCompose(prefill = {}) {
    const current = active.account || null;
    return openComposeWindow({ kind: 'new', from: current, ...prefill });
  }

  function replyTo(msg, kind) {
    const prefix = kind === 'reply' ? 'Re: ' : 'Fwd: ';
    const subject = /^(re|fwd?):/i.test(msg.subject) ? msg.subject : prefix + msg.subject;
    return openCompose({ kind, from: msg.account, to: kind === 'reply' ? msg.fromAddr : '', subject, source: msg.id });
  }

  // Ошибки действий из контекстного меню — всплывающим сообщением.
  setMenuErrorHandler((text) => showToast(text));

  function showToast(text) {
    toast = text;
    setTimeout(() => (toast = ''), 3000);
  }

  // Режим окна: почта или календарь (удобство этого компьютера — localStorage).
  let mode = $state(readMode());
  function readMode() {
    try {
      return localStorage.getItem('mmail.mode') === 'calendar' ? 'calendar' : 'mail';
    } catch {
      return 'mail';
    }
  }
  function setMode(next) {
    mode = next;
    try {
      localStorage.setItem('mmail.mode', next);
    } catch {}
  }

  // Календарь: список календарей и события видимого периода (только в памяти).
  let googleAccount = $state(null);
  let calendars = $state([]);
  let events = $state([]);
  let calDate = $state(new Date());
  let calView = $state('week');
  let calLoading = $state(false);
  let calError = $state('');
  let eventDialog = $state(null); // { event } | { preset }
  let calVersion = $state(0);
  const calColors = $derived(Object.fromEntries(calendars.map((c) => [c.id, c.color])));

  const refreshGoogle = () =>
    googleStatus().then((s) => {
      googleAccount = s.account;
      if (googleAccount) calendarList().then((list) => (calendars = list), (e) => (calError = String(e)));
      else {
        calendars = [];
        events = [];
      }
    });
  refreshGoogle();

  $effect(() => {
    if ((mode !== 'calendar' && !wide) || !googleAccount) return;
    const visible = calendars.filter((c) => c.visible).map((c) => c.id);
    const [from, to] = visibleRange(calView, calDate);
    calVersion;
    let cancelled = false;
    calLoading = true;
    calError = '';
    calendarEvents(visible, toRfc3339(from), toRfc3339(to))
      .then((list) => !cancelled && (events = list))
      .catch((e) => !cancelled && (calError = String(e)))
      .finally(() => !cancelled && (calLoading = false));
    // Изменения из других клиентов — раз в 5 минут, пока календарь на экране.
    const timer = setInterval(() => calVersion++, 5 * 60 * 1000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  });

  function toggleCalendar(id, visible) {
    calendars = calendars.map((c) => (c.id === id ? { ...c, visible } : c));
    calendarSetVisible(id, visible).catch((e) => showToast(String(e)));
  }

  async function saveEvent(calendarId, eventId, draft) {
    await calendarSaveEvent(calendarId, eventId, draft);
    calVersion++;
  }

  async function deleteEvent(calendarId, eventId) {
    await calendarDeleteEvent(calendarId, eventId);
    calVersion++;
  }

  function pickFolder(account, folder) {
    active = { account, folder };
    selectedId = null;
  }
</script>

{#snippet calendarPane()}
      <CalendarView
        date={calDate}
        ondate={(d) => (calDate = d)}
        view={calView}
        onview={(v) => (calView = v)}
        fullTitles={appearance.calendarFullTitles ?? false}
        {events}
        colors={calColors}
        onopen={(event) => (eventDialog = { event })}
        oncreate={(start, allDay) => (eventDialog = { preset: { start, allDay } })}
        loading={calLoading}
        error={calError}
        connected={!!googleAccount}
        onconnect={openSettings}
      />
{/snippet}

{#snippet calendarSidebar()}
      <CalendarSidebar
        {mode}
        onmode={setMode}
        date={calDate}
        onpick={(d) => (calDate = d)}
        {calendars}
        ontoggle={toggleCalendar}
        oncreate={() => (eventDialog = { preset: { start: new Date(calDate.getFullYear(), calDate.getMonth(), calDate.getDate(), new Date().getHours() + 1) } })}
        connected={!!googleAccount}
      />
{/snippet}

<svelte:window {onkeydown} bind:innerWidth={viewportWidth} />

<div class="window" class:maximized>
  <TitleBar
    {sync}
    {zoom}
    onzoom={changeZoom}
    middleClickMinimize={appearance.middleClickMinimize}
    onsettings={openSettings}
  />

  {#if mode === 'calendar' && !wide}
    <main class="layout" style:grid-template-columns="minmax(150px, {sizes.sidebar}px) 12px minmax(400px, 1fr)">
      {@render calendarSidebar()}
      <Splitter value={sizes.sidebar} min={150} max={380} initial={SIZES.sidebar} onchange={(px) => resize('sidebar', px)} />
      {@render calendarPane()}
    </main>
  {:else}
  <main class="layout" style:grid-template-columns="minmax(150px, {sizes.sidebar}px) 12px minmax(220px, {sizes.list}px) 12px minmax({wide ? 360 : 280}px, 1fr){wide ? ` 12px ${calendarWidth}px` : ''}">
    {#if wide && mode === 'calendar'}
      {@render calendarSidebar()}
    {:else}
    <Sidebar {mode} onmode={setMode} {groups} {active} onpick={pickFolder} oncompose={(from) => openCompose(from ? { from } : {})} onrefresh={() => { version++; refreshFolders(); }} onerror={showToast} />
    {/if}
    <Splitter value={sizes.sidebar} min={150} max={380} initial={SIZES.sidebar} onchange={(px) => resize('sidebar', px)} />
    <MessageList onreply={replyTo} onerror={showToast} account={active.account} folder={active.folder} {version} {selectedId} onselect={(id) => (selectedId = id)} />
    <Splitter value={sizes.list} min={220} max={640} initial={SIZES.list} onchange={(px) => resize('list', px)} />
    <Reader id={selectedId} onreply={replyTo} />
    {#if wide}
      <Splitter value={calendarWidth} min={560} max={calendarMax} initial={SIZES.calendar} reverse onchange={(px) => resize('calendar', px)} />
      {@render calendarPane()}
    {/if}
  </main>
  {/if}
  {#if eventDialog}
    <EventDialog
      event={eventDialog.event ?? null}
      preset={eventDialog.preset ?? null}
      {calendars}
      onclose={() => (eventDialog = null)}
      onsave={saveEvent}
      ondelete={deleteEvent}
    />
  {/if}

  <LinkDialog />
  <TranslationDialog />
  {#if toast}<div class="toast">{$t(toast)}</div>{/if}
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
    border: 1px solid rgba(var(--ink), 0.14);
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
