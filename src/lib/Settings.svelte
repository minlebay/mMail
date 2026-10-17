<script>
  import { t, setLanguage, LANGUAGES } from './i18n.js';
  // Настройки — отдельное окно (метка "settings"); размер подстраивается под содержимое.
  import { untrack, onMount } from 'svelte';
  import { getCurrentWindow, currentMonitor, LogicalSize } from '@tauri-apps/api/window';
  import Icon from './Icon.svelte';
  import Select from './Select.svelte';
  import TitleBar from './TitleBar.svelte';
  import { THEMES, applyTheme } from './themes.js';
  import { setAppearance, listAccounts, addAccount, removeAccount, storageInfo, setBodyDays, clearCache, setShowAllAccounts, googleStatus, googleConnect, googleDisconnect, setNotifications } from './api.js';

  let { appearance } = $props();

  const win = getCurrentWindow();
  async function onclose() {
    try {
      await saveAppearance();
      await win.close();
    } catch (error) {
      saveError = String(error);
    }
  }
  let maximized = $state(false);
  let panel;
  let fields;
  let canScroll = $state(false);

  onMount(() => {
    let stopped = false;
    let running = false;
    let pending = false;
    let frame;
    let lastConstraints = '';

    async function fitToContent() {
      const monitor = await currentMonitor();
      const scale = await win.scaleFactor();
      const size = await win.innerSize();
      if (stopped) return;
      const area = monitor?.workArea.size;
      const maxWidth = Math.floor(area ? area.width / monitor.scaleFactor : window.screen.availWidth);
      const maxHeight = Math.floor(area ? area.height / monitor.scaleFactor : window.screen.availHeight);
      const constraints = { minWidth: Math.min(520, maxWidth), minHeight: Math.min(360, maxHeight), maxWidth, maxHeight };
      const key = JSON.stringify(constraints);
      if (key !== lastConstraints) {
        await win.setSizeConstraints(constraints);
        lastConstraints = key;
      }
      if (stopped) return;

      // Измеряем внутренний блок естественной высоты, а не растянутую панель.
      // Соотношение учитывает и масштаб интерфейса, и масштаб экрана.
      const ratio = size.width / scale / window.innerWidth;
      const style = getComputedStyle(panel);
      const paddingY = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      const paddingX = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
      const chromeHeight = window.innerHeight - panel.clientHeight + paddingY;
      const chromeWidth = window.innerWidth - panel.clientWidth + paddingX;
      const neededHeight = Math.ceil((fields.getBoundingClientRect().height + chromeHeight) * ratio);
      const neededWidth = Math.ceil((fields.scrollWidth + chromeWidth) * ratio);
      const width = Math.min(maxWidth, Math.max(constraints.minWidth, size.width / scale, neededWidth));
      // Если высоты экрана мало, сначала даём тексту всю доступную ширину.
      // Следующий resize пересчитает высоту после переноса строк.
      const targetWidth = neededHeight > maxHeight ? maxWidth : width;
      const targetHeight = neededWidth > maxWidth
        ? maxHeight
        : Math.min(maxHeight, Math.max(constraints.minHeight, neededHeight));
      canScroll = size.width / scale >= maxWidth - 1 && size.height / scale >= maxHeight - 1;
      if (Math.abs(targetWidth - size.width / scale) > 1 || Math.abs(targetHeight - size.height / scale) > 1) {
        await win.setSize(new LogicalSize(targetWidth, targetHeight));
      }
    }

    function scheduleFit() {
      if (stopped) return;
      pending = true;
      if (running || frame) return;
      frame = requestAnimationFrame(async () => {
        frame = null;
        pending = false;
        running = true;
        try {
          await fitToContent();
        } catch (error) {
          console.error('Размер окна настроек:', error);
          // При недоступности API сохраняем доступ ко всем настройкам.
          canScroll = true;
        } finally {
          running = false;
          if (pending) scheduleFit();
        }
      });
    }

    const observer = new ResizeObserver(scheduleFit);
    observer.observe(fields);
    window.addEventListener('resize', scheduleFit);
    document.fonts.ready.then(scheduleFit);
    scheduleFit();
    return () => {
      stopped = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener('resize', scheduleFit);
    };
  });

  let tab = $state('appearance');
  // Начальные значения формы; дальше форма сама пишет в appearance.
  const initial = untrack(() => $state.snapshot(appearance));
  let theme = $state(initial.theme);
  let selectedLanguage = $state(initial.language || 'ru');
  let saveError = $state('');
  let opacity = $state(initial.opacity);
  let middleClickMinimize = $state(initial.middleClickMinimize);
  let zoom = $state(initial.zoom);
  let pollSeconds = $state(initial.pollSeconds);
  let trayMode = $state(initial.trayMode);
  let closeToTray = $state(initial.closeToTray);
  // Сохраняется сразу отдельной командой: это дерево папок, а не оформление.
  let showAllAccounts = $state(initial.showAllAccounts ?? true);
  let notifyMail = $state(initial.notifyMail ?? true);
  let notifyEvents = $state(initial.notifyEvents ?? true);
  const saveNotifications = () => setNotifications(notifyMail, notifyEvents);

  // Google Календарь: клиент OAuth вводится один раз; секреты уходят сразу в KWallet через ядро.
  let google = $state(null);
  let clientId = $state('');
  let clientSecret = $state('');
  let connecting = $state(false);
  let googleError = $state('');
  googleStatus().then((s) => (google = s));

  async function connectGoogle(e) {
    e.preventDefault();
    connecting = true;
    googleError = '';
    try {
      const account = await googleConnect(clientId, clientSecret);
      google = { account };
      clientId = '';
    } catch (err) {
      googleError = String(err);
    } finally {
      clientSecret = '';
      connecting = false;
    }
  }

  async function disconnectGoogle() {
    await googleDisconnect();
    google = { account: null };
  }
  let trayIcon = $state(initial.trayIcon);
  const TRAY_ICONS = $derived([
    ['color', $t("Цветной")],
    ['auto', $t("Монохромный, по теме KDE")],
    ['light', $t("Монохромный светлый (тёмная панель)")],
    ['dark', $t("Монохромный тёмный (светлая панель)")],
  ]);
  const TRAY = $derived([
    ['panelTray', $t("В панели задач и в трее")],
    ['tray', $t("Только в трее")],
    ['panel', $t("Только в панели задач")],
  ]);
  const POLL = $derived([
    [5, $t("каждые 5 секунд")],
    [10, $t("каждые 10 секунд")],
    [30, $t("каждые 30 секунд")],
    [60, $t("каждую минуту")],
    [300, $t("каждые 5 минут")],
  ]);

  // Тема применяется сразу, в конфиг пишется с небольшой задержкой (ползунок дёргается часто).
  $effect(() => {
    applyTheme(theme, opacity);
    setLanguage(selectedLanguage);
    appearance.language = selectedLanguage;
    appearance.theme = theme;
    appearance.opacity = opacity;
    appearance.middleClickMinimize = middleClickMinimize;
    appearance.zoom = zoom;
    appearance.pollSeconds = pollSeconds;
    appearance.trayMode = trayMode;
    appearance.closeToTray = closeToTray;
    appearance.trayIcon = trayIcon;
    const timer = setTimeout(() => {
      saveAppearance().catch((error) => { saveError = String(error); });
    }, 300);
    return () => clearTimeout(timer);
  });

  async function saveAppearance() {
    await setAppearance({ language: selectedLanguage, theme, opacity, middleClickMinimize, zoom, pollSeconds, trayMode, trayIcon, closeToTray });
    saveError = '';
  }

  // Select не различает null и «нет значения», поэтому «хранить всё» — строка 'all'.
  const BODY_DAYS = $derived([
    ['all', $t("всех писем")],
    [365, $t("писем за последний год")],
    [180, $t("писем за полгода")],
    [90, $t("писем за 3 месяца")],
    [30, $t("писем за месяц")],
    [0, $t("только открытых, до перезапуска")],
  ]);
  let bodyDays = $state(null);
  let cacheBytes = $state(null);
  let cleaning = $state(false);
  const showStorage = (info) => {
    cacheBytes = info.bytes;
    bodyDays = info.bodyDays ?? 'all';
  };
  storageInfo().then(showStorage);
  const megabytes = (bytes) => `${(bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} ${$t("МБ")}`;

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
  <div class="titlebar-wrap">
    <TitleBar title={$t("Настройки")} middleClickMinimize={appearance.middleClickMinimize} maximizable={false} {onclose} />
  </div>
  <div class="scroll">
  <div class="content panel" class:can-scroll={canScroll} bind:this={panel}>
  <div class="fields" bind:this={fields}>
    {#if saveError}<p class="error">{$t("Не удалось сохранить настройки")}: {$t(saveError)}</p>{/if}
    <header>
      <nav>
        <button class:active={tab === 'appearance'} onclick={() => (tab = 'appearance')}>{$t("Оформление")}</button>
        <button class:active={tab === 'accounts'} onclick={() => (tab = 'accounts')}>{$t("Почтовые ящики")}</button>
      </nav>
    </header>

    {#if tab === 'appearance'}
      <section>
        <h3>{$t('Язык интерфейса')}</h3>
        <Select bind:value={selectedLanguage} label={$t('Язык интерфейса')} options={LANGUAGES} />
        <h3>{$t("Цветовая гамма")}</h3>
        <div class="themes">
          {#each THEMES as themeOption (themeOption.id)}
            <button class="theme" class:active={themeOption.id === theme} onclick={() => (theme = themeOption.id)}>
              <span
                class="swatch"
                style:background="linear-gradient(135deg, rgb({themeOption.tint.join(',')}) 0%, {themeOption.accent} 140%)"
              >
                <span class="dot" style:background={themeOption.accent}></span>
              </span>
              {$t(themeOption.name)}
            </button>
          {/each}
        </div>

        <h3>{$t("Прозрачность окна")}</h3>
        <div class="slider">
          <span>{$t("Больше стекла")}</span>
          <input type="range" min="0.15" max="0.85" step="0.01" bind:value={opacity} />
          <span>{$t("Плотнее")}</span>
        </div>

        <h3>{$t("Масштаб по умолчанию")}</h3>
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

        <h3>{$t("Окно")}</h3>
        <label class="toggle">
          <input type="checkbox" bind:checked={middleClickMinimize} />
          {$t("Сворачивать окно средним кликом по заголовку")}
        </label>

        <h3>{$t("Уведомления")}</h3>
        <label class="toggle">
          <input type="checkbox" bind:checked={notifyMail} onchange={saveNotifications} />
          {$t("О новых письмах во «Входящих»")}
        </label>
        <label class="toggle">
          <input type="checkbox" bind:checked={notifyEvents} onchange={saveNotifications} />
          {$t("Напоминания о событиях календаря")}
        </label>

        <h3>{$t("Значок приложения")}</h3>
        <div class="field">
          <span>{$t("Где показывать")}</span>
          <div class="poll">
            <Select bind:value={trayMode} label={$t("Где показывать значок")} options={TRAY.map(([value, label]) => ({ value, label }))} />
          </div>
        </div>
        <div class="field" class:disabled={trayMode === 'panel'}>
          <span>{$t("Вид в трее")}</span>
          <div class="poll">
            <Select
              bind:value={trayIcon}
              label={$t("Вид значка в трее")}
              options={TRAY_ICONS.map(([value, label]) => ({ value, label }))}
            />
          </div>
        </div>
        <label class="toggle" class:disabled={trayMode === 'panel'}>
          <input type="checkbox" bind:checked={closeToTray} disabled={trayMode === 'panel'} />
          {$t("Крестик прячет окно в трей (выход — из меню значка)")}
        </label>
      </section>
    {:else}
      <section>
        <h3>{$t("Подключённые ящики")}</h3>
        {#if accounts.length === 0}
          <p class="muted">{$t("Пока нет ни одного ящика — показываются демо-данные.")}</p>
        {/if}
        <ul class="accounts">
          {#each accounts as address (address)}
            <li>
              <Icon name="inbox" />
              <span class="address">{address}</span>
              {#if confirmRemove === address}
                <span class="muted">{$t("Удалить ящик и пароль из KWallet?")}</span>
                <button class="chip danger" onclick={() => remove(address)}>{$t("Удалить")}</button>
                <button class="chip" onclick={() => (confirmRemove = null)}>{$t("Отмена")}</button>
              {:else}
                <button class="chip" onclick={() => (confirmRemove = address)}>{$t("Удалить")}</button>
              {/if}
            </li>
          {/each}
        </ul>
        <label class="toggle">
          <input type="checkbox" bind:checked={() => showAllAccounts, (v) => setShowAllAccounts((showAllAccounts = v))} />
          {$t("Показывать раздел «Все ящики»")}
        </label>

        <h3>{$t("Google Календарь")}</h3>
        {#if google?.account}
          <div class="storage">
            <span>{$t("Подключён: {account}", { account: google.account })}</span>
            <button class="chip" onclick={disconnectGoogle}>{$t("Отключить")}</button>
          </div>
        {:else if google}
          <ol class="steps">
            <li>{$t("Откройте console.cloud.google.com и создайте проект.")}</li>
            <li>{$t("APIs & Services → Library: включите Google Calendar API.")}</li>
            <li>{$t("Google Auth Platform → Audience: тип External, затем «Publish app» (в режиме Testing доступ истекает через 7 дней).")}</li>
            <li>{$t("Clients → Create client → Desktop app. Скопируйте Client ID и Client secret сюда.")}</li>
          </ol>
          <form onsubmit={connectGoogle} class="google">
            <input placeholder="Client ID" bind:value={clientId} required autocomplete="off" spellcheck="false" />
            <input type="password" placeholder="Client secret" bind:value={clientSecret} required autocomplete="off" />
            <button class="primary" disabled={connecting}>{connecting ? $t("Ждём вход в браузере…") : $t("Подключить")}</button>
          </form>
          {#if googleError}<div class="error">{googleError}</div>{/if}
          <p class="hint">
            {$t("Google покажет предупреждение «приложение не проверено» — это ваше собственное приложение, нажмите «Продолжить». Доступ можно отозвать здесь или в настройках аккаунта Google.")}
          </p>
        {/if}

        <h3>{$t("Хранилище на диске")}</h3>
        <div class="field">
          <span>{$t("Хранить содержимое")}</span>
          <div class="poll">
            <Select
              bind:value={() => bodyDays, changeBodyDays}
              label={$t("Срок хранения содержимого писем")}
              options={BODY_DAYS.map(([value, label]) => ({ value, label }))}
            />
          </div>
        </div>
        <div class="storage">
          <span class="muted">
            {#if cleaning}{$t("Очистка…")}{:else if cacheBytes != null}{$t("Занято:")} {megabytes(cacheBytes)}{$t(", зашифровано")}{/if}
          </span>
          {#if bodyDays !== 'all'}
            <button class="chip" disabled={cleaning} onclick={() => cleanup(clearCache)}>{$t("Очистить сейчас")}</button>
          {/if}
        </div>
        <p class="hint">
          {$t("Список писем хранится всегда. Содержимое более старых писем стирается с диска и снова скачивается с сервера, когда вы их открываете.")}
        </p>

        <h3>{$t("Проверка новых писем")}</h3>
        <div class="poll">
          <Select bind:value={pollSeconds} label={$t("Интервал проверки")} options={POLL.map(([value, label]) => ({ value, label }))} />
        </div>

        <h3>{$t("Добавить ящик")}</h3>
        <form onsubmit={add}>
          <input type="email" placeholder={$t("адрес@yandex.ru или @gmail.com")} bind:value={email} required autocomplete="off" spellcheck="false" />
          <input type="password" placeholder={$t("Пароль приложения")} bind:value={password} required autocomplete="off" />
          <button class="primary" disabled={adding}>{adding ? $t("Проверка входа…") : $t("Добавить")}</button>
        </form>
        {#if addError}<div class="error">{$t(addError)}</div>{/if}
        <p class="hint">
          {$t("Нужен пароль приложения, а не основной пароль. Яндекс: Яндекс ID → Безопасность → Пароли приложений (и включите IMAP в настройках почты). Gmail: аккаунт Google → Безопасность → Пароли приложений (нужна двухэтапная аутентификация). Пароль проверяется входом на сервер и хранится только в KWallet.")}
        </p>
      </section>
    {/if}
  </div>
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
  .titlebar-wrap {
    flex: none;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    display: flex;
    padding: 4px 16px 16px;
    overflow: hidden;
  }
  .content {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    padding: 14px 22px 22px;
  }

  .content.can-scroll {
    overflow: auto;
  }
  .fields {
    display: flow-root;
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
    grid-template-columns: repeat(auto-fit, minmax(min(145px, 100%), 1fr));
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
  form.google {
    flex-wrap: wrap;
  }
  form.google input {
    flex: 1 1 220px;
  }
  .steps {
    margin: 0 0 10px;
    padding-left: 20px;
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.6;
    user-select: text;
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
