# Privacy Policy — Island

Last updated: 2026-11-11

Island is a desktop program for Windows 10 and Windows 11 that shows a dynamic island at the top of the screen. It shows the calendar, weather, timer, system status and media playback. This policy explains which data the program uses, where it is stored, and how it is shared.

Contact: gulovmuhsin121@gmail.com

## 1. Summary

- Island runs on your computer. It has no server that receives your personal data.
- The developer does not collect, sell, rent, or share your personal data.
- The program does not use your data for advertising or any commercial purpose.
- Google account data (Google Calendar) is used only to show your events inside Island on your computer.

## 2. Data we request

| Source | Data | Why |
|---|---|---|
| Google (OAuth 2.0 + PKCE, optional) | Read-only access to your Google Calendar (scope `https://www.googleapis.com/auth/calendar.readonly`) via `accounts.google.com`, `oauth2.googleapis.com`, and CalDAV endpoint `apidata.googleusercontent.com/caldav/v2/`: calendar list and event titles, times and locations | To show your upcoming events in Island |
| Apple iCloud (CalDAV, optional) | Your calendars and events via `caldav.icloud.com` (and regional `p*-caldav.icloud.com` servers), using your Apple ID and an app-specific password you create | To show your upcoming events in Island |
| Calendar links (optional) | Calendar feeds in ICS format that you add yourself | To show events from other calendar services |
| Your location, approximated by IP address | Public IP address sent to IP-geolocation services | To find the city for the weather widget |
| Weather services | City coordinates | To show the weather forecast |
| GitHub | Request for the latest release at `api.github.com/repos/GulovM/island/releases/latest` (IP address, program version, and reading the release `body` changelog notes) | To check for program updates and display "What's new" release notes |
| Speed test service | Test data transfer | To run the speed test you start yourself |

Island reads the media information that Windows exposes for the current media session (track title, artist, playback state). It does not record or store that information beyond the current session.

## 3. How data is used

- Calendar events are used only to draw the calendar, the upcoming event notification, and the event list inside Island.
- Google OAuth 2.0 uses a public desktop `client_id` / `client_secret` pair with PKCE (`S256`) and a temporary local loopback listener (`127.0.0.1`).
- Data from Google APIs is used only to provide the features described above. It is not used to develop advertising, to build user profiles, or to train AI models.
- Application use of information received from Google APIs adheres to the Google API Services User Data Policy, including the Limited Use requirements.

## 4. Where data is stored

- Data is stored only on your computer, in the Island data folder (`%APPDATA%\island`) under your Windows user profile.
- OAuth tokens and Apple app-specific passwords (`%APPDATA%\island\cal_accounts.bin`), as well as calendar links and event caches (`%APPDATA%\island\calendar_cache_*.ics`), are encrypted with Windows DPAPI (`CryptProtectData`). A protected file cannot be opened by another Windows account or on another computer.
- Calendar event caches are stored locally as well, so that the calendar works offline.
- The developer does not receive copies of this data.

## 5. Sharing with third parties

Island sends data only to the services listed in section 2, and only for the features you use. Your data is not sold, shared or transferred to any other third party. Google and Apple receive requests from your computer exactly as they would from any other calendar app.

## 6. Your choices and deletion

- You choose whether to connect Google, Apple iCloud or any calendar link. You can disconnect any account in the program settings. Disconnecting deletes the stored tokens, passwords and caches from your computer.
- You can also remove access at any time at https://myaccount.google.com/permissions (Google) or in your Apple ID settings (Apple).
- To delete all Island data, remove the Island data folder under your Windows user profile and uninstall the program.

## 7. Children

Island is not directed to children under 13 and does not knowingly collect data from them.

## 8. Changes to this policy

This policy may change. The date at the top shows the last update. Changes are published in this file in the project repository.

---

# Политика конфиденциальности — Island (русская версия)

Дата обновления: 2026-11-11

Island — программа для Windows 10 и Windows 11, которая показывает «остров» (динамическое окно) вверху экрана: календарь, погоду, таймер, состояние системы и воспроизведение музыки. Этот документ описывает, какие данные использует программа, где они хранятся и кому передаются.

Контакт: gulovmuhsin121@gmail.com

## 1. Кратко

- Island работает на вашем компьютере. Нет сервера, который получает ваши персональные данные.
- Разработчик не собирает, не продаёт, не сдаёт в аренду и не передаёт персональные данные.
- Данные не используются для рекламы или других коммерческих целей.
- Данные аккаунта Google (Google Календарь) используются только для показа ваших событий внутри Island на вашем компьютере.

## 2. Какие данные запрашиваются

| Источник | Данные | Зачем |
|---|---|---|
| Google (OAuth 2.0 + PKCE, по желанию) | Доступ только на чтение Google Календаря (область `https://www.googleapis.com/auth/calendar.readonly`) через `accounts.google.com`, `oauth2.googleapis.com` и CalDAV-сервер `apidata.googleusercontent.com/caldav/v2/`: список календарей, названия, время и места событий | Показ ближайших событий в Island |
| Apple iCloud (CalDAV, по желанию) | Календари и события по протоколу CalDAV через `caldav.icloud.com` (и региональные серверы `p*-caldav.icloud.com`), вход по Apple ID и паролю приложения, который вы создаёте сами | Показ ближайших событий в Island |
| Ссылки на календари (по желанию) | Ленты календарей в формате ICS, которые вы добавляете сами | Показ событий из других календарных сервисов |
| Местоположение по IP-адресу | Публичный IP-адрес, отправляемый сервисам определения местоположения | Определение города для виджета погоды |
| Сервисы погоды | Координаты города | Прогноз погоды |
| GitHub | Запрос последней версии программы к `api.github.com/repos/GulovM/island/releases/latest` (IP-адрес, версия программы и чтение поля `body` со списком изменений) | Проверка обновлений и показ раздела «Что нового» |
| Сервис проверки скорости | Тестовая передача данных | Тест скорости, который вы запускаете сами |

Island читает сведения о воспроизведении, которые Windows отдаёт для текущей медиасессии (название трека, исполнитель, состояние воспроизведения). Эти сведения не записываются и не сохраняются дольше текущей сессии.

## 3. Как используются данные

- События календаря используются только для показа календаря, уведомлений о ближайшем событии и списка событий в Island.
- Для авторизации Google OAuth 2.0 используются публичные ключи настольного приложения (`client_id` / `client_secret`) вместе с механизмом PKCE (`S256`) и временным локальным сокетом (`127.0.0.1`).
- Данные из API Google используются только для работы описанных функций. Они не используются для разработки рекламы, построения профилей пользователей и обучения моделей ИИ.
- Использование полученных от API Google данных соответствует Политике пользовательских данных сервисов API Google, включая требования Limited Use.

## 4. Где хранятся данные

- Данные хранятся только на вашем компьютере, в папке данных Island (`%APPDATA%\island`) в профиле пользователя Windows.
- Токены OAuth и пароли приложений Apple (`%APPDATA%\island\cal_accounts.bin`), а также ссылки на календари и кэш событий (`%APPDATA%\island\calendar_cache_*.ics`) шифруются через Windows DPAPI (`CryptProtectData`). Защищённый файл не открывается другой учётной записью Windows и на другом компьютере.
- Кэш событий календаря также хранится локально, чтобы календарь работал без сети.
- Разработчик не получает копий этих данных.

## 5. Передача третьим лицам

Island отправляет данные только сервисам из раздела 2 и только для тех функций, которыми вы пользуетесь. Данные не продаются, не передаются и не сдаются другим третьим лицам. Google и Apple получают запросы с вашего компьютера так же, как от любого другого календарного приложения.

## 6. Ваш выбор и удаление данных

- Вы сами решаете, подключать ли Google, Apple iCloud или ссылку на календарь. Любой аккаунт можно отключить в настройках программы. При отключении токены, пароли и кэш удаляются с вашего компьютера.
- Доступ можно отозвать в любой момент: https://myaccount.google.com/permissions (Google) или в настройках Apple ID (Apple).
- Чтобы удалить все данные Island, удалите папку данных Island в профиле пользователя Windows и удалите программу.

## 7. Дети

Island не предназначен для детей младше 13 лет и сознательно не собирает их данные.

## 8. Изменения политики

Политика может меняться. Дата вверху показывает последнее обновление. Изменения публикуются в этом файле в репозитории проекта.
