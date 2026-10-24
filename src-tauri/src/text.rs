//! Безопасный вывод чужого текста: письма, темы, имена отправителей и ответы серверов.
//!
//! Управляющие символы в терминале — это не «просто мусор»: ESC открывает ANSI/OSC-
//! последовательности (смена заголовка, гиперссылки, запись в буфер обмена), а bidi-контролы
//! визуально переставляют адреса и домены. Поэтому весь текст из письма или от сервера
//! проходит через [`console_safe`] перед печатью в stdout/stderr и через [`display_safe`]
//! перед показом в интерфейсе.

use std::borrow::Cow;

/// Символы, меняющие направление письма: ими можно показать `evil.example` как `example.evil`.
fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Текст для терминала: сохраняем переводы строк и табуляцию, всё остальное управляющее и
/// bidi заменяем на видимый символ. Так письмо не может выполнить escape-последовательность.
pub fn console_safe(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|c| (c.is_control() && c != '\n' && c != '\t') || is_bidi(c))
    {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| {
                if c == '\n' || c == '\t' {
                    c
                } else if c.is_control() || is_bidi(c) {
                    '\u{fffd}'
                } else {
                    c
                }
            })
            .collect(),
    )
}

/// Текст для интерфейса: управляющие и bidi-символы заменяются, обычный Unicode (кириллица,
/// иероглифы, эмодзи) сохраняется. Переводы строк в именах и темах тоже заменяются — в списке
/// и шапке письма они не нужны.
pub fn display_safe(text: &str) -> Cow<'_, str> {
    if !text.chars().any(|c| c.is_control() || is_bidi(c)) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| {
                if c.is_control() || is_bidi(c) {
                    '\u{fffd}'
                } else {
                    c
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_output_cannot_inject_control_sequences() {
        let dirty = "тема\u{1b}]0;подмена\u{7}\u{1b}[2J\u{202e}evil";
        let clean = console_safe(dirty);
        for bad in ['\u{1b}', '\u{7}', '\u{202e}'] {
            assert!(!clean.contains(bad), "{bad:?} остался в {clean:?}");
        }
        // Обычный текст и переносы строк не пострадали.
        assert_eq!(console_safe("строка 1\nстрока 2"), "строка 1\nстрока 2");
        assert!(clean.contains("тема") && clean.contains("evil"));
    }

    #[test]
    fn display_output_drops_bidi_but_keeps_normal_unicode() {
        let dirty = "Иван\u{202e} <a@b.example>\u{200f}";
        let clean = display_safe(dirty);
        assert!(!clean.contains('\u{202e}') && !clean.contains('\u{200f}'));
        assert_eq!(display_safe("Привет, мир"), "Привет, мир");
        assert_eq!(display_safe("日本語"), "日本語");
        assert_eq!(display_safe("a\nb"), "a\u{fffd}b");
    }
}
