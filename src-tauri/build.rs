fn main() {
    // Явный список команд: UI может вызвать только то, что разрешено в capabilities/.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&["list_folders", "list_messages", "get_message", "set_sender_trusted", "get_appearance", "set_appearance", "set_zoom", "open_link", "sync_status", "list_accounts", "add_account", "remove_account", "send_message", "open_compose", "compose_init", "pick_attachments"]),
    ))
    .expect("failed to run tauri-build");
}
