// Единственная точка общения UI с Rust-ядром. Сети и диска у UI нет.
import { invoke } from '@tauri-apps/api/core';

export const listFolders = () => invoke('list_folders');

export const syncStatus = () => invoke('sync_status');

// account: null — все ящики, '' — демо, иначе адрес ящика.
export const listMessages = (account, folder, query, offset, limit) =>
  invoke('list_messages', { account, folder, query, offset, limit });

export const getMessage = (id, showRemote = false) => invoke('get_message', { id, showRemote });

export const setSenderTrusted = (address, trusted) => invoke('set_sender_trusted', { address, trusted });

// Передаём только id запроса: сам адрес хранится в ядре.
export const openLink = (id) => invoke('open_link', { id });

export const getAppearance = () => invoke('get_appearance');

export const setAppearance = (theme, opacity, middleClickMinimize, zoom, pollSeconds) =>
  invoke('set_appearance', { theme, opacity, middleClickMinimize, zoom, pollSeconds });

export const setZoom = (scale) => invoke('set_zoom', { scale });

export const listAccounts = () => invoke('list_accounts');

export const addAccount = (email, password) => invoke('add_account', { email, password });

export const removeAccount = (email) => invoke('remove_account', { email });

export const sendMessage = (draft) => invoke('send_message', { draft });

export const openCompose = (prefill) => invoke('open_compose', { prefill });

export const composeInit = () => invoke('compose_init');

// Файлы выбираются в системном диалоге ядра; сюда приходят только id, имя и размер.
export const pickAttachments = () => invoke('pick_attachments');
