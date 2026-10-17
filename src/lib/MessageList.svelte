<script>
  import { t } from './i18n.js';
  import { untrack } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import Icon from './Icon.svelte';
  import { showContextMenu } from './contextMenu.js';
  import { listMessages, writeClipboard, setMessageFlags, trashMessage, messageTranslationText, translateText } from './api.js';

  let { account, folder, version, selectedId, onselect, onreply, onerror } = $props();

  // Виртуальный список: в DOM только видимые строки, данные подгружаются страницами из ядра.
  const ROW = 64;
  const PAGE = 100;
  const OVERSCAN = 6;

  let query = $state('');
  let debounced = $state('');
  let total = $state(0);
  let loaded = $state(false);
  let scrollTop = $state(0);
  let height = $state(0);
  let selIndex = -1;
  let viewport;

  const pages = new SvelteMap();
  const pending = new Set();
  // Страницы, которые остались от прошлой версии данных: показываем, пока не придут свежие.
  const stale = new Set();
  let generation = 0;

  $effect(() => {
    const q = query.trim();
    const t = setTimeout(() => (debounced = q), 150);
    return () => clearTimeout(t);
  });

  $effect(() => {
    const scope = { account, folder };
    const q = debounced;
    untrack(() => reset(scope, q));
  });

  // Новая порция писем от синхронизации: перечитываем видимое, прокрутку и выделение не трогаем.
  $effect(() => {
    if (version) untrack(refresh);
  });

  let first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  let last = $derived(Math.min(total, Math.ceil((scrollTop + height) / ROW) + OVERSCAN));

  $effect(() => {
    if (last <= first) return;
    const from = Math.floor(first / PAGE);
    const to = Math.floor((last - 1) / PAGE);
    untrack(() => {
      for (let p = from; p <= to; p++) load(p);
    });
  });

  let rows = $derived.by(() => {
    const out = [];
    for (let i = first; i < last; i++) out.push({ i, item: itemAt(i) });
    return out;
  });

  function itemAt(i) {
    return pages.get(Math.floor(i / PAGE))?.[i % PAGE];
  }

  function reset(scope, q) {
    generation++;
    pages.clear();
    pending.clear();
    stale.clear();
    total = 0;
    loaded = false;
    selIndex = -1;
    scrollTop = 0;
    if (viewport) viewport.scrollTop = 0;
    load(0, scope, q);
  }

  function refresh() {
    generation++;
    pending.clear();
    for (const p of pages.keys()) stale.add(p);
    const from = Math.floor(first / PAGE);
    const to = Math.floor(Math.max(first, last - 1) / PAGE);
    for (let p = from; p <= to; p++) load(p);
  }

  async function load(p, scope = { account, folder }, q = debounced) {
    if ((pages.has(p) && !stale.has(p)) || pending.has(p)) return;
    const gen = generation;
    pending.add(p);
    try {
      const res = await listMessages(scope.account, scope.folder, q, p * PAGE, PAGE);
      if (gen !== generation) return;
      total = res.total;
      loaded = true;
      stale.delete(p);
      pages.set(p, res.items);
      const selected = res.items.findIndex((m) => m.id === selectedId);
      if (selected >= 0) selIndex = p * PAGE + selected;
      if (p === 0 && selectedId == null && res.items.length) pick(0, res.items[0]);
    } finally {
      if (gen === generation) pending.delete(p);
    }
  }

  function pick(i, item) {
    selIndex = i;
    onselect(item.id);
  }

  function menu(event, item) {
    if (!item) return;
    showContextMenu(event, [
      { label: 'Открыть письмо', run: () => onselect(item.id) },
      { label: 'Ответить', run: () => onreply(item, 'reply') },
      { label: 'Переслать', run: () => onreply(item, 'forward') },
      { separator: true },
      item.unread
        ? { label: 'Отметить прочитанным', run: () => setMessageFlags(item.id, { unread: false }) }
        : { label: 'Отметить непрочитанным', run: () => setMessageFlags(item.id, { unread: true }) },
      item.starred
        ? { label: 'Снять отметку «Важное»', run: () => setMessageFlags(item.id, { starred: false }) }
        : { label: 'Отметить важным', run: () => setMessageFlags(item.id, { starred: true }) },
      { label: 'Перевести письмо', run: () => messageTranslationText(item.id).then(translateText) },
      { separator: true },
      { label: 'Копировать адрес отправителя', run: () => writeClipboard(item.fromAddr) },
      { label: 'Копировать тему', run: () => writeClipboard(item.subject) },
      ...(item.inTrash ? [] : [{ separator: true }, { label: 'Удалить', danger: true, run: () => trashMessage(item.id) }]),
    ]);
  }

  function onkey(e) {
    if (e.key === 'Delete') {
      const item = itemAt(selIndex);
      if (item && !item.inTrash) trashMessage(item.id).catch((err) => onerror?.(String(err)));
      return;
    }
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      const item = itemAt(selIndex);
      if (item) menu(e, item);
      return;
    }
    const step = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const next = Math.min(total - 1, Math.max(0, selIndex + step));
    const item = itemAt(next);
    if (!item) return;
    pick(next, item);
    const top = next * ROW;
    if (top < viewport.scrollTop) viewport.scrollTop = top;
    else if (top + ROW > viewport.scrollTop + height) viewport.scrollTop = top + ROW - height;
  }
</script>

<section class="panel list">
  <label class="search">
    <Icon name="search" size={15} />
    <input placeholder={$t("Поиск")} bind:value={query} spellcheck="false" />
  </label>

  <div
    class="viewport scroll-gutter"
    role="listbox"
    aria-label={$t("Письма")}
    tabindex="0"
    bind:this={viewport}
    bind:clientHeight={height}
    onscroll={() => (scrollTop = viewport.scrollTop)}
    onkeydown={onkey}
  >
    <div class="spacer" style:height="{total * ROW}px">
      {#each rows as { i, item } (i)}
        <!-- Клавиатура обрабатывается на listbox (стрелки), поэтому у строк только клик. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          class="row"
          role="option"
          tabindex="-1"
          aria-selected={item?.id === selectedId}
          class:selected={item && item.id === selectedId}
          class:unread={item?.unread}
          style:transform="translateY({i * ROW}px)"
          onclick={() => item && pick(i, item)}
          oncontextmenu={(event) => menu(event, item)}
        >
          {#if item}
            <div class="line">
              <span class="from" class:muted={item.unavailable}>{item.unavailable ? $t('Заголовок недоступен') : item.fromName}</span>
              {#if item.starred}<span class="star"><Icon name="star" size={12} /></span>{/if}
              <span class="date">{item.date}</span>
            </div>
            <div class="line">
              <span class="subject" class:muted={item.unavailable} title={item.unavailable ? $t('Сервер сейчас не отдаёт это письмо; mMail попробует ещё раз позже') : undefined}>{item.unavailable ? $t('Сервер не отдал письмо') : item.subject}</span>
              {#if item.hasAttachments}<span class="clip" title={$t("Есть вложения")}><Icon name="paperclip" size={13} /></span>{/if}
            </div>
          {:else}
            <div class="skeleton"></div>
            <div class="skeleton short"></div>
          {/if}
        </div>
      {/each}
    </div>
    {#if loaded && total === 0}<p class="empty">{$t("Писем нет")}</p>{/if}
  </div>
</section>

<style>
  .from.muted,
  .subject.muted {
    color: var(--muted);
    font-style: italic;
  }

  .list {
    display: flex;
    flex-direction: column;
    padding: var(--panel-inset);
    gap: 10px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 12px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.08);
    color: var(--muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
  }
  .search input::placeholder {
    color: var(--muted);
  }

  .viewport {
    flex: 1;
    min-height: 0;
    outline: none;
  }

  .spacer {
    position: relative;
  }

  .row {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 64px;
    padding: 10px 14px;
    border-radius: 9px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    cursor: default;
  }
  .row:hover {
    background: rgba(255, 255, 255, 0.05);
  }
  .row.selected {
    background: var(--accent-soft);
  }

  .line {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }

  .from,
  .subject {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .from {
    flex: 1;
    font-weight: 500;
  }
  .unread .from {
    font-weight: 700;
  }
  .subject {
    color: var(--muted);
  }
  .unread .subject {
    color: var(--text);
  }

  .subject {
    flex: 1;
    min-width: 0;
  }
  .clip {
    flex: none;
    display: flex;
    color: var(--muted);
  }

  .star {
    color: #ffc95c;
    align-self: center;
    display: flex;
  }

  .date {
    flex: none;
    font-size: 13px;
    color: var(--muted);
  }

  .skeleton {
    height: 10px;
    width: 70%;
    border-radius: 5px;
    background: rgba(255, 255, 255, 0.08);
  }
  .skeleton.short {
    width: 45%;
    margin-top: 6px;
  }

  .empty {
    text-align: center;
    color: var(--muted);
  }
</style>
