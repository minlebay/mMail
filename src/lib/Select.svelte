<script>
  // Свой выпадающий список вместо <select>: нативный рисуется попапом GTK,
  // на который не действуют ни тема, ни размытие.
  // options: [{ value, label }]
  let { value = $bindable(), options, label = '' } = $props();

  let open = $state(false);
  let highlighted = $state(0);
  // Куда раскрываться и сколько места есть: у края окна список открывается вверх.
  let up = $state(false);
  let maxHeight = $state(260);
  let root;

  const OPTION_HEIGHT = 34;
  function place() {
    const rect = root.getBoundingClientRect();
    const wanted = Math.min(260, options.length * OPTION_HEIGHT + 10);
    const below = window.innerHeight - rect.bottom - 12;
    const above = rect.top - 12;
    up = below < wanted && above > below;
    maxHeight = Math.max(80, Math.min(260, up ? above : below));
  }

  const current = $derived(options.find((o) => o.value === value));

  function toggle() {
    open = !open;
    if (open) place();
    if (open) highlighted = Math.max(0, options.findIndex((o) => o.value === value));
  }

  function choose(option) {
    value = option.value;
    open = false;
    root?.querySelector('button.field')?.focus();
  }

  function onkeydown(e) {
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
        e.preventDefault();
        toggle();
      }
      return;
    }
    if (e.key === 'ArrowDown') highlighted = Math.min(options.length - 1, highlighted + 1);
    else if (e.key === 'ArrowUp') highlighted = Math.max(0, highlighted - 1);
    else if (e.key === 'Enter' || e.key === ' ') choose(options[highlighted]);
    else if (e.key === 'Escape') open = false;
    else return;
    // Escape не должен закрывать окно целиком, пока открыт список.
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<div class="select" bind:this={root}>
  <button class="field" type="button" aria-haspopup="listbox" aria-expanded={open} aria-label={label} onclick={toggle} {onkeydown}>
    <span class="text">{current?.label ?? ''}</span>
    <svg class="chevron" class:open viewBox="0 0 12 12"><path d="M3 4.5l3 3 3-3" /></svg>
  </button>
  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="backdrop" onclick={() => (open = false)}></div>
    <ul class="list" class:up role="listbox" style:max-height="{maxHeight}px">
      {#each options as option, i (option.value)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          role="option"
          aria-selected={option.value === value}
          class:selected={option.value === value}
          class:highlighted={i === highlighted}
          onmouseenter={() => (highlighted = i)}
          onclick={() => choose(option)}
        >
          {option.label}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .field {
    width: 100%;
    height: 34px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border-radius: 9px;
    background: rgba(var(--ink), 0.06);
    border: 1px solid transparent;
    text-align: left;
  }
  .field:hover {
    background: rgba(var(--ink), 0.1);
  }
  .field:focus-visible,
  .field[aria-expanded='true'] {
    outline: none;
    border-color: var(--accent);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chevron {
    width: 12px;
    height: 12px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    color: var(--muted);
    transition: transform 0.15s;
  }
  .chevron.open {
    transform: rotate(180deg);
  }

  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
  }
  .list {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    z-index: 41;
    max-height: 260px;
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
  .list.up {
    top: auto;
    bottom: calc(100% + 4px);
  }
  li {
    padding: 8px 10px;
    border-radius: 7px;
    cursor: default;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  li.highlighted {
    background: rgba(var(--ink), 0.08);
  }
  li.selected {
    background: var(--accent-soft);
  }
</style>
