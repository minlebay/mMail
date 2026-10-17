<script>
  import Icon from './Icon.svelte';
  import { getMessage } from './api.js';

  let { id } = $props();

  let msg = $state(null);
  let error = $state('');

  $effect(() => {
    const current = id;
    msg = null;
    error = '';
    if (current == null) return;
    let alive = true;
    getMessage(current)
      .then((m) => alive && (msg = m))
      .catch((e) => alive && (error = String(e)));
    return () => (alive = false);
  });

  const initial = (name) => name.trim().charAt(0).toUpperCase();
</script>

<article class="panel reader">
  {#if msg}
    <header>
      <h1>{msg.subject}</h1>
      <div class="sender">
        <div class="avatar">{initial(msg.fromName)}</div>
        <div class="who">
          <div class="name">
            {msg.fromName}
            {#if msg.auth === 'pass'}
              <span class="verified" title="SPF, DKIM и DMARC пройдены"><Icon name="shieldCheck" size={14} /></span>
            {/if}
          </div>
          <div class="addr">{msg.fromAddr}</div>
        </div>
        <div class="date">{msg.date}</div>
      </div>

      {#each msg.warnings as w}
        <div class="banner danger"><Icon name="alert" size={15} />{w}</div>
      {/each}
      {#if msg.blockedRemote}
        <div class="banner"><Icon name="imageOff" size={15} />Заблокировано внешних ресурсов: {msg.blockedRemote}</div>
      {/if}
    </header>

    <!-- Без скриптов, форм и доступа к origin приложения. allow-popups нужен только чтобы клик по ссылке
         дошёл до ядра как запрос нового окна: ядро его всегда отклоняет и спрашивает подтверждение. -->
    <iframe class="body" title="Содержимое письма" sandbox="allow-popups" referrerpolicy="no-referrer" srcdoc={msg.srcdoc}></iframe>

    <footer>
      <button class="action"><Icon name="reply" />Ответить</button>
      <button class="action"><Icon name="forward" />Переслать</button>
    </footer>
  {:else if error}
    <div class="placeholder">Не удалось открыть письмо: {error}</div>
  {:else if id == null}
    <div class="placeholder">Выберите письмо</div>
  {:else}
    <div class="placeholder">Загрузка…</div>
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
    background: rgba(255, 255, 255, 0.14);
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
    display: flex;
    color: var(--ok);
  }
  .addr,
  .date {
    color: var(--muted);
    font-size: 13px;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    padding: 8px 12px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.08);
    font-size: 13px;
  }
  .banner.danger {
    background: rgba(255, 79, 120, 0.18);
    color: #ffd0da;
  }

  .body {
    flex: 1;
    min-height: 0;
    width: 100%;
    margin-top: 16px;
    border: 0;
    background: transparent;
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
    background: rgba(255, 255, 255, 0.12);
  }
  .action:hover {
    background: rgba(255, 255, 255, 0.2);
  }

  .placeholder {
    margin: auto;
    color: var(--muted);
  }
</style>
