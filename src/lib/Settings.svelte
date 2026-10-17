<script>
  import { untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import { THEMES, applyTheme } from './themes.js';
  import { setAppearance, listAccounts, addAccount, removeAccount } from './api.js';

  let { appearance, onclose } = $props();

  let tab = $state('appearance');
  // Начальные значения формы; дальше форма сама пишет в appearance.
  const initial = untrack(() => $state.snapshot(appearance));
  let theme = $state(initial.theme);
  let opacity = $state(initial.opacity);
  let middleClickMinimize = $state(initial.middleClickMinimize);

  // Тема применяется сразу, в конфиг пишется с небольшой задержкой (ползунок дёргается часто).
  $effect(() => {
    applyTheme(theme, opacity);
    appearance.theme = theme;
    appearance.opacity = opacity;
    appearance.middleClickMinimize = middleClickMinimize;
    const t = setTimeout(() => setAppearance(theme, opacity, middleClickMinimize), 300);
    return () => clearTimeout(t);
  });

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

<div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Настройки">
    <header>
      <nav>
        <button class:active={tab === 'appearance'} onclick={() => (tab = 'appearance')}>Оформление</button>
        <button class:active={tab === 'accounts'} onclick={() => (tab = 'accounts')}>Почтовые ящики</button>
      </nav>
      <button class="close" aria-label="Закрыть" onclick={onclose}>
        <svg viewBox="0 0 12 12"><path d="M3 3l6 6M9 3l-6 6" /></svg>
      </button>
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

        <h3>Окно</h3>
        <label class="toggle">
          <input type="checkbox" bind:checked={middleClickMinimize} />
          Сворачивать окно средним кликом по заголовку
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

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: grid;
    place-items: center;
    background: rgba(10, 8, 18, 0.5);
    border-radius: 12px;
  }

  .dialog {
    width: min(620px, calc(100% - 48px));
    max-height: calc(100% - 48px);
    overflow-y: auto;
    padding: 14px 22px 22px;
    border-radius: var(--radius);
    background: rgba(32, 27, 46, 0.97);
    border: 1px solid var(--glass-border);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.45);
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

  .close {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.1);
  }
  .close:hover {
    background: rgba(255, 255, 255, 0.2);
  }
  .close svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
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

  .toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    cursor: pointer;
  }
  .toggle input {
    width: 16px;
    height: 16px;
    accent-color: var(--accent);
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
