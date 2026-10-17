<script>
  import { onMount, tick } from 'svelte';
  import { t } from './i18n.js';
  import { contextMenu, closeContextMenu, menuPosition } from './contextMenu.js';

  let { onerror = console.error } = $props();
  let root = $state();
  let position = $state({ x: 8, y: 8 });

  $effect(() => {
    const menu = $contextMenu;
    if (!menu) return;
    let cancelled = false;
    tick().then(() => {
      if (cancelled || !root) return;
      position = menuPosition(menu.x, menu.y, root.offsetWidth, root.offsetHeight, window.innerWidth, window.innerHeight);
      root.querySelector('button:not(:disabled)')?.focus();
    });
    return () => { cancelled = true; };
  });

  function dismiss(restore = false) {
    const anchor = $contextMenu?.anchor;
    closeContextMenu();
    if (restore && anchor?.isConnected) anchor.focus();
  }

  async function activate(item) {
    dismiss(true);
    try { await item.run(); } catch (error) { onerror(String(error)); }
  }

  function onkeydown(event) {
    if (!$contextMenu) return;
    if (event.key === 'Escape' || event.key === 'Tab') {
      event.preventDefault();
      event.stopPropagation();
      dismiss(true);
      return;
    }
    const buttons = [...root.querySelectorAll('button:not(:disabled)')];
    const index = buttons.indexOf(document.activeElement);
    let next;
    if (event.key === 'ArrowDown') next = (index + 1) % buttons.length;
    else if (event.key === 'ArrowUp') next = (index - 1 + buttons.length) % buttons.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = buttons.length - 1;
    else return;
    event.preventDefault();
    event.stopPropagation();
    buttons[next]?.focus();
  }

  onMount(() => {
    const outside = (event) => { if ($contextMenu && !root?.contains(event.target)) dismiss(); };
    const scroll = (event) => { if (!root?.contains(event.target)) dismiss(); };
    const close = () => dismiss();
    document.addEventListener('pointerdown', outside, true);
    document.addEventListener('scroll', scroll, true);
    document.addEventListener('keydown', onkeydown, true);
    window.addEventListener('resize', close);
    window.addEventListener('blur', close);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      document.removeEventListener('scroll', scroll, true);
      document.removeEventListener('keydown', onkeydown, true);
      window.removeEventListener('resize', close);
      window.removeEventListener('blur', close);
    };
  });
</script>

{#if $contextMenu}
  <div class="context-menu" role="menu" aria-label={$t('Действия')} tabindex="-1" bind:this={root}
    style:left="{position.x}px" style:top="{position.y}px" oncontextmenu={(event) => event.preventDefault()}>
    {#each $contextMenu.items as item}
      {#if item.separator}
        <div class="separator" role="separator"></div>
      {:else}
        <button role="menuitem" tabindex="-1" disabled={item.disabled} onclick={() => activate(item)}>{$t(item.label)}</button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .context-menu {
    position: fixed;
    z-index: 100;
    min-width: min(220px, calc(100vw - 16px));
    max-width: calc(100vw - 16px);
    max-height: calc(100vh - 16px);
    overflow: auto;
    padding: 6px;
    border: 1px solid var(--glass-border);
    border-radius: 10px;
    background: var(--surface);
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.4);
  }
  button {
    display: block;
    width: 100%;
    padding: 8px 12px;
    border-radius: 6px;
    text-align: left;
    overflow-wrap: anywhere;
  }
  button:hover, button:focus-visible { background: var(--accent-soft); outline: none; }
  button:disabled { opacity: 0.45; }
  .separator { height: 1px; margin: 5px 6px; background: var(--glass-border); }
</style>
