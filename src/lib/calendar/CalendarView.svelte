<script>
  import { t } from '../i18n.js';
  import Icon from '../Icon.svelte';
  import { fmt } from './locale.js';
  import { monthGrid, addDays, addMonths, sameDay, startOfWeek, startOfDay, onDay, layoutDay, eventBounds } from './dates.js';

  // events: [{ id, calendarId, title, start, end, allDay, ... }]; colors: { calendarId: '#rrggbb' }
  let { date, ondate, view, onview, events, colors, onopen, oncreate, loading, error, connected, onconnect } = $props();

  const HOUR = 48;
  let now = $state(new Date());
  $effect(() => {
    const timer = setInterval(() => (now = new Date()), 60_000);
    return () => clearInterval(timer);
  });

  const title = $derived(
    view === 'month'
      ? $fmt.monthYear(date)
      : view === 'week'
        ? `${$fmt.dayMonth(startOfWeek(date))} — ${$fmt.dayMonth(addDays(startOfWeek(date), 6))}`
        : $fmt.full(date),
  );
  const columns = $derived(view === 'week' ? Array.from({ length: 7 }, (_, i) => addDays(startOfWeek(date), i)) : [startOfDay(date)]);
  const color = (e) => colors[e.calendarId] ?? 'var(--accent)';

  function step(direction) {
    if (view === 'month') ondate(addMonths(date, direction));
    else ondate(addDays(date, direction * (view === 'week' ? 7 : 1)));
  }

  const dayEvents = (day) =>
    events.filter((e) => onDay(e, day)).sort((a, b) => Number(b.allDay) - Number(a.allDay) || eventBounds(a)[0] - eventBounds(b)[0]);

  // Двойной клик по пустому месту сетки — новое событие в это время (шаг полчаса).
  function createAt(event, day) {
    const rect = event.currentTarget.getBoundingClientRect();
    const minutes = Math.floor(((event.clientY - rect.top) / HOUR) * 2) * 30;
    const start = new Date(day.getFullYear(), day.getMonth(), day.getDate(), 0, Math.max(0, Math.min(minutes, 23 * 60 + 30)));
    oncreate(start, false);
  }

  // Неделя/день открываются прокрученными к рабочему утру.
  let scroller = $state();
  $effect(() => {
    if (scroller && view !== 'month') scroller.scrollTop = HOUR * 7.5;
  });
</script>

<section class="panel calendar">
  <header class="toolbar">
    <button class="chip" onclick={() => ondate(new Date())}>{$t('Сегодня')}</button>
    <button class="nav" aria-label={$t('Назад')} onclick={() => step(-1)}><Icon name="chevronLeft" size={16} /></button>
    <button class="nav" aria-label={$t('Вперёд')} onclick={() => step(1)}><Icon name="chevronRight" size={16} /></button>
    <h2>{title}</h2>
    {#if loading}<span class="muted">{$t('Загрузка…')}</span>{/if}
    <div class="views" role="tablist">
      {#each [['month', 'Месяц'], ['week', 'Неделя'], ['day', 'День']] as [id, label]}
        <button role="tab" aria-selected={view === id} class:active={view === id} onclick={() => onview(id)}>{$t(label)}</button>
      {/each}
    </div>
  </header>

  {#if !connected}
    <div class="empty">
      <Icon name="calendar" size={32} />
      <p>{$t('Подключите Google Календарь в настройках, на вкладке «Почтовые ящики».')}</p>
      <button class="chip" onclick={onconnect}>{$t('Открыть настройки')}</button>
    </div>
  {:else}
    {#if error}<div class="error">{error}</div>{/if}

    {#if view === 'month'}
      <div class="month">
        {#each monthGrid(date).slice(0, 7) as d}<div class="wd">{$fmt.weekday(d)}</div>{/each}
        {#each monthGrid(date) as d (d.getTime())}
          {@const list = dayEvents(d)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="cell"
            class:other={d.getMonth() !== date.getMonth()}
            ondblclick={(e) => e.target === e.currentTarget && oncreate(new Date(d.getFullYear(), d.getMonth(), d.getDate(), 9), false)}
          >
            <button class="num" class:today={sameDay(d, now)} onclick={() => { ondate(d); onview('day'); }}>{d.getDate()}</button>
            {#each list.slice(0, 3) as e (e.id + e.calendarId)}
              <button class="chipev" class:allday={e.allDay} style:--c={color(e)} onclick={() => onopen(e)} title={e.title}>
                {#if !e.allDay}<span class="time">{$fmt.time(eventBounds(e)[0])}</span>{/if}{e.title}
              </button>
            {/each}
            {#if list.length > 3}
              <button class="more" onclick={() => { ondate(d); onview('day'); }}>{$t('ещё {count}', { count: list.length - 3 })}</button>
            {/if}
          </div>
        {/each}
      </div>
    {:else}
      <div class="days" style:--cols={columns.length}>
        <div class="corner"></div>
        {#each columns as d (d.getTime())}
          <button class="dayhead" class:today={sameDay(d, now)} onclick={() => { ondate(d); onview('day'); }}>
            <span>{$fmt.weekday(d)}</span><b>{d.getDate()}</b>
          </button>
        {/each}
        <div class="corner small">{$t('весь день')}</div>
        {#each columns as d (d.getTime())}
          <div class="alldays">
            {#each dayEvents(d).filter((e) => e.allDay) as e (e.id + e.calendarId)}
              <button class="chipev allday" style:--c={color(e)} onclick={() => onopen(e)} title={e.title}>{e.title}</button>
            {/each}
          </div>
        {/each}
      </div>
      <div class="scroll" bind:this={scroller}>
        <div class="timegrid" style:--cols={columns.length} style:height="{HOUR * 24}px">
          <div class="hours">
            {#each Array.from({ length: 24 }, (_, h) => h) as h}
              <span style:top="{h * HOUR}px">{h ? `${String(h).padStart(2, '0')}:00` : ''}</span>
            {/each}
          </div>
          {#each columns as d (d.getTime())}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div class="col" ondblclick={(e) => e.target === e.currentTarget && createAt(e, d)}>
              {#each layoutDay(dayEvents(d).filter((e) => !e.allDay), d) as { event: e, pos, column, columns: n } (e.id + e.calendarId)}
                {@const height = Math.max(pos[1] * HOUR * 24 - 2, 18)}
                <button
                  class="block"
                  class:short={height < 38}
                  style:--c={color(e)}
                  style:top="{pos[0] * HOUR * 24}px"
                  style:height="{height}px"
                  style:left="{(column / n) * 100}%"
                  style:width="{100 / n}%"
                  onclick={() => onopen(e)}
                  title={e.title}
                >
                  <b>{e.title}</b>
                  <span>{$fmt.time(eventBounds(e)[0])} – {$fmt.time(eventBounds(e)[1])}</span>
                </button>
              {/each}
              {#if sameDay(d, now)}
                <div class="now" style:top="{((now - startOfDay(now)) / 3_600_000) * HOUR}px"></div>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</section>

<style>
  .calendar {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--glass-border);
  }
  h2 {
    margin: 0 8px;
    font-size: 17px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .muted {
    color: var(--muted);
    font-size: 12.5px;
  }
  .chip {
    height: 30px;
    padding: 0 12px;
    border-radius: 8px;
    background: rgba(var(--ink), 0.08);
  }
  .chip:hover {
    background: rgba(var(--ink), 0.14);
  }
  .nav {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: 8px;
  }
  .nav:hover {
    background: rgba(var(--ink), 0.08);
  }
  .views {
    margin-left: auto;
    display: flex;
    gap: 2px;
    padding: 3px;
    border-radius: 9px;
    background: rgba(var(--ink), 0.05);
  }
  .views button {
    height: 26px;
    padding: 0 12px;
    border-radius: 7px;
    color: var(--muted);
    font-size: 13px;
  }
  .views button.active {
    background: var(--accent-soft);
    color: var(--text);
  }
  .empty {
    flex: 1;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 10px;
    color: var(--muted);
    text-align: center;
  }
  .error {
    margin: 10px 14px 0;
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 79, 120, 0.18);
    color: var(--danger-text);
    font-size: 13px;
  }

  /* Месяц */
  .month {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    grid-template-rows: auto repeat(6, minmax(0, 1fr));
  }
  .month .wd {
    padding: 6px 8px;
    font-size: 12px;
    color: var(--muted);
  }
  .cell {
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 4px;
    border-top: 1px solid var(--glass-border);
    border-left: 1px solid var(--glass-border);
  }
  .cell:nth-child(7n + 1) {
    border-left: 0;
  }
  .cell.other {
    background: rgba(0, 0, 0, 0.08);
  }
  .cell.other .num {
    color: var(--muted);
  }
  .num {
    align-self: flex-start;
    min-width: 24px;
    height: 24px;
    padding: 0 6px;
    border-radius: 12px;
    font-size: 12.5px;
  }
  .num:hover {
    background: rgba(var(--ink), 0.08);
  }
  .num.today {
    background: var(--accent-strong);
    color: #fff;
  }
  .chipev {
    display: block;
    width: 100%;
    padding: 2px 6px;
    border-radius: 5px;
    font-size: 12px;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-left: 3px solid var(--c);
    background: color-mix(in srgb, var(--c) 14%, transparent);
  }
  .chipev.allday {
    border-left: 0;
    background: color-mix(in srgb, var(--c) 55%, transparent);
  }
  .chipev:hover {
    filter: brightness(1.2);
  }
  .time {
    color: var(--muted);
    margin-right: 5px;
  }
  .more {
    font-size: 11.5px;
    color: var(--muted);
    text-align: left;
    padding: 0 6px;
  }

  /* Неделя и день */
  .days {
    display: grid;
    grid-template-columns: 56px repeat(var(--cols), minmax(0, 1fr));
    border-bottom: 1px solid var(--glass-border);
    padding-right: var(--scrollbar-size, 12px);
  }
  .corner.small {
    font-size: 11px;
    color: var(--muted);
    padding: 4px 6px;
    text-align: right;
  }
  .dayhead {
    display: flex;
    align-items: baseline;
    justify-content: center;
    gap: 6px;
    padding: 6px 0;
    font-size: 12.5px;
    color: var(--muted);
  }
  .dayhead b {
    font-size: 18px;
    color: var(--text);
    font-weight: 500;
  }
  .dayhead.today b {
    color: var(--accent);
  }
  .alldays {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 2px;
    min-height: 26px;
    border-left: 1px solid var(--glass-border);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: scroll;
  }
  .timegrid {
    position: relative;
    display: grid;
    grid-template-columns: 56px repeat(var(--cols), minmax(0, 1fr));
    background-image: repeating-linear-gradient(to bottom, var(--glass-border) 0 1px, transparent 1px 48px);
  }
  .hours {
    position: relative;
  }
  .hours span {
    position: absolute;
    right: 8px;
    transform: translateY(-50%);
    font-size: 11px;
    color: var(--muted);
  }
  .col {
    position: relative;
    border-left: 1px solid var(--glass-border);
  }
  .block {
    position: absolute;
    /* Кнопка центрирует содержимое по вертикали — у короткого события название уезжало за край. */
    justify-content: flex-start;
    padding: 3px 6px;
    border-radius: 6px;
    overflow: hidden;
    text-align: left;
    font-size: 12px;
    line-height: 1.3;
    display: flex;
    flex-direction: column;
    border-left: 3px solid var(--c);
    background: color-mix(in srgb, var(--c) 30%, rgba(20, 18, 30, 0.6));
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.15);
  }
  .block:hover {
    filter: brightness(1.15);
    z-index: 2;
  }
  .block b {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .block span {
    color: var(--muted);
    font-size: 11px;
  }
  /* Короткое событие (меньше ~45 минут): одна строка «Название · 14:45». */
  .block.short {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    padding-top: 0;
    padding-bottom: 0;
  }
  .block.short b {
    flex: 0 1 auto;
    min-width: 0;
  }
  .block.short span {
    flex: none;
  }
  .now {
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--danger);
    z-index: 3;
    pointer-events: none;
  }
</style>
