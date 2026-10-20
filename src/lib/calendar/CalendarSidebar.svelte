<script>
  import { t } from '../i18n.js';
  import Icon from '../Icon.svelte';
  import ModeSwitch from '../ModeSwitch.svelte';
  import { fmt } from './locale.js';
  import { monthGrid, addMonths, sameDay } from './dates.js';

  // calendars: [{ id, name, color, primary, writable, visible }]
  let { mode, onmode, date, onpick, calendars, ontoggle, oncreate, connected, footer = null } = $props();

  // Мини-календарь листается сам по себе; выбор дня меняет основной вид.
  let shown = $state(new Date());
  $effect(() => {
    shown = new Date(date.getFullYear(), date.getMonth(), 1);
  });
  const today = new Date();
  const days = $derived(monthGrid(shown));
  const weekdays = $derived(days.slice(0, 7).map((d) => $fmt.weekday(d).slice(0, 2)));
</script>

<nav class="panel sidebar">
  <ModeSwitch {mode} onchange={onmode} />
  <button class="create" disabled={!connected} onclick={oncreate}><Icon name="plus" />{$t('Создать событие')}</button>

  <div class="mini">
    <div class="head">
      <button class="nav" aria-label={$t('Назад')} onclick={() => (shown = addMonths(shown, -1))}><Icon name="chevronLeft" size={14} /></button>
      <span>{$fmt.monthYear(shown)}</span>
      <button class="nav" aria-label={$t('Вперёд')} onclick={() => (shown = addMonths(shown, 1))}><Icon name="chevronRight" size={14} /></button>
    </div>
    <div class="grid">
      {#each weekdays as w}<span class="wd">{w}</span>{/each}
      {#each days as d (d.getTime())}
        <button
          class="day"
          class:other={d.getMonth() !== shown.getMonth()}
          class:today={sameDay(d, today)}
          class:selected={sameDay(d, date)}
          onclick={() => onpick(d)}>{d.getDate()}</button
        >
      {/each}
    </div>
  </div>

  {#if connected}
    <div class="calendars scroll-gutter">
      {#each calendars as c (c.id)}
        <label class="calendar" title={c.name}>
          <input type="checkbox" checked={c.visible} onchange={(e) => ontoggle(c.id, e.currentTarget.checked)} />
          <span class="dot" style:background={c.color}></span>
          <span class="name">{c.name}</span>
        </label>
      {/each}
    </div>
  {/if}
  {@render footer?.()}
</nav>

<style>
  .sidebar {
    padding: var(--panel-inset);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .create {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 38px;
    border-radius: 9px;
    background: var(--accent-soft);
    font-weight: 500;
  }
  .create:hover:not(:disabled) {
    filter: brightness(1.15);
  }
  .create:disabled {
    opacity: 0.45;
  }
  .mini .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 13px;
    font-weight: 600;
    margin-bottom: 6px;
  }
  .nav {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--muted);
  }
  .nav:hover {
    background: rgba(var(--ink), 0.08);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 2px;
    text-align: center;
  }
  .wd {
    font-size: 11px;
    color: var(--muted);
    padding-bottom: 2px;
  }
  .day {
    aspect-ratio: 1;
    border-radius: 50%;
    font-size: 12px;
  }
  .day:hover {
    background: rgba(var(--ink), 0.08);
  }
  .day.other {
    color: var(--muted);
    opacity: 0.6;
  }
  .day.today {
    color: var(--accent);
    font-weight: 700;
  }
  .day.selected {
    background: var(--accent-soft);
    color: var(--text);
  }
  .calendars {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .calendar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 4px;
    border-radius: 8px;
    font-size: 13px;
    cursor: pointer;
  }
  .calendar:hover {
    background: rgba(var(--ink), 0.05);
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex: none;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
