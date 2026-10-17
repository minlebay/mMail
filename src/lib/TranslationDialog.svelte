<script>
  import { t } from './i18n.js';
  import { listen } from '@tauri-apps/api/event';
  import { translateText, writeClipboard } from './api.js';

  // «Перевести выделенное»: ядро присылает выделенный текст, перевод показываем только как текст
  // (интерполяция Svelte экранирует всё — разметка из ответа переводчика не исполняется).
  let state = $state(null); // { loading } | { text, sourceLanguage } | { error }
  let closeButton = $state();
  let request = 0;

  $effect(() => {
    const unlisten = listen('translate-selection', async (e) => {
      const mine = ++request;
      state = { loading: true };
      try {
        const result = await translateText(e.payload);
        if (mine === request) state = result;
      } catch (err) {
        if (mine === request) state = { error: String(err) };
      }
    });
    return () => unlisten.then((stop) => stop());
  });
  $effect(() => closeButton?.focus());

  function onkeydown(e) {
    if (state && e.key === 'Escape') state = null;
  }
</script>

<svelte:window {onkeydown} />

{#if state}
  <div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && (state = null)}>
    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="translation-title">
      <h2 id="translation-title">
        {$t('Перевод')}{#if state.sourceLanguage}<span class="lang"> · {state.sourceLanguage}</span>{/if}
      </h2>
      {#if state.loading}
        <div class="text muted">{$t('Перевожу…')}</div>
      {:else if state.error}
        <div class="warn">{state.error}</div>
      {:else}
        <div class="text">{state.text}</div>
      {/if}
      <div class="buttons">
        {#if state.text}<button class="secondary" onclick={() => writeClipboard(state.text)}>{$t('Копировать')}</button>{/if}
        <button class="primary" bind:this={closeButton} onclick={() => (state = null)}>{$t('Закрыть')}</button>
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
    width: min(560px, calc(100% - 48px));
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
  .text {
    max-height: 50vh;
    overflow-y: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 1.5;
    user-select: text;
  }
  .muted {
    color: var(--muted);
  }
  .warn {
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 79, 120, 0.18);
    color: #ffd0da;
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
    background: rgba(255, 255, 255, 0.1);
  }
  .secondary:hover {
    background: rgba(255, 255, 255, 0.18);
  }
  .primary {
    background: var(--accent);
    color: #fff;
  }
  .primary:hover {
    filter: brightness(1.1);
  }
  button:focus-visible {
    outline: 2px solid rgba(255, 255, 255, 0.7);
    outline-offset: 2px;
  }
</style>
