<script>
  // Настройки — отдельное окно (метка "settings"); размер подстраивается под содержимое.
  import { untrack, onMount } from 'svelte';
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import Icon from './Icon.svelte';
  import Select from './Select.svelte';
  import TitleBar from './TitleBar.svelte';
  import { THEMES, applyTheme } from './themes.js';
  import { setAppearance, listAccounts, addAccount, removeAccount, storageInfo, setBodyDays, clearCache } from './api.js';

  let { appearance } = $props();

  const win = getCurrentWindow();
  const onclose = () => win.close();
  let maximized = $state(false);
  let titlebar = $state();
  let content = $state();

  // Высота окна = шапка + содержимое, но не больше 90% экрана (дальше — прокрутка).
  // Переводим CSS-пиксели в логические через текущее соотношение: у окна может быть свой масштаб.
  async function fitToContent() {
    if (!content || !titlebar || maximized) return;
    const scale = await win.scaleFactor();
    const inner = await win.innerSize();
    const ratio = inner.height / scale / window.innerHeight;
    const style = getComputedStyle(content);
    const margins = parseFloat(style.marginTop) + parseFloat(style.marginBottom);
    // + рамка окна (по 1px сверху и снизу).
    const css = titlebar.offsetHeight + content.offsetHeight + margins + 2;
    const height = Math.min(css * ratio, window.screen.availHeight * 0.9);
    await win.setSize(new LogicalSize(inner.width / scale, Math.round(height)));
  }

  onMount(() => {
    const observer = new ResizeObserver(() => fitToContent());
    observer.observe(content);
    const syncMaximized = () => win.isMaximized().then((m) => (maximized = m));
    window.addEventListener('resize', syncMaximized);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', syncMaximized);
    };
  });

  let tab = $state('appearance');
  // Начальные значения формы; дальше форма сама пишет в appearance.
  const initial = untrack(() => $state.snapshot(appearance));
  let theme = $state(initial.theme);
  let opacity = $state(initial.opacity);
  let middleClickMinimize = $state(initial.middleClickMinimize);
  let zoom = $state(initial.zoom);
  let pollSeconds = $state(initial.pollSeconds);
  let trayMode = $state(initial.trayMode);
  let closeToTray = $state(initial.closeToTray);
  let trayIcon = $state(initial.trayIcon);
  const TRAY_ICONS = [
    ['color', 'Цветной'],
    ['auto', 'Монохромный, по теме KDE'],
    ['light', 'Монохромный светлый (тёмная панель)'],
    ['dark', 'Монохромный тёмный (светлая панель)'],
  ];
  const TRAY = [
    ['panelTray', 'В панели задач и в трее'],
    ['tray', 'Только в трее'],
    ['panel', 'Только в панели задач'],
  ];
  const POLL = [
    [5, 'каждые 5 секунд'],
    [10, 'каждые 10 секунд'],
    [30, 'каждые 30 секунд'],
    [60, 'каждую минуту'],
    [300, 'каждые 5 минут'],
  ];

  // Тема применяется сразу, в конфиг пишется с небольшой задержкой (ползунок дёргается часто).
  $effect(() => {
    applyTheme(theme, opacity);
    appearance.theme = theme;
    appearance.opacity = opacity;
    appearance.middleClickMinimize = middleClickMinimize;
    appearance.zoom = zoom;
    appearance.pollSeconds = pollSeconds;
    appearance.trayMode = trayMode;
    appearance.closeToTray = closeToTray;
    appearance.trayIcon = trayIcon;
    const t = setTimeout(
      () => setAppearance({ theme, opacity, middleClickMinimize, zoom, pollSeconds, trayMode, trayIcon, closeToTray }),
      300,
    );
    return () => clearTimeout(t);
  });

  // Select не различает null и «нет значения», поэтому «хранить всё» — строка 'all'.
  const BODY_DAYS = [
    ['all', 'всех писем'],
    [365, 'писем за последний год'],
    [180, 'писем за полгода'],
    [90, 'писем за 3 месяца'],
    [30, 'писем за месяц'],
    [0, 'только открытых, до перезапуска'],
  ];
  let bodyDays = $state(null);
  let cacheBytes = $state(null);
  let cleaning = $state(false);
  const showStorage = (info) => {
    cacheBytes = info.bytes;
    bodyDays = info.bodyDays ?? 'all';
  };
  storageInfo().then(showStorage);
  const megabytes = (bytes) => `${(bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} МБ`;

  async function cleanup(change) {
    cleaning = true;
    try {
      showStorage(await change());
    } finally {
      cleaning = false;
    }
  }
  function changeBodyDays(value) {
    bodyDays = value;
    cleanup(() => setBodyDays(value === 'all' ? null : value));
  }

  let accounts = $state([]);
  let email = $state('');
  let password = $state('');
  let adding = $state(false);
  let addError = $state('');
  let confirmRemove = $state(null);

  const refreshAccounts = () => listAccounts().then((a) => (accounts = a));
  refreshAccounts();

  async function add(e) {
    e.preventDefault();
    adding = true;
    addError = '';
    try {
      await addAccount(email, password);
      email = '';
      password = '';
      await refreshAccounts();
    } catch (err) {
      addError = String(err);
    } finally {
      // Пароль не держим в состоянии дольше необходимого.
      password = '';
      adding = false;
    }
  }

  async function remove(address) {
    confirmRemove = null;
    await removeAccount(address);
    await refreshAccounts();
  }

  function onkeydown(e) {
    if (e.key === 'Escape') onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="window" class:maximized>
  <div bind:this={titlebar}>
    <TitleBar title="Настройки" middleClickMinimize={appearance.middleClickMinimize} {onclose} />
  </div>
  <div class="scroll">
  <div class="content" bind:this={content}>
    <header>
      <nav>
        <button class:active={tab === 'appearance'} onclick={() => (tab = 'appearance')}>Оформление</button>
        <button class:active={tab === 'accounts'} onclick={() => (tab = 'accounts')}>Почтовые ящики</button>
      </nav>
    </header>

    {#if tab === 'appearance'}
      <section>
        <h3>Цветовая гамма</h3>
        <div class="themes">
          {#each THEMES as t (t.id)}
            <button class="theme" class:active={t.id === theme} onclick={() => (theme = t.id)}>
              <span
                class="swatch"
                style:background="linear-gradient(135deg, rgb({t.tint.join(',')}) 0%, {t.accent} 140%)"
              >
                <span class="dot" style:background={t.accent}></span>
              </span>
              {t.name}
            </button>
          {/each}
        </div>

        <h3>Прозрачность окна</h3>
        <div class="slider">
          <span>Больше стекла</span>
          <input type="range" min="0.15" max="0.85" step="0.01" bind:value={opacity} />
          <span>Плотнее</span>
        </div>

        <h3>Масштаб по умолчанию</h3>
        <div class="slider">
          <span>50%</span>
          <input
            type="range"
            min="0.5"
            max="2"
            step="0.05"
            bind:value={zoom}
            
          />
          <span>200%</span>
          <span class="current">{Math.round(zoom * 100)}%</span>
        </div>

        <h3>Окно</h3>
        <label class="toggle">
          <input type="checkbox" bind:checked={middleClickMinimize} />
          Сворачивать окно средним кликом по заголовку
        </label>

        <h3>Значок приложения</h3>
        <div class="field">
          <span>Где показывать</span>
          <div class="poll">
            <Select bind:value={trayMode} label="Где показывать значок" options={TRAY.map(([value, label]) => ({ value, label }))} />
          </div>
        </div>
        <div class="field" class:disabled={trayMode === 'panel'}>
          <span>Вид в трее</span>
          <div class="poll">
            <Select
              bind:value={trayIcon}
              label="Вид значка в трее"
              options={TRAY_ICONS.map(([value, label]) => ({ value, label }))}
            />
          </div>
        </div>
        <label class="toggle" class:disabled={trayMode === 'panel'}>
          <input type="checkbox" bind:checked={closeToTray} disabled={trayMode === 'panel'} />
          Крестик прячет окно в трей (выход — из меню значка)
        </label>
      </section>
    {:else}
      <section>
        <h3>Подключённые ящики</h3>
        {#if accounts.length === 0}
          <p class="muted">Пока нет ни одного ящика — показываются демо-данные.</p>
        {/if}
        <ul class="accounts">
          {#each accounts as address (address)}
            <li>
              <Icon name="inbox" />
              <span class="address">{address}</span>
              {#if confirmRemove === address}
                <span class="muted">Удалить ящик и пароль из KWallet?</span>
                <button class="chip danger" onclick={() => remove(address)}>Удалить</button>
                <button class="chip" onclick={() => (confirmRemove = null)}>Отмена</button>
              {:else}
                <button class="chip" onclick={() => (confirmRemove = address)}>Удалить</button>
              {/if}
            </li>
          {/each}
        </ul>

        <h3>Хранилище на диске</h3>
        <div class="field">
          <span>Хранить содержимое</span>
          <div class="poll">
            <Select
              bind:value={() => bodyDays, changeBodyDays}
              label="Срок хранения содержимого писем"
              options={BODY_DAYS.map(([value, label]) => ({ value, label }))}
            />
          </div>
        </div>
        <div class="storage">
          <span class="muted">
            {#if cleaning}Очистка…{:else if cacheBytes != null}Занято: {megabytes(cacheBytes)}, зашифровано{/if}
          </span>
          {#if bodyDays !== 'all'}
            <button class="chip" disabled={cleaning} onclick={() => cleanup(clearCache)}>Очистить сейчас</button>
          {/if}
        </div>
        <p class="hint">
          Список писем хранится всегда. Содержимое более старых писем стирается с диска и снова скачивается с
          сервера, когда вы их открываете.
        </p>

        <h3>Проверка новых писем</h3>
        <div class="poll">
          <Select bind:value={pollSeconds} label="Интервал проверки" options={POLL.map(([value, label]) => ({ value, label }))} />
        </div>

        <h3>Добавить ящик</h3>
        <form onsubmit={add}>
          <input type="email" placeholder="адрес@yandex.ru или @gmail.com" bind:value={email} required autocomplete="off" spellcheck="false" />
          <input type="password" placeholder="Пароль приложения" bind:value={password} required autocomplete="off" />
          <button class="primary" disabled={adding}>{adding ? 'Проверка входа…' : 'Добавить'}</button>
        </form>
        {#if addError}<div class="error">{addError}</div>{/if}
        <p class="hint">
          Нужен <b>пароль приложения</b>, а не основной пароль. Яндекс: Яндекс ID → Безопасность → Пароли приложений
          (и включите IMAP в настройках почты). Gmail: аккаунт Google → Безопасность → Пароли приложений (нужна
          двухэтапная аутентификация). Пароль проверяется входом на сервер и хранится только в KWallet.
        </p>
      </section>
    {/if}
  </div>
  </div>
</div>

<style>
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
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .content {
    margin: 4px 16px 16px;
    padding: 14px 22px 22px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--glass-border);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 6px;
  }

  nav {
    display: flex;
    gap: 4px;
  }
  nav button {
    height: 34px;
    padding: 0 14px;
    border-radius: 9px;
    color: var(--muted);
  }
  nav button:hover {
    background: rgba(255, 255, 255, 0.07);
  }
  nav button.active {
    background: var(--accent-soft);
    color: var(--text);
  }


  h3 {
    margin: 18px 0 10px;
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
  }

  .themes {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
  }

  .theme {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px;
    border-radius: 11px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid transparent;
    text-align: left;
  }
  .theme:hover {
    background: rgba(255, 255, 255, 0.09);
  }
  .theme.active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .swatch {
    position: relative;
    width: 38px;
    height: 38px;
    flex: none;
    border-radius: 9px;
    border: 1px solid rgba(255, 255, 255, 0.15);
  }
  .dot {
    position: absolute;
    right: 5px;
    bottom: 5px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }

  .slider {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .slider input {
    flex: 1;
    accent-color: var(--accent);
  }

  .current {
    width: 44px;
    text-align: right;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
  }
  .toggle.disabled {
    opacity: 0.45;
    cursor: default;
  }
  .toggle input {
    width: 16px;
    height: 16px;
    accent-color: var(--accent);
  }

  .field {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 10px;
  }
  .field > span {
    width: 150px;
    flex: none;
  }
  .field .poll {
    margin-bottom: 0;
  }
  .field.disabled {
    opacity: 0.45;
    pointer-events: none;
  }
  .storage {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 8px;
  }
  .poll {
    display: flex;
    width: 320px;
    margin-bottom: 10px;
  }

  .accounts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .accounts li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.06);
  }
  .address {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: text;
  }

  form {
    display: flex;
    gap: 8px;
  }
  form input {
    flex: 1;
    min-width: 0;
    height: 36px;
    padding: 0 12px;
    border-radius: 9px;
    border: 1px solid var(--glass-border);
    background: rgba(255, 255, 255, 0.06);
    outline: none;
  }
  form input:focus {
    border-color: var(--accent);
  }

  .primary {
    flex: none;
    height: 36px;
    padding: 0 16px;
    border-radius: 9px;
    background: var(--accent);
    color: #fff;
  }
  .primary:disabled {
    opacity: 0.7;
  }

  .chip {
    flex: none;
    height: 26px;
    padding: 0 12px;
    border-radius: 13px;
    background: rgba(255, 255, 255, 0.12);
    font-size: 12.5px;
  }
  .chip:hover {
    background: rgba(255, 255, 255, 0.22);
  }
  .chip.danger {
    background: rgba(255, 79, 120, 0.35);
  }

  .error {
    margin-top: 10px;
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 79, 120, 0.18);
    color: #ffd0da;
    font-size: 13px;
    user-select: text;
  }

  .hint,
  .muted {
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.5;
  }
  .hint {
    margin: 12px 0 0;
  }
</style>
