<script>
  import { tick } from 'svelte';
  import { covers } from '../covers.js';
  import { t } from '../i18n.js';
  import { parseTime, timeToMinutes, minutesToTime } from './dates.js';

  // Поле времени вместо <input type="time">: тот рисует 12-часовой формат по локали WebKit.
  // Время можно набрать («930», «9.30», «14») или выбрать из списка с шагом 15 минут.
  // from — время начала в тот же день: список начинается с него и показывает длительность.
  let { value = $bindable(), from = null, label = '', onchange = null } = $props();

  const STEP = 15;
  let text = $state(value);
  let open = $state(false);
  let highlighted = $state(0);
  let pos = $state({ left: 0, top: 0, up: false, maxHeight: 240 });
  let input, list;

  $effect(() => {
    // Значение поменяли снаружи (например, конец сдвинулся вслед за началом).
    if (document.activeElement !== input) text = value;
  });

  function duration(minutes) {
    const hours = Math.floor(minutes / 60);
    if (!hours) return $t('{minutes} мин', { minutes });
    if (minutes % 60) return $t('{hours} ч {minutes} мин', { hours, minutes: minutes % 60 });
    return $t('{hours} ч', { hours });
  }

  const options = $derived.by(() => {
    const start = from == null ? 0 : timeToMinutes(from);
    const result = [];
    for (let m = start; m < 24 * 60; m += STEP) {
      result.push({ value: minutesToTime(m), hint: from == null ? '' : duration(m - start) });
    }
    return result;
  });

  // Ближайший пункт списка к набранному времени.
  function nearest(time) {
    if (!time) return 0;
    const target = timeToMinutes(time);
    let best = 0;
    options.forEach((o, i) => {
      if (Math.abs(timeToMinutes(o.value) - target) < Math.abs(timeToMinutes(options[best].value) - target)) best = i;
    });
    return best;
  }

  async function show() {
    if (open) return;
    const rect = input.getBoundingClientRect();
    const below = window.innerHeight - rect.bottom - 12;
    const above = rect.top - 12;
    const up = below < 200 && above > below;
    pos = { left: rect.left, top: up ? rect.top - 4 : rect.bottom + 4, up, maxHeight: Math.max(120, Math.min(240, up ? above : below)) };
    highlighted = nearest(parseTime(text));
    open = true;
    await tick();
    reveal('center');
  }

  function reveal(block = 'nearest') {
    list?.children[highlighted]?.scrollIntoView({ block });
  }

  function commit(time) {
    open = false;
    if (!time) {
      text = value;
      return;
    }
    text = time;
    if (time !== value) {
      const previous = value;
      value = time;
      onchange?.(time, previous);
    }
  }

  function oninput() {
    show();
    const time = parseTime(text);
    if (time) {
      highlighted = nearest(time);
      tick().then(() => reveal('center'));
    }
  }

  async function onkeydown(e) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      if (!open) return show();
      highlighted = Math.max(0, Math.min(options.length - 1, highlighted + (e.key === 'ArrowDown' ? 1 : -1)));
      text = options[highlighted].value;
      await tick();
      reveal();
    } else if (e.key === 'Enter') {
      // При открытом списке Enter выбирает время, а не отправляет форму.
      if (open) e.preventDefault();
      commit(parseTime(text));
    } else if (e.key === 'Escape' && open) {
      e.preventDefault();
      e.stopPropagation();
      open = false;
      text = value;
    }
  }
</script>

<div class="time">
  <input
    bind:this={input}
    bind:value={text}
    aria-label={label}
    inputmode="numeric"
    autocomplete="off"
    spellcheck="false"
    maxlength="5"
    class:invalid={!parseTime(text)}
    onfocus={(e) => e.currentTarget.select()}
    onclick={show}
    {oninput}
    {onkeydown}
    onblur={() => commit(parseTime(text))}
  />
  {#if open}
    <ul
      class="list"
      class:wide={from != null}
      bind:this={list}
      use:covers
      role="listbox"
      style:left="{pos.left}px"
      style:top="{pos.top}px"
      style:max-height="{pos.maxHeight}px"
      style:transform={pos.up ? 'translateY(-100%)' : null}
    >
      {#each options as option, i (option.value)}
        <!-- Клик по пункту — до blur поля, иначе список закроется раньше выбора. -->
        <li
          role="option"
          aria-selected={option.value === value}
          class:selected={option.value === value}
          class:highlighted={i === highlighted}
          onmouseenter={() => (highlighted = i)}
          onmousedown={(e) => {
            e.preventDefault();
            commit(option.value);
          }}
        >
          <span>{option.value}</span>
          {#if option.hint}<span class="hint">{option.hint}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  input {
    width: 76px;
    height: 34px;
    padding: 0 10px;
    border-radius: 9px;
    border: 1px solid var(--glass-border);
    background: rgba(var(--ink), 0.06);
    color: inherit;
    font: inherit;
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
  input:focus {
    outline: none;
    border-color: var(--accent);
  }
  input.invalid {
    border-color: var(--danger);
  }
  .list {
    position: fixed;
    z-index: 41;
    min-width: 76px;
    overflow-y: auto;
    margin: 0;
    padding: 4px;
    list-style: none;
    border-radius: 10px;
    background:
      linear-gradient(180deg, rgba(var(--ink), 0.07), rgba(var(--ink), 0.02)),
      var(--glass-menu);
    border: 1px solid var(--glass-border);
    box-shadow: 0 14px 40px rgba(0, 0, 0, 0.45), inset 0 1px 0 rgba(var(--ink), 0.08);
  }
  .list.wide {
    min-width: 150px;
  }
  li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 7px 10px;
    border-radius: 7px;
    cursor: default;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  li.highlighted {
    background: rgba(var(--ink), 0.08);
  }
  li.selected {
    background: var(--accent-soft);
  }
  .hint {
    color: var(--muted);
  }
</style>
