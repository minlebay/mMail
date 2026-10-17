<script>
  import { untrack } from 'svelte';
  import { sendMessage } from './api.js';

  // draft: { from, to, cc, subject, body }; accounts: адреса подключённых ящиков.
  let { draft: initialDraft, accounts, onclose, onsent } = $props();

  let draft = $state(untrack(() => ({ ...initialDraft })));
  let showCc = $state(untrack(() => Boolean(initialDraft.cc)));
  let sending = $state(false);
  let error = $state('');
  let confirmDiscard = $state(false);

  const dirty = $derived(Boolean(draft.to.trim() || draft.subject.trim() || draft.body.trim()));

  async function send() {
    if (sending) return;
    sending = true;
    error = '';
    try {
      await sendMessage({ ...draft });
      onsent();
    } catch (e) {
      error = String(e);
    } finally {
      sending = false;
    }
  }

  function close() {
    if (dirty && !confirmDiscard) confirmDiscard = true;
    else onclose();
  }

  function onkeydown(e) {
    if (e.key === 'Escape') close();
    else if (e.key === 'Enter' && e.ctrlKey) send();
  }

  // Фокус: на «Кому», если пусто (новое письмо), иначе на текст (ответ).
  function autofocus(node, when) {
    if (when) node.focus();
  }
</script>

<svelte:window {onkeydown} />

<div class="overlay" role="presentation">
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Новое письмо">
    <header>
      <h2>{draft.subject.trim() || 'Новое письмо'}</h2>
      <button class="close" aria-label="Закрыть" onclick={close}>
        <svg viewBox="0 0 12 12"><path d="M3 3l6 6M9 3l-6 6" /></svg>
      </button>
    </header>

    <div class="fields">
      <label>
        <span>От</span>
        <select bind:value={draft.from}>
          {#each accounts as address (address)}<option value={address}>{address}</option>{/each}
        </select>
      </label>
      <label>
        <span>Кому</span>
        <input bind:value={draft.to} placeholder="адрес@пример.ру, ещё@пример.ру" spellcheck="false" use:autofocus={!draft.to} />
        {#if !showCc}<button class="link" onclick={() => (showCc = true)}>Копия</button>{/if}
      </label>
      {#if showCc}
        <label>
          <span>Копия</span>
          <input bind:value={draft.cc} spellcheck="false" />
        </label>
      {/if}
      <label>
        <span>Тема</span>
        <input bind:value={draft.subject} />
      </label>
    </div>

    <textarea bind:value={draft.body} placeholder="Текст письма" use:autofocus={!!draft.to}></textarea>

    {#if error}<div class="error">{error}</div>{/if}

    <footer>
      {#if confirmDiscard}
        <span class="muted">Удалить черновик?</span>
        <button class="secondary" onclick={onclose}>Удалить</button>
        <button class="secondary" onclick={() => (confirmDiscard = false)}>Продолжить писать</button>
      {:else}
        <span class="muted">Ctrl+Enter — отправить</span>
        <button class="primary" disabled={sending || !accounts.length} onclick={send}>
          {sending ? 'Отправка…' : 'Отправить'}
        </button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: grid;
    place-items: center;
    background: rgba(10, 8, 18, 0.5);
    border-radius: 12px;
  }

  .dialog {
    width: min(760px, calc(100% - 48px));
    height: min(640px, calc(100% - 48px));
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 22px 18px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--glass-border);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.45);
  }

  header {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  h2 {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: 16px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .close {
    width: 28px;
    height: 28px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.1);
  }
  .close:hover {
    background: rgba(255, 255, 255, 0.2);
  }
  .close svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
  }

  .fields {
    display: flex;
    flex-direction: column;
  }
  label {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 40px;
    border-bottom: 1px solid var(--glass-border);
  }
  label span {
    width: 52px;
    flex: none;
    color: var(--muted);
    font-size: 13px;
  }
  input,
  select {
    flex: 1;
    min-width: 0;
    height: 38px;
    border: 0;
    outline: none;
    background: transparent;
    font: inherit;
    color: inherit;
    user-select: text;
  }
  select option {
    background: var(--surface);
  }

  .link {
    flex: none;
    color: var(--muted);
    font-size: 13px;
  }
  .link:hover {
    color: var(--text);
  }

  textarea {
    flex: 1;
    min-height: 0;
    resize: none;
    padding: 12px 14px;
    border-radius: 10px;
    border: 1px solid var(--glass-border);
    background: rgba(255, 255, 255, 0.04);
    outline: none;
    font: 15px/1.6 var(--font);
    color: inherit;
    user-select: text;
  }
  textarea:focus {
    border-color: var(--accent);
  }

  .error {
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 79, 120, 0.18);
    color: #ffd0da;
    font-size: 13px;
    user-select: text;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
  }
  .muted {
    margin-right: auto;
    color: var(--muted);
    font-size: 12.5px;
  }

  .primary,
  .secondary {
    height: 36px;
    padding: 0 18px;
    border-radius: 9px;
  }
  .primary {
    background: var(--accent);
    color: #fff;
  }
  .primary:disabled {
    opacity: 0.6;
  }
  .secondary {
    background: rgba(255, 255, 255, 0.1);
  }
  .secondary:hover {
    background: rgba(255, 255, 255, 0.18);
  }
</style>
