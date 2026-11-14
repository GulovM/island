# Privacy Policy — Kite Island

Last updated: 2026-11-14

Kite Island is a desktop program for Windows 10 and Windows 11 that shows a dynamic island at the top of the screen. It displays quick controls, notifications, calendar, weather, timer, system status and media playback. This policy explains which data the program uses, where it is stored, and how it is shared.

Contact: gulovmuhsin121@gmail.com

## 1. Summary

- Kite Island runs on your computer. It has no server that receives your personal data.
- The developer does not collect, sell, rent, or share your personal data.
- The program does not use your data for advertising or any commercial purpose.
- Google account data (Google Calendar) is used only to show your events inside Kite Island on your computer and, when you explicitly enable "Add to Google Calendar" for an event you create or edit, to create, update, or delete that event in your Google Calendar.

## 2. Data we request

| Source | Data | Why |
|---|---|---|
| Google (OAuth 2.0 + PKCE, optional) | Access to your Google Calendar (`calendar.readonly` and `calendar.events` scopes) via `accounts.google.com`, `oauth2.googleapis.com`, Calendar API `www.googleapis.com/calendar/v3/`, and CalDAV endpoint `apidata.googleusercontent.com/caldav/v2/`: calendar list and event titles, times and locations, plus creating/updating/deleting events that you choose to save to Google Calendar | To display your upcoming events in Kite Island and sync events you explicitly write back |
| Apple iCloud (CalDAV, optional) | Your calendars and events via `caldav.icloud.com` (and regional `p*-caldav.icloud.com` servers), using your Apple ID and an app-specific password you create (including creating/updating/deleting events you choose to save to iCloud) | To display your upcoming events in Kite Island and sync events you explicitly write back |
| Calendar links (optional) | Calendar feeds in ICS format that you add yourself | To show events from other calendar services |
| Your location, approximated by IP address | Public IP address sent to IP-geolocation services | To find the city for the weather widget |
| Weather services | City coordinates | To show the weather forecast |
| GitHub | Request for the latest release at `api.github.com/repos/GulovM/island/releases/latest` (IP address, program version, and reading the release `body` changelog notes) | To check for program updates and display "What's new" release notes |
| Speed test service | Test data transfer | To run the speed test you start yourself |

On your computer, Kite Island also reads the media information (`SMTC` / audio meter) that Windows exposes for the current media session and, if you grant access in Windows, the Windows system notifications (`UserNotificationListener`) so you can view them in the Notification Center and popup capsules. Notification contents, app rules (`notif_rules.txt`) and read state (`notif_read.txt`) stay strictly local on your device and are never sent over the network.

## 3. How data is used

- Calendar events are used only to draw the calendar, the upcoming event notification, the event list, and (when you opt in per event) to write or delete that event in your connected Google or iCloud calendar.
- Google OAuth 2.0 uses a public desktop `client_id` / `client_secret` pair with PKCE (`S256`) and a temporary local loopback listener (`127.0.0.1`).
- Data from Google APIs is used only to provide the features described above. It is not used to develop advertising, to build user profiles, or to train AI models.
- Application use of information received from Google APIs adheres to the Google API Services User Data Policy, including the Limited Use requirements.

## 4. Where data is stored

- Data is stored only on your computer, in the Kite Island data folder (`%APPDATA%\island`) under your Windows user profile.
- OAuth tokens and Apple app-specific passwords (`%APPDATA%\island\cal_accounts.bin`), clipboard history/Quick Access (`clipboard.txt`, `quick.txt`), and calendar links and event caches (`%APPDATA%\island\calendar_cache_*.ics`) are encrypted with Windows DPAPI (`CryptProtectData`). A protected file cannot be opened by another Windows account or on another computer.
- Calendar event caches and pending calendar sync items (`events.txt`, `events_sync.txt`), as well as notification preferences (`notif_rules.txt`, `notif_read.txt`) and app icon caches, are stored locally so features work offline and recover cleanly.
- The developer does not receive copies of this data.

## 5. Sharing with third parties

Kite Island sends data only to the services listed in section 2, and only for the features you use. Your data is not sold, shared or transferred to any other third party. Google and Apple receive requests from your computer exactly as they would from any other calendar app.

## 6. Your choices and deletion

- You choose whether to connect Google, Apple iCloud, any calendar link, or Windows notification access. You can disconnect any calendar account in the program settings (disconnecting deletes stored tokens, passwords and caches from your computer) and disable Windows notification access in Windows Privacy settings or inside Kite Island.
- You can also remove access at any time at https://myaccount.google.com/permissions (Google) or in your Apple ID settings (Apple).
- To delete all Kite Island data, remove `%APPDATA%\island` under your Windows user profile and uninstall the program.

## 7. Children

Kite Island is not directed to children under 13 and does not knowingly collect data from them.

## 8. Changes to this policy

This policy may change. The date at the top shows the last update. Changes are published in this file in the project repository.

---

# Политика конфиденциальности — Kite Island (русская версия)

Дата обновления: 2026-11-14

Kite Island — программа для Windows 10 и Windows 11, которая показывает «остров» (динамическое окно) вверху экрана: быстрые переключатели, уведомления, календарь, погоду, таймер, состояние системы и воспроизведение музыки. Этот документ описывает, какие данные использует программа, где они хранятся и кому передаются.

Контакт: gulovmuhsin121@gmail.com

## 1. Кратко

- Kite Island работает на вашем компьютере. Нет сервера, который получает ваши персональные данные.
- Разработчик не собирает, не продаёт, не сдаёт в аренду и не передаёт персональные данные.
- Данные не используются для рекламы или других коммерческих целей.
- Данные аккаунта Google (Google Календарь) используются только для показа ваших событий внутри Kite Island на вашем компьютере и — если вы сами включите переключатель «Записать в …» при создании или правке события — для записи, обновления или удаления этого события в вашем Google Календаре.

## 2. Какие данные запрашиваются

| Источник | Данные | Зачем |
|---|---|---|
| Google (OAuth 2.0 + PKCE, по желанию) | Доступ к Google Календарю (области `https://www.googleapis.com/auth/calendar.readonly` и `https://www.googleapis.com/auth/calendar.events`) через `accounts.google.com`, `oauth2.googleapis.com`, Calendar API `www.googleapis.com/calendar/v3/` и CalDAV-сервер `apidata.googleusercontent.com/caldav/v2/`: список календарей, названия, время и места событий, а также запись/правка/удаление событий, которые вы сами сохраняете в Google Календарь | Показ ближайших событий в Kite Island и синхронизация событий, которые вы записываете в календарь |
| Apple iCloud (CalDAV, по желанию) | Календари и события по протоколу CalDAV через `caldav.icloud.com` (и региональные серверы `p*-caldav.icloud.com`), вход по Apple ID и паролю приложения, который вы создаёте сами (включая запись/правку/удаление событий по вашему выбору) | Показ ближайших событий в Kite Island и синхронизация событий, которые вы записываете в календарь |
| Ссылки на календари (по желанию) | Ленты календарей в формате ICS, которые вы добавляете сами | Показ событий из других календарных сервисов |
| Местоположение по IP-адресу | Публичный IP-адрес, отправляемый сервисам определения местоположения | Определение города для виджета погоды |
| Сервисы погоды | Координаты города | Прогноз погоды |
| GitHub | Запрос последней версии программы к `api.github.com/repos/GulovM/island/releases/latest` (IP-адрес, версия программы и чтение поля `body` со списком изменений) | Проверка обновлений и показ раздела «Что нового» |
| Сервис проверки скорости | Тестовая передача данных | Тест скорости, который вы запускаете сами |

На вашем компьютере Kite Island также читает сведения о воспроизведении, которые Windows отдаёт для текущей медиасессии (`SMTC` и уровень звука для визуализации волн), а при вашем разрешении в Windows — системные уведомления (`UserNotificationListener`) для показа во вкладке уведомлений и всплывающих капсулах. Содержимое уведомлений, правила по приложениям (`notif_rules.txt`) и отметки о прочтении (`notif_read.txt`) остаются исключительно на вашем компьютере и никогда не передаются по сети.

## 3. Как используются данные

- События календаря используются только для показа календаря, уведомлений о ближайшем событии, списка событий и (при явном выборе переключателя на событии) записи или удаления этого события в подключённом календаре Google или iCloud.
- Для авторизации Google OAuth 2.0 используются публичные ключи настольного приложения (`client_id` / `client_secret`) вместе с механизмом PKCE (`S256`) и временным локальным сокетом (`127.0.0.1`).
- Данные из API Google используются только для работы описанных функций. Они не используются для разработки рекламы, построения профилей пользователей и обучения моделей ИИ.
- Использование полученных от API Google данных соответствует Политике пользовательских данных сервисов API Google, включая требования Limited Use.

## 4. Где хранятся данные

- Данные хранятся только на вашем компьютере, в папке данных Kite Island (`%APPDATA%\island`) в профиле пользователя Windows.
- Токены OAuth и пароли приложений Apple (`%APPDATA%\island\cal_accounts.bin`), история буфера и «Быстрый доступ» (`clipboard.txt`, `quick.txt`), а также ссылки на календари и кэш событий (`%APPDATA%\island\calendar_cache_*.ics`) шифруются через Windows DPAPI (`CryptProtectData`). Защищённый файл не открывается другой учётной записью Windows и на другом компьютере.
- Кэш событий, очередь отложенной записи в календарь (`events.txt`, `events_sync.txt`), правила уведомлений (`notif_rules.txt`, `notif_read.txt`) и кэш иконок приложений также хранятся локально на вашем компьютере.
- Разработчик не получает копий этих данных.

## 5. Передача третьим лицам

Kite Island отправляет данные только сервисам из раздела 2 и только для тех функций, которыми вы пользуетесь. Данные не продаются, не передаются и не сдаются другим третьим лицам. Google и Apple получают запросы с вашего компьютера так же, как от любого другого календарного приложения.

## 6. Ваш выбор и удаление данных

- Вы сами решаете, подключать ли Google, Apple iCloud, ссылку на календарь или доступ к уведомлениям Windows. Любой аккаунт календаря можно отключить в настройках программы (при отключении токены, пароли и кэш удаляются с вашего компьютера), а доступ к уведомлениям — в параметрах конфиденциальности Windows.
- Доступ можно отозвать в любой момент: https://myaccount.google.com/permissions (Google) или в настройках Apple ID (Apple).
- Чтобы удалить все данные Kite Island, удалите папку `%APPDATA%\island` в профиле пользователя Windows и удалите программу.

## 7. Дети

Kite Island не предназначен для детей младше 13 лет и сознательно не собирает их данные.

## 8. Изменения политики

Политика может меняться. Дата вверху показывает последнее обновление. Изменения публикуются в этом файле в репозитории проекта.
