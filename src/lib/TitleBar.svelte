<script>
  import { t, statusText } from './i18n.js';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  import Icon from './Icon.svelte';
  import { unlockStore } from './api.js';

  // Шапка общая для главного окна и окон письма: лишнее (статус, масштаб, настройки) необязательно.
  let { title = 'mMail', sync = null, middleClickMinimize = true, zoom = null, onzoom, onsettings = null, onclose = null, maximizable = true } = $props();

  let zoomOpen = $state(false);
  const percent = (z) => `${Math.round(z * 100)}%`;

  const win = getCurrentWindow();

  // Средний клик по свободному месту шапки сворачивает окно (настраивается).
  function onmousedown(e) {
    if (e.button !== 1 || !middleClickMinimize || e.target.closest('button')) return;
    e.preventDefault();
    win.minimize();
  }
</script>

<!-- Своя шапка вместо рамки KDE: тянется за пустое место, двойной клик разворачивает. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="titlebar" data-tauri-drag-region {onmousedown}>
  <div class="title" data-tauri-drag-region>
    <span class="name" data-tauri-drag-region>{title}</span>
    {#if sync?.message}
      <span class="status {sync.state}" data-tauri-drag-region>{statusText($t, sync.message)}</span>
    {/if}
    {#if sync?.state === 'locked'}
      <button class="pill" title={$t("Снова запросить ключ у KWallet")} onclick={unlockStore}>{$t("Открыть хранилище")}</button>
    {/if}
  </div>
  <div class="controls">
    {#if zoom != null}
    <div class="zoom">
      <button class="pill" title={$t("Масштаб (Ctrl +/−/0)")} onclick={() => (zoomOpen = !zoomOpen)}>{percent(zoom)}</button>
      {#if zoomOpen}
        <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
        <div class="backdrop" onclick={() => (zoomOpen = false)}></div>
        <div class="popover">
          <button class="step" aria-label={$t("Уменьшить")} onclick={() => onzoom(zoom - 0.1)}>−</button>
          <input
            type="range"
            min="0.5"
            max="2"
            step="0.05"
            value={zoom}
            oninput={(e) => onzoom(Number(e.currentTarget.value))}
            aria-label={$t("Масштаб")}
          />
          <button class="step" aria-label={$t("Увеличить")} onclick={() => onzoom(zoom + 0.1)}>+</button>
          <span class="value">{percent(zoom)}</span>
        </div>
      {/if}
    </div>
    {/if}
    {#if onsettings}
      <button class="ctl" aria-label={$t("Настройки")} title={$t("Настройки")} onclick={onsettings}>
        <Icon name="settings" size={14} />
      </button>
    {/if}
    <button class="ctl" aria-label={$t("Свернуть")} onclick={() => win.minimize()}>
      <svg viewBox="0 0 12 12"><path d="M2.5 6h7" /></svg>
    </button>
    {#if maximizable}
    <button class="ctl" aria-label={$t("Развернуть")} onclick={() => win.toggleMaximize()}>
      <svg viewBox="0 0 12 12"><rect x="2.5" y="2.5" width="7" height="7" rx="1.5" /></svg>
    </button>
    {/if}
    <button class="ctl close" aria-label={$t("Закрыть")} onclick={() => (onclose ? onclose() : win.close())}>
      <svg viewBox="0 0 12 12"><path d="M3 3l6 6M9 3l-6 6" /></svg>
    </button>
  </div>
</header>

<style>
  .titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 36px;
    padding: 0 6px 0 18px;
  }

  .title {
    display: flex;
    align-items: baseline;
    gap: 14px;
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--muted);
  }

  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status {
    font-weight: 400;
    letter-spacing: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status.error,
  .status.locked {
    color: var(--danger);
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .zoom {
    position: relative;
  }
  .pill {
    height: 26px;
    padding: 0 10px;
    border-radius: 13px;
    background: rgba(255, 255, 255, 0.1);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .pill:hover {
    background: rgba(255, 255, 255, 0.22);
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 20;
  }
  .popover {
    position: absolute;
    top: 32px;
    right: 0;
    z-index: 21;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 12px;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
  }
  .popover input {
    width: 160px;
    accent-color: var(--accent);
  }
  .step {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.1);
    font-size: 15px;
    line-height: 1;
  }
  .step:hover {
    background: rgba(255, 255, 255, 0.22);
  }
  .value {
    width: 40px;
    text-align: right;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--muted);
  }

  .ctl {
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.1);
  }
  .ctl:hover {
    background: rgba(255, 255, 255, 0.22);
  }
  .ctl.close:hover {
    background: #ff4f78;
  }

  svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
  }
</style>
