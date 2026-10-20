<script>
  import { t } from './i18n.js';
  import Icon from './Icon.svelte';

  // Низ боковой панели (почта и календарь): проверить почту, масштаб интерфейса, настройки.
  let { zoom, onzoom, oncheck, onsettings } = $props();

  let zoomOpen = $state(false);
  let root = $state();
  // Крутится, пока ядро проверяет ящики; не меньше оборота, чтобы клик был заметен.
  let checking = $state(false);
  async function check() {
    if (checking) return;
    checking = true;
    try {
      await Promise.all([oncheck(), new Promise((r) => setTimeout(r, 700))]);
    } finally {
      checking = false;
    }
  }
  const percent = (z) => `${Math.round(z * 100)}%`;

  // Панель обрезает всё, что за её краями, поэтому подложку на всё окно не ставим —
  // закрываем по нажатию вне блока.
  function outside(event) {
    if (zoomOpen && !root?.contains(event.target)) zoomOpen = false;
  }
</script>

<svelte:window onpointerdown={outside} />

<div class="controls" bind:this={root}>
  {#if zoomOpen}
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
    </div>
  {/if}
  <button class="ctl" class:spinning={checking} aria-label={$t("Проверить почту")} title={$t("Проверить почту")} disabled={checking} onclick={check}>
    <Icon name="refresh" size={14} />
  </button>
  <button class="pill" class:active={zoomOpen} title={$t("Масштаб (Ctrl +/−/0)")} onclick={() => (zoomOpen = !zoomOpen)}>{percent(zoom)}</button>
  <button class="ctl" aria-label={$t("Настройки")} title={$t("Настройки")} onclick={onsettings}>
    <Icon name="settings" size={14} />
  </button>
</div>

<style>
  .controls {
    position: relative;
    flex: none;
    margin-top: auto;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .ctl {
    flex: none;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: rgba(var(--ink), 0.1);
  }
  .ctl:hover,
  .pill:hover,
  .pill.active {
    background: rgba(var(--ink), 0.22);
  }
  .ctl.spinning :global(svg) {
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .pill {
    height: 28px;
    padding: 0 10px;
    border-radius: 14px;
    background: rgba(var(--ink), 0.1);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  /* Над кнопками и в ширину панели: шире она не может — панель обрезает края. */
  .popover {
    position: absolute;
    bottom: 36px;
    left: 0;
    right: 0;
    z-index: 21;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: 12px;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
  }
  .popover input {
    flex: 1;
    min-width: 0;
    accent-color: var(--accent);
  }
  .step {
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: rgba(var(--ink), 0.1);
    font-size: 15px;
    line-height: 1;
  }
  .step:hover {
    background: rgba(var(--ink), 0.22);
  }
</style>
