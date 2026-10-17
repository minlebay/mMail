<script>
  import { t } from './i18n.js';
  import Icon from './Icon.svelte';
  import { showContextMenu } from './contextMenu.js';
  import { getMessage, writeClipboard, setSenderTrusted, saveAttachment, translateMessage } from './api.js';
  import { translateRequest } from './translate.js';
  import { darkScheme } from './themes.js';

  // Документ письма собирает ядро (sanitize::wrap, начинается с его <html>); схему окна
  // передаём атрибутом — стили обеих схем уже в нём.
  const withScheme = (doc, dark) => (dark ? doc : doc.replace('<html>', '<html data-scheme="light">'));

  let { id, onreply } = $props();

  let msg = $state(null);
  let error = $state('');
  let loadingRemote = $state(false);
  let request = 0;

  $effect(() => {
    const current = id;
    msg = null;
    error = '';
    saved = {};
    translation = null;
    if (current != null) load(current, false);
  });

  // Перевод: null — оригинал; { loading } — ждём переводчик; { srcdoc, sourceLanguage } — готово.
  let translation = $state(null);
  let translationError = $state('');

  async function translate() {
    const current = id;
    translationError = '';
    translation = { loading: true };
    try {
      const result = await translateMessage(current);
      if (current === id) translation = result;
    } catch (e) {
      if (current === id) {
        translation = null;
        translationError = String(e);
      }
    }
  }

  // «Перевести письмо» из меню списка писем.
  $effect(() => {
    const request = $translateRequest;
    if (request && request.id === id) {
      translateRequest.set(null);
      translate();
    }
  });

  async function load(current, showRemote) {
    const mine = ++request;
    try {
      const m = await getMessage(current, showRemote);
      if (mine === request) msg = m;
    } catch (e) {
      if (mine === request) error = String(e);
    }
  }

  async function showRemote() {
    loadingRemote = true;
    await load(msg.id, true);
    loadingRemote = false;
  }

  let saved = $state({});

  const size = (bytes) =>
    bytes < 1024 ? `${bytes} ${$t("Б")}` : bytes < 1048576 ? `${(bytes / 1024).toFixed(0)} ${$t("КБ")}` : `${(bytes / 1048576).toFixed(1)} ${$t("МБ")}`;

  async function save(index) {
    saved = { ...saved, [index]: 'saving' };
    try {
      const ok = await saveAttachment(msg.id, index);
      saved = { ...saved, [index]: ok ? 'done' : undefined };
    } catch (e) {
      saved = { ...saved, [index]: undefined };
      error = String(e);
    }
  }

  async function trust(trusted) {
    await setSenderTrusted(msg.fromAddr, trusted);
    loadingRemote = trusted;
    await load(msg.id, trusted);
    loadingRemote = false;
  }

  function menu(event) {
    // Keep native copying for selected text and editable controls.
    if (!msg || window.getSelection()?.toString() || event.target.closest('input, textarea, [contenteditable]')) return;
    showContextMenu(event, [
      { label: 'Ответить', run: () => onreply(msg, 'reply') },
      { label: 'Переслать', run: () => onreply(msg, 'forward') },
      { separator: true },
      translation?.srcdoc
        ? { label: 'Показать оригинал', run: () => (translation = null) }
        : { label: 'Перевести письмо', disabled: !!translation?.loading, run: translate },
      { separator: true },
      { label: 'Копировать адрес отправителя', run: () => writeClipboard(msg.fromAddr) },
      { label: 'Копировать тему', run: () => writeClipboard(msg.subject) },
    ]);
  }

  function attachmentMenu(event, attachment) {
    showContextMenu(event, [
      { label: 'Сохранить вложение', disabled: saved[attachment.index] === 'saving', run: () => save(attachment.index) },
      { label: 'Копировать имя файла', run: () => writeClipboard(attachment.name) },
    ]);
  }

  const initial = (name) => name.trim().charAt(0).toUpperCase();
</script>

<article class="panel reader" oncontextmenu={menu}>
  {#if msg}
    <header>
      <h1>{msg.subject}</h1>
      <div class="sender">
        <div class="avatar">{initial(msg.fromName)}</div>
        <div class="who">
          <div class="name">
            <span class="clip" title={msg.fromName}>{msg.fromName}</span>
            {#if msg.auth === 'pass'}
              <span class="verified" title={$t("SPF, DKIM и DMARC пройдены")}><Icon name="shieldCheck" size={14} /></span>
            {/if}
          </div>
          <div class="addr clip" title={msg.fromAddr}>{msg.fromAddr}</div>
        </div>
        <div class="date">{msg.date}</div>
      </div>

      {#each msg.warnings as w}
        <div class="banner danger"><Icon name="alert" size={15} />{$t(w)}</div>
      {/each}
      {#if msg.blockedRemote && !msg.remoteShown}
        <div class="banner">
          <Icon name="imageOff" size={15} />
          <span class="text">{$t("Внешние картинки скрыты:")} {msg.blockedRemote}</span>
          <button class="chip" disabled={loadingRemote} onclick={showRemote}>
            {loadingRemote ? $t("Загрузка…") : $t("Показать")}
          </button>
          {#if msg.canTrust}
            <button class="chip" title={$t("Всегда показывать от {address}", { address: msg.fromAddr })} disabled={loadingRemote} onclick={() => trust(true)}>
              {$t("Всегда для {address}", { address: msg.fromAddr })}
            </button>
          {/if}
        </div>
      {:else if msg.remoteShown}
        <div class="banner">
          <Icon name="image" size={15} />
          <span class="text">
            {msg.trusted ? $t("Внешнее содержимое показывается: {address} в доверенных", { address: msg.fromAddr }) : $t("Внешнее содержимое показано")}
          </span>
          {#if msg.trusted}
            <button class="chip" onclick={() => trust(false)}>{$t("Больше не доверять")}</button>
          {:else if msg.canTrust}
            <button class="chip" title={$t("Всегда показывать от {address}", { address: msg.fromAddr })} onclick={() => trust(true)}>{$t("Всегда для {address}", { address: msg.fromAddr })}</button>
          {/if}
        </div>
      {/if}
      {#if translation}
        <div class="banner">
          <Icon name="globe" size={15} />
          <span class="text">
            {#if translation.loading}{$t("Перевожу…")}{:else}{$t("Переведено Google с языка: {language}", { language: translation.sourceLanguage ?? '?' })}{/if}
          </span>
          {#if !translation.loading}<button class="chip" onclick={() => (translation = null)}>{$t("Показать оригинал")}</button>{/if}
        </div>
      {/if}
      {#if translationError}
        <div class="banner danger"><Icon name="alert" size={15} /><span class="text">{translationError}</span></div>
      {/if}
    </header>

    {#if translation?.srcdoc}
      <!-- Перевод — тоже внешнее содержимое: экранирован и очищен ядром, CSP та же, ссылок нет — sandbox пустой. -->
      <iframe class="body" title={$t("Перевод письма")} sandbox="" referrerpolicy="no-referrer" srcdoc={withScheme(translation.srcdoc, $darkScheme)}></iframe>
    {:else}
    <!-- Без скриптов, форм и доступа к origin приложения. allow-popups нужен только чтобы клик по ссылке
         дошёл до ядра как запрос нового окна: ядро его всегда отклоняет и спрашивает подтверждение. -->
    <iframe class="body" title={$t("Содержимое письма")} sandbox="allow-popups" referrerpolicy="no-referrer" srcdoc={withScheme(msg.srcdoc, $darkScheme)}></iframe>
    {/if}

    {#if msg.attachments.length}
      <div class="attachments">
        {#each msg.attachments as a (a.index)}
          <div class="file" role="group" aria-label={a.name} oncontextmenu={(event) => attachmentMenu(event, a)} class:dangerous={a.dangerous} title={a.dangerous ? $t("Исполняемый файл — запускайте, только если доверяете отправителю") : a.name}>
            <Icon name={a.dangerous ? 'alert' : 'paperclip'} size={14} />
            <span class="fname">{a.name}</span>
            <span class="fsize">{size(a.size)}</span>
            <button onkeydown={(event) => { if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) attachmentMenu(event, a); }} class="save" disabled={saved[a.index] === 'saving'} onclick={() => save(a.index)} title={$t("Сохранить")}>
              {#if saved[a.index] === 'done'}✓{:else}<Icon name="download" size={14} />{/if}
            </button>
          </div>
        {/each}
      </div>
    {/if}

    <footer>
      <button class="action" onclick={() => onreply(msg, 'reply')}><Icon name="reply" />{$t("Ответить")}</button>
      <button class="action" onclick={() => onreply(msg, 'forward')}><Icon name="forward" />{$t("Переслать")}</button>
    </footer>
  {:else if error}
    <div class="placeholder">{$t("Не удалось открыть письмо:")} {$t(error)}</div>
  {:else if id == null}
    <div class="placeholder">{$t("Выберите письмо")}</div>
  {:else}
    <div class="placeholder">{$t("Загрузка…")}</div>
  {/if}
</article>

<style>
  .reader {
    display: flex;
    flex-direction: column;
    padding: 22px 24px 18px;
  }

  h1 {
    margin: 0 0 14px;
    font-size: 20px;
    font-weight: 500;
    user-select: text;
  }

  .sender {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .avatar {
    width: 38px;
    height: 38px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: rgba(var(--ink), 0.14);
    font-weight: 600;
  }

  .who {
    flex: 1;
    min-width: 0;
    user-select: text;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 15px;
  }
  .verified {
    flex: none;
    display: flex;
    color: var(--ok);
  }
  .clip {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .addr,
  .date {
    color: var(--muted);
    font-size: 13px;
  }
  .date {
    flex: none;
    white-space: nowrap;
  }

  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(var(--ink), 0.08);
    font-size: 13px;
  }
  .banner .text {
    flex: 1 1 160px;
    min-width: 0;
  }
  .chip {
    flex: 0 1 auto;
    min-width: 0;
    max-width: 100%;
    height: 26px;
    padding: 0 12px;
    border-radius: 13px;
    background: rgba(var(--ink), 0.14);
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chip:hover:not(:disabled) {
    background: rgba(var(--ink), 0.24);
  }
  .chip:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .banner.danger {
    background: rgba(255, 79, 120, 0.18);
    color: var(--danger-text);
  }

  .body {
    flex: 1;
    min-height: 0;
    width: calc(100% + var(--scrollbar-size));
    margin-top: 16px;
    border: 0;
    background: transparent;
  }

  .attachments {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-top: 12px;
  }
  .file {
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 100%;
    height: 32px;
    padding: 0 4px 0 12px;
    border-radius: 16px;
    background: rgba(var(--ink), 0.08);
    font-size: 13px;
  }
  .file.dangerous {
    background: rgba(255, 79, 120, 0.18);
    color: var(--danger-text);
  }
  .fname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fsize {
    flex: none;
    color: var(--muted);
  }
  .save {
    flex: none;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 50%;
  }
  .save:hover:not(:disabled) {
    background: rgba(var(--ink), 0.16);
  }

  footer {
    display: flex;
    gap: 10px;
    padding-top: 12px;
  }

  .action {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 38px;
    padding: 0 16px;
    border-radius: 9px;
    background: rgba(var(--ink), 0.12);
  }
  .action:hover {
    background: rgba(var(--ink), 0.2);
  }

  .placeholder {
    margin: auto;
    color: var(--muted);
  }
</style>
