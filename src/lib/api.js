// Единственная точка общения UI с Rust-ядром. Сети и диска у UI нет.
import { invoke } from '@tauri-apps/api/core';

export const listFolders = () => invoke('list_folders');

export const syncStatus = () => invoke('sync_status');
export const unlockStore = () => invoke('unlock_store');

// account: null — все ящики, '' — демо, иначе адрес ящика.
export const listMessages = (account, folder, query, offset, limit) =>
  invoke('list_messages', { account, folder, query, offset, limit });

export const getMessage = (id, showRemote = false) => invoke('get_message', { id, showRemote });

export const setSenderTrusted = (address, trusted) => invoke('set_sender_trusted', { address, trusted });

// Передаём только id запроса: сам адрес хранится в ядре.
export const openLink = (id) => invoke('open_link', { id });

export const getAppearance = () => invoke('get_appearance');

// a: { theme, opacity, middleClickMinimize, zoom, pollSeconds, trayMode, closeToTray }
export const setAppearance = (a) => invoke('set_appearance', a);

// Шифрованный кэш писем: { bytes, bodyDays } (bodyDays null — хранить всё).
export const storageInfo = () => invoke('storage_info');
export const setBodyDays = (days) => invoke('set_body_days', { days });
export const clearCache = () => invoke('clear_cache');

export const setZoom = (scale) => invoke('set_zoom', { scale });

export const listAccounts = () => invoke('list_accounts');

export const addAccount = (email, password) => invoke('add_account', { email, password });

export const removeAccount = (email) => invoke('remove_account', { email });

export const sendMessage = (draft) => invoke('send_message', { draft });

export const openCompose = (prefill) => invoke('open_compose', { prefill });

export const composeInit = () => invoke('compose_init');

// Файлы выбираются в системном диалоге ядра; сюда приходят только id, имя и размер.
export const pickAttachments = () => invoke('pick_attachments');

// Путь выбирается в системном диалоге ядра; возвращает false, если сохранение отменили.
export const saveAttachment = (id, index) => invoke('save_attachment', { id, index });
