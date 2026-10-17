<script>
  import Icon from './Icon.svelte';

  let { folders, active, onpick } = $props();

  const ICONS = { inbox: 'inbox', starred: 'star', sent: 'send', drafts: 'file', trash: 'trash' };
</script>

<nav class="panel sidebar">
  <button class="compose"><Icon name="pencil" />Написать</button>
  <ul>
    {#each folders as f (f.id)}
      <li>
        <button class="folder" class:active={f.id === active} onclick={() => onpick(f.id)}>
          <Icon name={ICONS[f.id]} />
          <span class="name">{f.name}</span>
          {#if f.id === 'inbox' && f.unread}<span class="count">{f.unread}</span>{/if}
        </button>
      </li>
    {/each}
  </ul>
</nav>

<style>
  .sidebar {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .compose {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    height: 38px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.2);
    font-weight: 500;
  }
  .compose:hover {
    background: rgba(255, 255, 255, 0.26);
  }

  ul {
    list-style: none;
    margin: 0;
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
    height: 38px;
    padding: 0 12px;
    border-radius: 9px;
    text-align: left;
  }
  .folder:hover {
    background: rgba(255, 255, 255, 0.07);
  }
  .folder.active {
    background: var(--glass-strong);
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
