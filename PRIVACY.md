# Политика конфиденциальности mMail

mMail — настольный почтовый клиент с календарём, который работает только на компьютере пользователя.
У mMail нет своего сервера: никакие данные не отправляются разработчику и третьим лицам.

**Google Календарь.** Если пользователь подключает Google Календарь, mMail получает доступ к его
календарям через Google Calendar API (область `https://www.googleapis.com/auth/calendar`), чтобы
показывать, создавать, изменять и удалять события по действиям пользователя. События запрашиваются
напрямую у Google с компьютера пользователя и хранятся только в памяти программы, на диск не пишутся.
Ключи доступа хранятся в системном хранилище паролей пользователя (KWallet). Доступ можно отозвать
в mMail («Отключить») или в настройках аккаунта Google.

**Почта.** Письма загружаются с почтовых серверов пользователя (IMAP) и хранятся на его компьютере
в зашифрованном виде; пароли — только в KWallet.

**Перевод.** Текст письма отправляется в Google Translate только по явному действию пользователя
(«Перевести»).

Использование данных Google соответствует
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
включая требования Limited Use.

Вопросы: https://github.com/minlebay/mMail/issues

---

# mMail Privacy Policy

mMail is a desktop mail client with a calendar that runs only on the user's computer. mMail has no
server of its own: no data is sent to the developer or to third parties.

**Google Calendar.** When the user connects Google Calendar, mMail accesses their calendars through
the Google Calendar API (scope `https://www.googleapis.com/auth/calendar`) to display, create, edit
and delete events at the user's request. Events are requested directly from Google by the user's
computer and are kept in memory only, never written to disk. Access tokens are stored in the user's
system password store (KWallet). Access can be revoked in mMail ("Disconnect") or in the Google
Account settings.

**Mail.** Messages are downloaded from the user's mail servers (IMAP) and stored encrypted on the
user's computer; passwords are kept only in KWallet.

**Translation.** Message text is sent to Google Translate only on an explicit user action
("Translate").

mMail's use of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

Contact: https://github.com/minlebay/mMail/issues
