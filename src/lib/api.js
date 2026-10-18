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

// a: { language, theme, scheme, opacity, middleClickMinimize, zoom, pollSeconds, trayMode, trayIcon, closeToTray }
export const setAppearance = (a) => invoke('set_appearance', a);

// Шифрованный кэш писем: { bytes, bodyDays } (bodyDays null — хранить всё).
export const storageInfo = () => invoke('storage_info');
export const setBodyDays = (days) => invoke('set_body_days', { days });
export const clearCache = () => invoke('clear_cache');

export const setShowAllAccounts = (show) => invoke('set_show_all_accounts', { show });
// Google Календарь (OAuth: клиент из Google Cloud Console, вход — в браузере).
export const googleStatus = () => invoke('google_status');
export const googleConnect = (clientId, clientSecret) => invoke('google_connect', { clientId, clientSecret });
export const googleDisconnect = () => invoke('google_disconnect');
export const calendarList = () => invoke('calendar_list');
export const calendarSetVisible = (id, visible) => invoke('calendar_set_visible', { id, visible });
// from/to — RFC 3339; события: { id, calendarId, title, start, end, allDay, location, description, recurring }
export const calendarEvents = (calendars, from, to) => invoke('calendar_events', { calendars, from, to });
export const calendarSaveEvent = (calendarId, eventId, draft) => invoke('calendar_save_event', { calendarId, eventId, draft });
export const calendarDeleteEvent = (calendarId, eventId) => invoke('calendar_delete_event', { calendarId, eventId });
export const setNotifications = (mail, events) => invoke('set_notifications', { mail, events });
export const setCalendarView = (view) => invoke('set_calendar_view', { view });
export const setCalendarFullTitles = (full) => invoke('set_calendar_full_titles', { full });
export const setTitleButtons = (windowButtons, pinButtons) => invoke('set_title_buttons', { windowButtons, pinButtons });
// Поверх других окон / на всех рабочих столах — для окна, которое вызывает.
export const windowPins = () => invoke('window_pins');
export const setWindowPin = (pin, on) => invoke('set_window_pin', { pin, on });
export const openSettings = () => invoke('open_settings');

export const setZoom = (scale) => invoke('set_zoom', { scale });

export const listAccounts = () => invoke('list_accounts');

export const addAccount = (email, password) => invoke('add_account', { email, password });

export const removeAccount = (email) => invoke('remove_account', { email });

export const sendMessage = (draft) => invoke('send_message', { draft });

export const openCompose = (prefill) => invoke('open_compose', { prefill });

export const composeInit = () => invoke('compose_init');

// Файлы выбираются в системном диалоге ядра; сюда приходят только id, имя и размер.
export const pickAttachments = () => invoke('pick_attachments');
export const cleanComposeHtml = (html) => invoke('clean_compose_html', { html });

// Путь выбирается в системном диалоге ядра; возвращает false, если сохранение отменили.
export const saveAttachment = (id, index) => invoke('save_attachment', { id, index });

export const writeClipboard = (text) => invoke('write_clipboard', { text });

export const markMessageRead = (id) => invoke('mark_message_read', { id });
export const markFolderRead = (account, folder) => invoke('mark_folder_read', { account, folder });
export const trashMessage = (id) => invoke('trash_message', { id });
// flags: { unread?: boolean, starred?: boolean } — «важное» это флаг, письмо остаётся в своей папке.
export const setMessageFlags = (id, flags) => invoke('set_message_flags', { id, unread: flags.unread ?? null, starred: flags.starred ?? null });
// Перевод (Google, по явному действию): { text, srcdoc?, sourceLanguage }. srcdoc — для песочницы письма.
export const translateMessage = (id) => invoke('translate_message', { id });
export const translateText = (text) => invoke('translate_text', { text });
