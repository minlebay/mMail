<script>
  import { getCurrentWindow } from '@tauri-apps/api/window';

  import Icon from './Icon.svelte';

  let { sync, middleClickMinimize, onsettings } = $props();

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
    <span data-tauri-drag-region>mMail</span>
    {#if sync.message}
      <span class="status {sync.state}" data-tauri-drag-region>{sync.message}</span>
    {/if}
  </div>
  <div class="controls">
    <button class="ctl" aria-label="Настройки" title="Настройки" onclick={onsettings}>
      <Icon name="settings" size={14} />
    </button>
    <button class="ctl" aria-label="Свернуть" onclick={() => win.minimize()}>
      <svg viewBox="0 0 12 12"><path d="M2.5 6h7" /></svg>
    </button>
    <button class="ctl" aria-label="Развернуть" onclick={() => win.toggleMaximize()}>
      <svg viewBox="0 0 12 12"><rect x="2.5" y="2.5" width="7" height="7" rx="1.5" /></svg>
    </button>
    <button class="ctl close" aria-label="Закрыть" onclick={() => win.close()}>
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

  .status {
    font-weight: 400;
    letter-spacing: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status.error {
    color: var(--danger);
  }

  .controls {
    display: flex;
    gap: 8px;
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
