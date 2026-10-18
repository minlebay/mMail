<script>
  import { untrack } from 'svelte';
  import { t } from '../i18n.js';
  import Icon from '../Icon.svelte';
  import Select from '../Select.svelte';
  import { fmt } from './locale.js';
  import { eventBounds, toDateKey, toRfc3339, addDays, fromDateKey } from './dates.js';

  // event — существующее событие (просмотр) или null (новое: preset { start: Date, allDay }).
  let { event = null, preset = null, calendars, onclose, onsave, ondelete } = $props();

  const writable = $derived(calendars.filter((c) => c.writable));
  const calendarOf = (id) => calendars.find((c) => c.id === id);
  const canEdit = $derived(!event || !!calendarOf(event.calendarId)?.writable);

  let editing = $state(untrack(() => !event));
  let busy = $state(false);
  let error = $state('');
  let confirmDelete = $state(false);

  // Поля формы: даты и время по отдельности (так их показывают поля ввода).
  const pad = (n) => String(n).padStart(2, '0');
  const timeOf = (d) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  function initial() {
    if (event) {
      const [s, e] = eventBounds(event);
      // У событий на весь день конец в Google не включается — в форме показываем последний день.
      const endDay = event.allDay ? addDays(e, -1) : e;
      return {
        calendarId: event.calendarId,
        title: event.title,
        allDay: event.allDay,
        startDate: toDateKey(s),
        startTime: timeOf(s),
        endDate: toDateKey(endDay),
        endTime: timeOf(e),
        location: event.location,
        description: event.description,
      };
    }
    const s = preset?.start ?? new Date();
    const e = new Date(s.getTime() + 60 * 60 * 1000);
    return {
      calendarId: (calendars.find((c) => c.primary && c.writable) ?? calendars.find((c) => c.writable))?.id ?? '',
      title: '',
      allDay: !!preset?.allDay,
      startDate: toDateKey(s),
      startTime: timeOf(s),
      endDate: toDateKey(e),
      endTime: timeOf(e),
      location: '',
      description: '',
    };
  }
  let form = $state(untrack(initial));

  function draft() {
    if (form.allDay) {
      return { title: form.title, allDay: true, start: form.startDate, end: toDateKey(addDays(fromDateKey(form.endDate), 1)), location: form.location, description: form.description };
    }
    return {
      title: form.title,
      allDay: false,
      start: toRfc3339(new Date(`${form.startDate}T${form.startTime}`)),
      end: toRfc3339(new Date(`${form.endDate}T${form.endTime}`)),
      location: form.location,
      description: form.description,
    };
  }

  async function save(e) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      await onsave(form.calendarId, event?.id ?? null, draft());
      onclose();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    busy = true;
    try {
      await ondelete(event.calendarId, event.id);
      onclose();
    } catch (err) {
      error = String(err);
      busy = false;
    }
  }

  function when(e) {
    const [s, end] = eventBounds(e);
    if (e.allDay) {
      const last = addDays(end, -1);
      return toDateKey(s) === toDateKey(last) ? $fmt.full(s) : `${$fmt.full(s)} — ${$fmt.full(last)}`;
    }
    return toDateKey(s) === toDateKey(end) ? `${$fmt.full(s)}, ${$fmt.time(s)} – ${$fmt.time(end)}` : `${$fmt.full(s)} ${$fmt.time(s)} — ${$fmt.full(end)} ${$fmt.time(end)}`;
  }

  function onkeydown(e) {
    if (e.key === 'Escape') onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={$t('Событие')}>
    {#if !editing && event}
      <div class="title">
        <span class="dot" style:background={calendarOf(event.calendarId)?.color}></span>
        <h2>{event.title}</h2>
      </div>
      <div class="row"><Icon name="clock" size={15} /><span>{when(event)}</span></div>
      {#if event.recurring}<div class="row muted">{$t('Повторяющееся событие: изменения коснутся только этого повторения')}</div>{/if}
      {#if event.location}<div class="row"><Icon name="mapPin" size={15} /><span class="selectable">{event.location}</span></div>{/if}
      <div class="row muted"><Icon name="calendar" size={15} /><span>{calendarOf(event.calendarId)?.name ?? ''}</span></div>
      <!-- Описание — чужой текст (приглашения извне): только текст, без разметки и ссылок. -->
      {#if event.description}<div class="description selectable">{event.description}</div>{/if}
      {#if error}<div class="error">{error}</div>{/if}
      <div class="buttons">
        {#if canEdit}
          {#if confirmDelete}
            <span class="muted">{$t('Удалить событие?')}</span>
            <button class="danger" disabled={busy} onclick={remove}>{$t('Удалить')}</button>
            <button class="secondary" onclick={() => (confirmDelete = false)}>{$t('Отмена')}</button>
          {:else}
            <button class="secondary danger-text" onclick={() => (confirmDelete = true)}>{$t('Удалить')}</button>
            <button class="secondary" onclick={() => (editing = true)}>{$t('Изменить')}</button>
          {/if}
        {/if}
        <button class="primary" onclick={onclose}>{$t('Закрыть')}</button>
      </div>
    {:else}
      <form onsubmit={save}>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="titleinput" placeholder={$t('Название')} bind:value={form.title} autofocus maxlength="1000" />
        <div class="grid">
          <span>{$t('Календарь')}</span>
          <Select bind:value={form.calendarId} label={$t('Календарь')} options={writable.map((c) => ({ value: c.id, label: c.name }))} />
          <span></span>
          <label class="check"><input type="checkbox" bind:checked={form.allDay} />{$t('Весь день')}</label>
          <span>{$t('Начало')}</span>
          <div class="when">
            <input type="date" bind:value={form.startDate} required />
            {#if !form.allDay}<input type="time" bind:value={form.startTime} required />{/if}
          </div>
          <span>{$t('Конец')}</span>
          <div class="when">
            <input type="date" bind:value={form.endDate} required />
            {#if !form.allDay}<input type="time" bind:value={form.endTime} required />{/if}
          </div>
          <span>{$t('Место')}</span>
          <input bind:value={form.location} maxlength="1000" />
        </div>
        <textarea placeholder={$t('Описание')} bind:value={form.description} rows="4"></textarea>
        {#if error}<div class="error">{error}</div>{/if}
        <div class="buttons">
          <button type="button" class="secondary" onclick={() => (event ? (editing = false) : onclose())}>{$t('Отмена')}</button>
          <button class="primary" disabled={busy || !form.calendarId}>{busy ? $t('Сохраняю…') : $t('Сохранить')}</button>
        </div>
      </form>
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
    background: rgba(10, 8, 18, 0.55);
    border-radius: 12px;
  }
  .dialog {
    width: min(520px, calc(100% - 48px));
    max-height: calc(100% - 48px);
    overflow-y: auto;
    padding: 20px 22px 16px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--glass-border);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.45);
  }
  .title {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  .title h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 500;
    overflow-wrap: anywhere;
  }
  .dot {
    width: 12px;
    height: 12px;
    border-radius: 4px;
    flex: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 6px 0;
    font-size: 13.5px;
  }
  .muted {
    color: var(--muted);
  }
  .selectable {
    user-select: text;
  }
  .description {
    margin-top: 12px;
    padding: 10px 12px;
    border-radius: 9px;
    background: rgba(var(--ink), 0.05);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 240px;
    overflow-y: auto;
    font-size: 13px;
    line-height: 1.5;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  input,
  textarea {
    height: 34px;
    padding: 0 10px;
    border-radius: 9px;
    border: 1px solid var(--glass-border);
    background: rgba(var(--ink), 0.06);
    color: inherit;
    font: inherit;
  }
  textarea {
    height: auto;
    padding: 8px 10px;
    resize: vertical;
  }
  input:focus,
  textarea:focus {
    outline: none;
    border-color: var(--accent);
  }
  .titleinput {
    height: 40px;
    font-size: 16px;
  }
  .grid {
    display: grid;
    grid-template-columns: 90px 1fr;
    align-items: center;
    gap: 8px 10px;
    font-size: 13px;
  }
  .grid > span {
    color: var(--muted);
  }
  .when {
    display: flex;
    gap: 8px;
  }
  .when input[type='date'] {
    flex: 1;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .error {
    margin-top: 8px;
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 79, 120, 0.18);
    color: var(--danger-text);
    font-size: 13px;
  }
  .buttons {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  button {
    height: 34px;
    padding: 0 14px;
    border-radius: 9px;
  }
  .secondary {
    background: rgba(var(--ink), 0.1);
  }
  .secondary:hover {
    background: rgba(var(--ink), 0.18);
  }
  .danger-text {
    color: var(--danger);
  }
  .danger {
    background: var(--danger);
    color: #fff;
  }
  .primary {
    background: var(--accent-strong);
    color: #fff;
  }
  .primary:disabled {
    opacity: 0.5;
  }
</style>
