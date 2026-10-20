<script>
  import { covers } from './covers.js';
  import { t } from './i18n.js';
  import { listen } from '@tauri-apps/api/event';
  import { openLink } from './api.js';

  // Ядро присылает сюда каждый клик по ссылке в письме; открывает только по id после подтверждения.
  let request = $state(null);
  let cancelButton = $state();

  $effect(() => {
    const unlisten = listen('link-request', (e) => (request = e.payload));
    return () => unlisten.then((stop) => stop());
  });

  $effect(() => cancelButton?.focus());

  function open() {
    const { id } = request;
    request = null;
    openLink(id);
  }

  function onkeydown(e) {
    if (request && e.key === 'Escape') request = null;
  }
</script>

<svelte:window {onkeydown} />

{#if request}
  <div class="overlay" use:covers role="presentation" onclick={(e) => e.target === e.currentTarget && (request = null)}>
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="link-title">
      <h2 id="link-title">{$t("Открыть ссылку?")}</h2>
      <div class="host">{request.host || request.url}</div>
      <div class="url">{request.url}</div>

      {#if request.punycode}
        <div class="warn">{$t("В адресе закодированы не-латинские символы — домен может выдавать себя за другой")}</div>
      {/if}
      {#if request.insecure}
        <div class="warn">{$t("Соединение без шифрования (http)")}</div>
      {/if}

      <div class="buttons">
        <button class="secondary" bind:this={cancelButton} onclick={() => (request = null)}>{$t("Отмена")}</button>
        <button class="primary" onclick={open}>{$t("Открыть в браузере")}</button>
      </div>
    </div>
  </div>
{/if}

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
    width: min(480px, calc(100% - 48px));
    padding: 22px 24px 18px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--glass-border);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.45);
  }

  h2 {
    margin: 0 0 14px;
    font-size: 16px;
    font-weight: 500;
    color: var(--muted);
  }

  .host {
    font-size: 20px;
    font-weight: 600;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .url {
    margin-top: 6px;
    font: 12.5px/1.5 ui-monospace, 'JetBrains Mono', monospace;
    color: var(--muted);
    overflow-wrap: anywhere;
    max-height: 7.5em;
    overflow-y: auto;
    user-select: text;
  }

  .warn {
    margin-top: 12px;
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 79, 120, 0.18);
    color: var(--danger-text);
    font-size: 13px;
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    margin-top: 20px;
  }

  button {
    height: 36px;
    padding: 0 16px;
    border-radius: 9px;
  }
  .secondary {
    background: rgba(var(--ink), 0.1);
  }
  .secondary:hover {
    background: rgba(var(--ink), 0.18);
  }
  .primary {
    background: var(--accent-strong);
    color: #fff;
  }
  .primary:hover {
    filter: brightness(1.1);
  }
  button:focus-visible {
    outline: 2px solid rgba(var(--ink), 0.7);
    outline-offset: 2px;
  }
</style>
