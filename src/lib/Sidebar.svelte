<script>
  import Icon from './Icon.svelte';

  // groups: [{ account: null | '' | email, title, folders: [{ id, name, unread }] }]
  let { groups, active, onpick, oncompose } = $props();

  const ICONS = { inbox: 'inbox', starred: 'star', sent: 'send', drafts: 'file', trash: 'trash' };
  const keyOf = (account) => account ?? '*';

  // Свёрнутые узлы дерева — удобство на этом компьютере, поэтому localStorage.
  let collapsed = $state(new Set(readCollapsed()));

  function readCollapsed() {
    try {
      return JSON.parse(localStorage.getItem('mmail.collapsed') ?? '[]');
    } catch {
      return [];
    }
  }

  function toggle(account) {
    const key = keyOf(account);
    const next = new Set(collapsed);
    next.has(key) ? next.delete(key) : next.add(key);
    collapsed = next;
    try {
      localStorage.setItem('mmail.collapsed', JSON.stringify([...next]));
    } catch {}
  }

  const inboxUnread = (group) => group.folders.find((f) => f.id === 'inbox')?.unread ?? 0;
</script>

<nav class="panel sidebar">
  <button class="compose" onclick={oncompose}><Icon name="pencil" />Написать</button>

  <div class="tree scroll-gutter">
    {#each groups as group (keyOf(group.account))}
      {@const open = !collapsed.has(keyOf(group.account))}
      <section>
        <button class="account" class:open onclick={() => toggle(group.account)} title={group.title}>
          <svg class="chevron" viewBox="0 0 12 12"><path d="M4.5 3l3 3-3 3" /></svg>
          <span class="title">{group.title}</span>
          {#if !open && inboxUnread(group)}<span class="count">{inboxUnread(group)}</span>{/if}
        </button>
        {#if open}
          <ul>
            {#each group.folders as f (f.id)}
              {@const isActive = active.account === group.account && active.folder === f.id}
              <li>
                <button class="folder" class:active={isActive} onclick={() => onpick(group.account, f.id)}>
                  <Icon name={ICONS[f.id]} />
                  <span class="name">{f.name}</span>
                  {#if f.id === 'inbox' && f.unread}<span class="count">{f.unread}</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  </div>
</nav>

<style>
  .sidebar {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .compose {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    height: 38px;
    border-radius: 9px;
    background: var(--accent-soft);
    font-weight: 500;
  }
  .compose:hover {
    filter: brightness(1.15);
  }

  .tree {
    --gutter: 10px;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .account {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 6px;
    border-radius: 7px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
    text-align: left;
  }
  .account:hover {
    background: rgba(255, 255, 255, 0.05);
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
    transition: transform 0.15s;
  }
  .account.open .chevron {
    transform: rotate(90deg);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  ul {
    list-style: none;
    margin: 2px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .folder {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 36px;
    padding: 0 12px;
    border-radius: 9px;
    text-align: left;
  }
  .folder:hover {
    background: rgba(255, 255, 255, 0.07);
  }
  .folder.active {
    background: var(--accent-soft);
  }

  .name {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .count {
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
</style>
