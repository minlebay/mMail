<script>
  import { t } from './lib/i18n.js';
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import TitleBar from './lib/TitleBar.svelte';
  import Icon from './lib/Icon.svelte';
  import Select from './lib/Select.svelte';
  import { applyTheme, isDark } from './lib/themes.js';
  import { composeInit, sendMessage, pickAttachments, getAppearance, cleanComposeHtml } from './lib/api.js';

  const win = getCurrentWindow();

  let init = $state(null);
  let error = $state('');
  let from = $state('');
  let to = $state('');
  let cc = $state('');
  let bcc = $state('');
  let showCc = $state(false);
  let subject = $state('');
  let mode = $state('html');
  let text = $state('');
  // Режим «Код»: HTML письма как текст с тегами.
  let code = $state('');
  let linkAnchor = null;
  let linkRemovable = $state(false);
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
    applyTheme(a.theme, a.opacity ?? undefined, isDark(a), a.blurEnabled ?? true);
    middleClickMinimize = a.middleClickMinimize;
  }

  onMount(() => {
    getAppearance().then(applyAppearance);
    const stop = listen('appearance-changed', (e) => applyAppearance(e.payload));
    const syncMaximized = () => win.isMaximized().then((m) => (maximized = m));
    syncMaximized();
    window.addEventListener('resize', syncMaximized);
    document.addEventListener('selectionchange', rememberSelection);

    composeInit()
      .then((data) => {
        init = data;
        from = data.from;
        to = data.to;
        cc = data.cc ?? '';
        bcc = data.bcc ?? '';
        showCc = !!(cc || bcc);
        subject = data.subject;
        attachments = data.attachments ?? [];
        setupBody(data);
      })
      .catch((e) => (error = String(e)));

    return () => {
      stop.then((f) => f());
      window.removeEventListener('resize', syncMaximized);
      document.removeEventListener('selectionchange', rememberSelection);
    };
  });

  $effect(() => {
    win.setTitle(subject.trim() || $t("Новое письмо"));
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
      // Готовый текст (письмо открыто извне) — закрытие окна переспросит, как после правки.
      editor.innerHTML = data.body ? textToHtml(data.body) : '<div><br></div>';
      if (data.body) touched = true;
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

  const currentHtml = () => (mode === 'html' ? editor.innerHTML : mode === 'code' ? code : textToHtml(text));

  async function switchMode(next) {
    if (next === mode) return;
    const html = currentHtml();
    if (next === 'text') {
      text = htmlToText(html);
    } else if (next === 'code') {
      code = html;
    } else {
      // Из кода — только через фильтр ядра; текст экранирован textToHtml.
      let clean = html;
      if (mode === 'code') {
        try {
          clean = await cleanComposeHtml(html);
        } catch (e) {
          error = String(e);
          return;
        }
      }
      queueMicrotask(() => (editor.innerHTML = clean));
    }
    mode = next;
  }

  // Панель форматирования: execCommand работает с выделением внутри contenteditable.
  function format(command, value = null) {
    editor.focus();
    document.execCommand(command, false, value);
  }

  // Последнее положение курсора/выделения в теле письма: поле адреса ссылки забирает фокус,
  // а сам редактор при focus() ставит курсор в начало.
  function rememberSelection() {
    const sel = window.getSelection();
    if (sel.rangeCount && editor?.contains(sel.getRangeAt(0).commonAncestorContainer)) {
      savedRange = sel.getRangeAt(0).cloneRange();
    }
  }

  // Ссылка под курсором или внутри выделения — её можно изменить или убрать.
  function linkAt(range) {
    if (!range) return null;
    const node = range.commonAncestorContainer;
    const el = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement;
    const anchor = el?.closest('a');
    if (anchor && editor.contains(anchor)) return anchor;
    return range.collapsed ? null : [...editor.querySelectorAll('a')].find((a) => range.intersectsNode(a)) ?? null;
  }

  function openLink() {
    rememberSelection();
    linkAnchor = linkAt(savedRange);
    linkRemovable = !!linkAnchor;
    linkUrl = linkAnchor?.getAttribute('href') ?? 'https://';
    linkOpen = true;
  }

  function restoreSelection() {
    editor.focus();
    const sel = window.getSelection();
    sel.removeAllRanges();
    if (savedRange && editor.contains(savedRange.commonAncestorContainer)) {
      sel.addRange(savedRange);
    } else {
      // Курсор в тело ещё не ставили — в конец письма.
      const end = document.createRange();
      end.selectNodeContents(editor);
      end.collapse(false);
      sel.addRange(end);
    }
    return sel;
  }

  function removeLink() {
    linkOpen = false;
    const sel = restoreSelection();
    // Курсор просто стоит в ссылке — убираем её целиком, текст остаётся.
    if (sel.isCollapsed && linkAnchor) {
      const range = document.createRange();
      range.selectNodeContents(linkAnchor);
      sel.removeAllRanges();
      sel.addRange(range);
    }
    document.execCommand('unlink');
    touched = true;
  }

  function applyLink(e) {
    e.preventDefault();
    const url = linkUrl.trim();
    linkOpen = false;
    if (!/^(https?:\/\/|mailto:)\S+$/i.test(url)) return;
    const sel = restoreSelection();
    if (sel.isCollapsed && linkAnchor) {
      // Курсор в существующей ссылке — меняем её адрес.
      linkAnchor.setAttribute('href', url);
    } else if (sel.isCollapsed) {
      // Без выделения — вставляем сам адрес ссылкой там, где курсор.
      document.execCommand('insertHTML', false, `<a href="${escapeHtml(url)}">${escapeHtml(url)}</a>`);
    } else {
      document.execCommand('createLink', false, url);
    }
    touched = true;
  }

  async function attach() {
    const picked = await pickAttachments();
    attachments = [...attachments, ...picked];
  }

  const size = (bytes) =>
    bytes < 1024 ? `${bytes} ${$t("Б")}` : bytes < 1048576 ? `${(bytes / 1024).toFixed(0)} ${$t("КБ")}` : `${(bytes / 1048576).toFixed(1)} ${$t("МБ")}`;
  const totalSize = $derived(attachments.reduce((sum, a) => sum + a.size, 0));

  async function send() {
    if (sending || !init) return;
    sending = true;
    error = '';
    try {
      const html = mode === 'text' ? null : currentHtml();
      await sendMessage({
        from,
        to,
        cc,
        bcc,
        subject,
        body: mode === 'html' ? editor.innerText : mode === 'code' ? htmlToText(code) : text,
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
  <TitleBar title={subject.trim() || $t("Новое письмо")} {middleClickMinimize} onclose={close} />

  <main class="panel compose">
    <div class="fields">
      <div class="row">
        <span>{$t("От")}</span>
        <Select bind:value={from} label={$t("Отправитель")} options={(init?.accounts ?? []).map((a) => ({ value: a, label: a }))} />
      </div>
      <label>
        <span>{$t("Кому")}</span>
        <input bind:value={to} placeholder={$t("адрес@пример.ру, ещё@пример.ру")} spellcheck="false" />
        {#if !showCc}<button class="link" onclick={() => (showCc = true)}>{$t("Копия")}</button>{/if}
      </label>
      {#if showCc}
        <label>
          <span>{$t("Копия")}</span>
          <input bind:value={cc} spellcheck="false" />
        </label>
        <label>
          <span>{$t("Скрытая")}</span>
          <input bind:value={bcc} spellcheck="false" placeholder={$t("получатели не увидят этих адресов")} />
        </label>
      {/if}
      <label>
        <span>{$t("Тема")}</span>
        <input bind:value={subject} oninput={() => (touched = true)} />
      </label>
    </div>

    <div class="toolbar">
      {#if mode === 'html'}
        <!-- mousedown preventDefault: кнопки не забирают выделение у редактора -->
        <button title={$t("Жирный (Ctrl+B)")} onmousedown={(e) => e.preventDefault()} onclick={() => format('bold')}><b>{$t("Ж")}</b></button>
        <button title={$t("Курсив (Ctrl+I)")} onmousedown={(e) => e.preventDefault()} onclick={() => format('italic')}><i>{$t("К")}</i></button>
        <button title={$t("Подчёркнутый (Ctrl+U)")} onmousedown={(e) => e.preventDefault()} onclick={() => format('underline')}><u>{$t("Ч")}</u></button>
        <button title={$t("Зачёркнутый")} onmousedown={(e) => e.preventDefault()} onclick={() => format('strikeThrough')}><s>{$t("З")}</s></button>
        <span class="sep"></span>
        <button title={$t("Маркированный список")} onmousedown={(e) => e.preventDefault()} onclick={() => format('insertUnorderedList')}>•≡</button>
        <button title={$t("Нумерованный список")} onmousedown={(e) => e.preventDefault()} onclick={() => format('insertOrderedList')}>1≡</button>
        <button title={$t("Цитата")} onmousedown={(e) => e.preventDefault()} onclick={() => format('formatBlock', 'blockquote')}>❝</button>
        <button title={$t("Ссылка")} onmousedown={(e) => e.preventDefault()} onclick={openLink}>🔗</button>
        <button title={$t("Убрать форматирование")} onmousedown={(e) => e.preventDefault()} onclick={() => format('removeFormat')}>⌫</button>
        {#if linkOpen}
          <form class="linkform" onsubmit={applyLink}>
            <!-- svelte-ignore a11y_autofocus -->
            <input bind:value={linkUrl} autofocus spellcheck="false" />
            <button class="ok">OK</button>
            {#if linkRemovable}<button type="button" class="ok" onclick={removeLink}>{$t("Убрать ссылку")}</button>{/if}
          </form>
        {/if}
      {/if}
      <div class="modes">
        <button class:active={mode === 'html'} onclick={() => switchMode('html')}>HTML</button>
        <button class:active={mode === 'text'} onclick={() => switchMode('text')}>{$t("Текст")}</button>
        <button class:active={mode === 'code'} title={$t("HTML-код письма")} onclick={() => switchMode('code')}>{$t("Код")}</button>
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
      <textarea class="editor" bind:value={text} oninput={() => (touched = true)} placeholder={$t("Текст письма")}></textarea>
    {:else if mode === 'code'}
      <textarea class="editor code" bind:value={code} oninput={() => (touched = true)} spellcheck="false"></textarea>
    {/if}

    {#if attachments.length}
      <div class="attachments">
        {#each attachments as a (a.id)}
          <span class="file" title={a.name}>
            <Icon name="file" size={14} />
            <span class="fname">{a.name}</span>
            <span class="fsize">{size(a.size)}</span>
            <button aria-label={$t("Убрать")} onclick={() => (attachments = attachments.filter((x) => x.id !== a.id))}>×</button>
          </span>
        {/each}
      </div>
    {/if}

    {#if error}<div class="error">{$t(error)}</div>{/if}

    <footer>
      {#if confirmDiscard}
        <span class="muted">{$t("Удалить черновик?")}</span>
        <button class="secondary" onclick={() => win.close()}>{$t("Удалить")}</button>
        <button class="secondary" onclick={() => (confirmDiscard = false)}>{$t("Продолжить писать")}</button>
      {:else}
        <button class="secondary" onclick={attach}>{$t("📎 Прикрепить")}</button>
        <span class="muted">
          {#if attachments.length}{size(totalSize)} {$t("из 25 МБ ·")}{/if} {$t("Ctrl+Enter — отправить")}
        </span>
        <button class="primary" disabled={sending || !init?.accounts.length} onclick={send}>
          {sending ? $t("Отправка…") : $t("Отправить")}
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
    border: 1px solid rgba(var(--ink), 0.14);
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
    background: rgba(var(--ink), 0.1);
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
    background: rgba(var(--ink), 0.06);
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
    background: rgba(var(--ink), 0.06);
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
    /* Цитата чужого письма со стилями вроде position:fixed не должна перекрывать поля окна. */
    contain: paint;
    padding: 12px 14px;
    border-radius: 10px;
    border: 1px solid var(--glass-border);
    background: rgba(var(--ink), 0.04);
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
  .editor.code {
    font-family: ui-monospace, 'JetBrains Mono', 'DejaVu Sans Mono', monospace;
    font-size: 12.5px;
    white-space: pre-wrap;
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
    background: rgba(var(--ink), 0.08);
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
    background: rgba(var(--ink), 0.15);
  }

  .error {
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 79, 120, 0.18);
    color: var(--danger-text);
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
    background: var(--accent-strong);
    color: #fff;
  }
  .primary:disabled {
    opacity: 0.6;
  }
  .secondary {
    background: rgba(var(--ink), 0.1);
  }
  .secondary:hover {
    background: rgba(var(--ink), 0.18);
  }
</style>
