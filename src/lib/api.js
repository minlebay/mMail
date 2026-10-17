// Единственная точка общения UI с Rust-ядром. Сети и диска у UI нет.
import { invoke } from '@tauri-apps/api/core';

export const listFolders = () => invoke('list_folders');

export const syncStatus = () => invoke('sync_status');

export const listMessages = (folder, query, offset, limit) =>
  invoke('list_messages', { folder, query, offset, limit });

export const getMessage = (id, showRemote = false) => invoke('get_message', { id, showRemote });

export const setSenderTrusted = (address, trusted) => invoke('set_sender_trusted', { address, trusted });

// Передаём только id запроса: сам адрес хранится в ядре.
export const openLink = (id) => invoke('open_link', { id });
