//! Keep WebKit editing actions, without navigation or bypassing link confirmation.
use webkit2gtk::{ContextMenuAction, ContextMenuExt, ContextMenuItem, ContextMenuItemExt, WebViewExt};

fn label(action: ContextMenuAction) -> Option<&'static str> {
    use ContextMenuAction::*;
    Some(match action {
        Copy => "Копировать",
        Cut => "Вырезать",
        Paste => "Вставить",
        PasteAsPlainText => "Вставить без форматирования",
        Delete => "Удалить",
        SelectAll => "Выделить всё",
        CopyLinkToClipboard => "Копировать адрес ссылки",
        CopyImageToClipboard => "Копировать изображение",
        CopyImageUrlToClipboard => "Копировать адрес изображения",
        _ => return None,
    })
}

pub fn install(view: &webkit2gtk::WebView, app: tauri::AppHandle) {
    view.connect_context_menu(move |_, menu, _, _| {
        let items = menu.items();
        menu.remove_all();
        let mut count = 0;
        for item in items {
            if let Some(key) = label(item.stock_action()) {
                // Preserve enabled/disabled state and the original editing target.
                if let Some(action) = item.gaction() {
                    menu.append(&ContextMenuItem::from_gaction(&action, &crate::i18n::text(&app, key), None));
                    count += 1;
                }
            }
        }
        count == 0
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_editing_and_copy_actions_are_allowed() {
        assert!(label(ContextMenuAction::Copy).is_some());
        assert!(label(ContextMenuAction::Paste).is_some());
        assert!(label(ContextMenuAction::CopyLinkToClipboard).is_some());
        for action in [ContextMenuAction::OpenLink, ContextMenuAction::OpenLinkInNewWindow,
            ContextMenuAction::DownloadLinkToDisk, ContextMenuAction::OpenFrameInNewWindow,
            ContextMenuAction::Reload, ContextMenuAction::GoBack, ContextMenuAction::InspectElement] {
            assert!(label(action).is_none());
        }
    }
}
