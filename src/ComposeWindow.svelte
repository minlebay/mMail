<script>
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import TitleBar from './lib/TitleBar.svelte';
  import Icon from './lib/Icon.svelte';
  import Select from './lib/Select.svelte';
  import { applyTheme } from './lib/themes.js';
  import { composeInit, sendMessage, pickAttachments, getAppearance } from './lib/api.js';

  const win = getCurrentWindow();

  let init = $state(null);
  let error = $state('');
  let from = $state('');
  let to = $state('');
  let cc = $state('');
  let showCc = $state(false);
  let subject = $state('');
  let mode = $state('html');
  let text = $state('');
  let editor = $state();
  let attachments = $state([]);
  let sending = $state(false);
  let confirmDiscard = $state(false);
  let maximized = $state(false);
  let middleClickMinimize = $state(true);
  let linkOpen = $state(false);
  let linkUrl = $state('');
  let savedRange = null;
  let touched = $state(false);

  // Оформление как у главного окна, и сразу подхватываем изменения из настроек.
  function applyAppearance(a) {
    applyTheme(a.theme, a.opacity ?? undefined);
    middleClickMinimize = a.middleClickMinimize;
  }

  onMount(() => {
    getAppearance().then(applyAppearance);
    const stop = listen('appearance-changed', (e) => applyAppearance(e.payload));
    const syncMaximized = () => win.isMaximized().then((m) => (maximized = m));
    syncMaximized();
    window.addEventListener('resize', syncMaximized);

    composeInit()
      .then((data) => {
        init = data;
        from = data.from;
        to = data.to;
        subject = data.subject;
        setupBody(data);
      })
      .catch((e) => (error = String(e)));

    return () => {
      stop.then((f) => f());
      window.removeEventListener('resize', syncMaximized);
    };
  });

  $effect(() => {
    win.setTitle(subject.trim() || 'Новое письмо');
  });

  const escapeHtml = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);

  // HTML → текст с переносами строк на месте блоков (DOMParser не исполняет скрипты и не грузит картинки).
  function htmlToText(html) {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    const BLOCK = /^(P|DIV|LI|TR|H[1-6]|BLOCKQUOTE|PRE|TABLE|UL|OL)$/;
    let out = '';
    const walk = (node) => {
      if (node.nodeType === Node.TEXT_NODE) {
        out += node.textContent.replace(/\s+/g, ' ');
        return;
      }
      if (node.nodeName === 'BR') {
        out += '\n';
        return;
      }
      const block = BLOCK.test(node.nodeName);
      if (block && out && !out.endsWith('\n')) out += '\n';
      node.childNodes.forEach(walk);
      if (block && !out.endsWith('\n')) out += '\n';
    };
    walk(doc.body);
    return out.replace(/\n{3,}/g, '\n\n').trim();
  }

  const textToHtml = (t) =>
    t
      .split('\n')
      .map((line) => `<div>${line ? escapeHtml(line) : '<br>'}</div>`)
      .join('');

  function setupBody(data) {
    if (!data.quoteHtml && !data.quoteHeader) {
      editor.innerHTML = '<div><br></div>';
      return;
    }
    const header = escapeHtml(data.quoteHeader ?? '').replace(/\n/g, '<br>');
    const quote = data.quoteHtml ?? '';
    editor.innerHTML =
      data.kind === 'forward'
        ? `<div><br></div><div><br></div><div class="fwd">${header}</div>${quote}`
        : `<div><br></div><div><br></div><div>${header}</div><blockquote type="cite">${quote}</blockquote>`;
    placeCaretAtStart();
  }

  function placeCaretAtStart() {
    editor.focus();
    const range = document.createRange();
    range.setStart(editor, 0);
    range.collapse(true);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  function switchMode(next) {
    if (next === mode) return;
    if (next === 'text') {
      text = htmlToText(editor.innerHTML);
    } else {
      queueMicrotask(() => (editor.innerHTML = textToHtml(text)));
    }
    mode = next;
  }

  // Панель форматирования: execCommand работает с выделением внутри contenteditable.
  function format(command, value = null) {
    editor.focus();
    document.execCommand(command, false, value);
  }

  function openLink() {
    const sel = window.getSelection();
    savedRange = sel.rangeCount ? sel.getRangeAt(0).cloneRange() : null;
    linkUrl = 'https://';
    linkOpen = true;
  }

  function applyLink(e) {
    e.preventDefault();
    const url = linkUrl.trim();
    linkOpen = false;
    if (!/^(https?:\/\/|mailto:)\S+$/i.test(url) || !savedRange) return;
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(savedRange);
    format('createLink', url);
  }

  async function attach() {
    const picked = await pickAttachments();
    attachments = [...attachments, ...picked];
  }

  const size = (bytes) =>
    bytes < 1024 ? `${bytes} Б` : bytes < 1048576 ? `${(bytes / 1024).toFixed(0)} КБ` : `${(bytes / 1048576).toFixed(1)} МБ`;
  const totalSize = $derived(attachments.reduce((sum, a) => sum + a.size, 0));

  async function send() {
    if (sending || !init) return;
    sending = true;
    error = '';
    try {
      const html = mode === 'html' ? editor.innerHTML : null;
      await sendMessage({
        from,
        to,
        cc,
        subject,
        body: mode === 'html' ? editor.innerText : text,
        html,
        attachments: attachments.map((a) => a.id),
        inReplyTo: init.inReplyTo,
        references: init.references,
      });
      await win.close();
    } catch (e) {
      error = String(e);
    } finally {
      sending = false;
    }
  }

  const dirty = () => touched || attachments.length > 0 || to.trim() !== (init?.to ?? '');

  function close() {
    if (dirty() && !confirmDiscard) confirmDiscard = true;
    else win.close();
  }

  function onkeydown(e) {
    if (e.key === 'Escape' && !linkOpen) close();
    else if (e.key === 'Enter' && e.ctrlKey) {
      e.preventDefault();
      send();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="window" class:maximized>
  <TitleBar title={subject.trim() || 'Новое письмо'} {middleClickMinimize} onclose={close} />

  <main class="panel compose">
    <div class="fields">
      <div class="row">
        <span>От</span>
        <Select bind:value={from} label="Отправитель" options={(init?.accounts ?? []).map((a) => ({ value: a, label: a }))} />
      </div>
      <label>
        <span>Кому</span>
        <input bind:value={to} placeholder="адрес@пример.ру, ещё@пример.ру" spellcheck="false" />
        {#if !showCc}<button class="link" onclick={() => (showCc = true)}>Копия</button>{/if}
      </label>
      {#if showCc}
        <label>
          <span>Копия</span>
          <input bind:value={cc} spellcheck="false" />
        </label>
      {/if}
      <label>
        <span>Тема</span>
        <input bind:value={subject} oninput={() => (touched = true)} />
      </label>
    </div>

    <div class="toolbar">
      {#if mode === 'html'}
        <!-- mousedown preventDefault: кнопки не забирают выделение у редактора -->
        <button title="Жирный (Ctrl+B)" onmousedown={(e) => e.preventDefault()} onclick={() => format('bold')}><b>Ж</b></button>
        <button title="Курсив (Ctrl+I)" onmousedown={(e) => e.preventDefault()} onclick={() => format('italic')}><i>К</i></button>
        <button title="Подчёркнутый (Ctrl+U)" onmousedown={(e) => e.preventDefault()} onclick={() => format('underline')}><u>Ч</u></button>
        <button title="Зачёркнутый" onmousedown={(e) => e.preventDefault()} onclick={() => format('strikeThrough')}><s>З</s></button>
        <span class="sep"></span>
        <button title="Маркированный список" onmousedown={(e) => e.preventDefault()} onclick={() => format('insertUnorderedList')}>•≡</button>
        <button title="Нумерованный список" onmousedown={(e) => e.preventDefault()} onclick={() => format('insertOrderedList')}>1≡</button>
        <button title="Цитата" onmousedown={(e) => e.preventDefault()} onclick={() => format('formatBlock', 'blockquote')}>❝</button>
        <button title="Ссылка" onmousedown={(e) => e.preventDefault()} onclick={openLink}>🔗</button>
        <button title="Убрать форматирование" onmousedown={(e) => e.preventDefault()} onclick={() => format('removeFormat')}>⌫</button>
        {#if linkOpen}
          <form class="linkform" onsubmit={applyLink}>
            <!-- svelte-ignore a11y_autofocus -->
            <input bind:value={linkUrl} autofocus spellcheck="false" />
            <button class="ok">OK</button>
          </form>
        {/if}
      {/if}
      <div class="modes">
        <button class:active={mode === 'html'} onclick={() => switchMode('html')}>HTML</button>
        <button class:active={mode === 'text'} onclick={() => switchMode('text')}>Текст</button>
      </div>
    </div>

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="editor"
      class:hidden={mode !== 'html'}
      contenteditable="true"
      role="textbox"
      aria-multiline="true"
      tabindex="0"
      bind:this={editor}
      oninput={() => (touched = true)}
    ></div>
    {#if mode === 'text'}
      <textarea class="editor" bind:value={text} oninput={() => (touched = true)} placeholder="Текст письма"></textarea>
    {/if}

    {#if attachments.length}
      <div class="attachments">
        {#each attachments as a (a.id)}
          <span class="file" title={a.name}>
            <Icon name="file" size={14} />
            <span class="fname">{a.name}</span>
            <span class="fsize">{size(a.size)}</span>
            <button aria-label="Убрать" onclick={() => (attachments = attachments.filter((x) => x.id !== a.id))}>×</button>
          </span>
        {/each}
      </div>
    {/if}

    {#if error}<div class="error">{error}</div>{/if}

    <footer>
      {#if confirmDiscard}
        <span class="muted">Удалить черновик?</span>
        <button class="secondary" onclick={() => win.close()}>Удалить</button>
        <button class="secondary" onclick={() => (confirmDiscard = false)}>Продолжить писать</button>
      {:else}
        <button class="secondary" onclick={attach}>📎 Прикрепить</button>
        <span class="muted">
          {#if attachments.length}{size(totalSize)} из 25 МБ ·{/if} Ctrl+Enter — отправить
        </span>
        <button class="primary" disabled={sending || !init?.accounts.length} onclick={send}>
          {sending ? 'Отправка…' : 'Отправить'}
        </button>
      {/if}
    </footer>
  </main>
</div>

<style>
  .window {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: 12px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    background: var(--window-tint);
  }
  .window.maximized {
    border-radius: 0;
    border: 0;
  }

  .compose {
    flex: 1;
    min-height: 0;
    margin: 4px 16px 16px;
    padding: 6px 18px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .fields {
    display: flex;
    flex-direction: column;
  }
  label,
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 40px;
    border-bottom: 1px solid var(--glass-border);
  }
  label span,
  .row > span {
    width: 52px;
    flex: none;
    color: var(--muted);
    font-size: 13px;
  }
  input {
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
  .link {
    flex: none;
    color: var(--muted);
    font-size: 13px;
  }
  .link:hover {
    color: var(--text);
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }
  .toolbar > button,
  .modes button {
    min-width: 30px;
    height: 30px;
    padding: 0 8px;
    border-radius: 7px;
    font-size: 14px;
  }
  .toolbar > button:hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .sep {
    width: 1px;
    height: 18px;
    margin: 0 4px;
    background: var(--glass-border);
  }
  .linkform {
    display: flex;
    gap: 4px;
  }
  .linkform input {
    width: 240px;
    height: 30px;
    padding: 0 10px;
    border-radius: 7px;
    border: 1px solid var(--accent);
    background: rgba(255, 255, 255, 0.06);
  }
  .linkform .ok {
    height: 30px;
    padding: 0 10px;
    border-radius: 7px;
    background: var(--accent-soft);
  }
  .modes {
    margin-left: auto;
    display: flex;
    padding: 2px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.06);
  }
  .modes button {
    font-size: 12.5px;
    color: var(--muted);
  }
  .modes button.active {
    background: var(--accent-soft);
    color: var(--text);
  }

  .editor {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 14px;
    border-radius: 10px;
    border: 1px solid var(--glass-border);
    background: rgba(255, 255, 255, 0.04);
    outline: none;
    font: 15px/1.6 var(--font);
    color: inherit;
    resize: none;
    user-select: text;
    overflow-wrap: anywhere;
    /* Цитата чужого письма не может вылезти поверх интерфейса (position: fixed и т.п.). */
    contain: paint;
    transform: translateZ(0);
  }
  .editor:focus {
    border-color: var(--accent);
  }
  .editor.hidden {
    display: none;
  }
  .editor :global(blockquote) {
    margin: 0 0 0 4px;
    padding-left: 12px;
    border-left: 2px solid var(--accent);
    color: var(--muted);
  }
  .editor :global(a) {
    color: var(--accent);
  }
  .editor :global(img) {
    max-width: 100%;
    height: auto;
  }
  .editor :global(.fwd) {
    color: var(--muted);
  }

  .attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .file {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 280px;
    height: 30px;
    padding: 0 6px 0 10px;
    border-radius: 15px;
    background: rgba(255, 255, 255, 0.08);
    font-size: 12.5px;
  }
  .fname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fsize {
    color: var(--muted);
    flex: none;
  }
  .file button {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    flex: none;
  }
  .file button:hover {
    background: rgba(255, 255, 255, 0.15);
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
    gap: 10px;
  }
  .muted {
    flex: 1;
    min-width: 0;
    color: var(--muted);
    font-size: 12.5px;
  }
  .primary,
  .secondary {
    height: 36px;
    padding: 0 18px;
    border-radius: 9px;
    flex: none;
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
