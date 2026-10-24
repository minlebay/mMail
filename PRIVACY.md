# Политика конфиденциальности mMail

mMail — настольный почтовый клиент с календарём, который работает только на компьютере пользователя.
У mMail нет своего сервера: разработчик не получает данные пользователя. Данные отправляются
третьим лицам только в описанных ниже случаях (Google Календарь, Google Translate и внешние
картинки из писем).

**Google Календарь.** Если пользователь подключает Google Календарь, mMail получает доступ к его
календарям через Google Calendar API (область `https://www.googleapis.com/auth/calendar`), чтобы
показывать, создавать, изменять и удалять события по действиям пользователя. События запрашиваются
напрямую у Google с компьютера пользователя и хранятся только в памяти программы, на диск не пишутся.
Ключи доступа хранятся в системном хранилище паролей пользователя (KWallet). Доступ можно отозвать
в mMail («Отключить») или в настройках аккаунта Google.

**Почта.** Письма загружаются с почтовых серверов пользователя (IMAP) и хранятся на его компьютере
в зашифрованном виде; пароли — только в KWallet.

**Внешние картинки.** Пока пользователь не нажал «Показать» (и пока адрес не добавлен в доверенные
с подтверждённой проверкой DMARC), картинки из писем не загружаются. После показа mMail скачивает их
напрямую с серверов отправителя: эти серверы видят IP-адрес и время запроса. Cookies и Referer не
отправляются, доступ к локальной сети блокируется.

**Перевод.** Текст письма отправляется в Google Translate только по явному действию пользователя
(«Перевести»).

**Командная строка.** Команды `mmail accounts`, `folders`, `list`, `search` и `read` открывают кэш
писем только на чтение и не запускают графическое окно. Любой процесс того же пользователя может
запустить эти команды; доступ к ключу кэша регулирует KWallet. Не запускайте недоверенные программы
от имени пользователя, который читает почту в mMail.

Использование данных Google соответствует
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
включая требования Limited Use.

Вопросы: https://github.com/minlebay/mMail/issues

---

# mMail Privacy Policy

mMail is a desktop mail client with a calendar that runs only on the user's computer. mMail has no
server of its own: the developer does not receive the user's data. Data is sent to third parties only
in the cases described below (Google Calendar, Google Translate and external images in messages).

**Google Calendar.** When the user connects Google Calendar, mMail accesses their calendars through
the Google Calendar API (scope `https://www.googleapis.com/auth/calendar`) to display, create, edit
and delete events at the user's request. Events are requested directly from Google by the user's
computer and are kept in memory only, never written to disk. Access tokens are stored in the user's
system password store (KWallet). Access can be revoked in mMail ("Disconnect") or in the Google
Account settings.

**Mail.** Messages are downloaded from the user's mail servers (IMAP) and stored encrypted on the
user's computer; passwords are kept only in KWallet.

**External images.** Until the user clicks "Show" (and unless the sender is trusted with a verified
DMARC result), images in messages are not loaded. After that, mMail downloads them directly from the
sender's servers: those servers can see the IP address and the time of the request. No cookies or
Referer are sent, and access to the local network is blocked.

**Translation.** Message text is sent to Google Translate only on an explicit user action
("Translate").

**Command line.** `mmail accounts`, `folders`, `list`, `search` and `read` open the message cache
read-only and do not start the GUI. Any process running as the same user can run these commands;
access to the cache key is controlled by KWallet. Do not run untrusted programs as the user who reads
mail in mMail.

mMail's use of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

Contact: https://github.com/minlebay/mMail/issues
