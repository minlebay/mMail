<script>
  import { t } from './i18n.js';
  import Icon from './Icon.svelte';

  // Почта / Календарь — режим главного окна.
  let { mode, onchange } = $props();
</script>

<div class="switch" role="tablist">
  <button role="tab" aria-selected={mode === 'mail'} class:active={mode === 'mail'} onclick={() => onchange('mail')}>
    <Icon name="inbox" size={15} />{$t('Почта')}
  </button>
  <button role="tab" aria-selected={mode === 'calendar'} class:active={mode === 'calendar'} onclick={() => onchange('calendar')}>
    <Icon name="calendar" size={15} />{$t('Календарь')}
  </button>
</div>

<style>
  .switch {
    flex: none;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    padding: 3px;
    border-radius: 10px;
    background: rgba(var(--ink), 0.05);
  }
  button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 30px;
    border-radius: 8px;
    color: var(--muted);
    font-size: 13px;
  }
  button:hover {
    background: rgba(var(--ink), 0.06);
  }
  button.active {
    background: var(--accent-soft);
    color: var(--text);
  }
</style>
