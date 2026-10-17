fn main() {
    // Явный список команд: UI может вызвать только то, что разрешено в capabilities/.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&["context_menu_items", "google_status", "google_connect", "google_disconnect", "calendar_list", "calendar_set_visible", "calendar_events", "calendar_save_event", "calendar_delete_event", "mark_message_read", "mark_folder_read", "set_message_flags", "trash_message", "translate_message", "translate_text", "write_clipboard", "list_folders", "list_messages", "get_message", "set_sender_trusted", "get_appearance", "set_appearance", "set_zoom", "save_attachment", "storage_info", "set_body_days", "clear_cache", "open_settings", "set_show_all_accounts", "set_notifications", "open_link", "sync_status", "list_accounts", "add_account", "remove_account",
    "unlock_store", "send_message", "open_compose", "compose_init", "pick_attachments", "clean_compose_html", "window_pins", "set_window_pin", "set_title_buttons"]),
    ))
    .expect("failed to run tauri-build");
}
