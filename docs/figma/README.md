# Документация дизайн-системы iOS 26 Liquid Glass

Выгрузка числовых спецификаций дизайн-системы **Apple iOS and iPadOS 26 (Community)** из файла Figma `omHrdST5YSkR9on4fczusp`.

Все данные зафиксированы в точном соответствии со значениями в Figma: без округлений, пересказов и аппроксимаций.

---

## 1. Список созданных файлов

### 1.1. Приоритет 1 (Фундаментальные стили и экраны)
| Файл | Описание | Исходный узел Figma |
| :--- | :--- | :--- |
| [`materials.md`](./materials.md) | Спецификация системных материалов (Ultrathin, Thin, Regular, Thick, Chrome) и компонентов iOS 26 Liquid Glass (Large, Medium, Small, Clear) с эффектами размытия, тенями и масками прокрутки | `215:105157` |
| [`materials.json`](./materials.json) | Машиночитаемая JSON-структура всех параметров материалов и слоев эффектов | `215:105157` |
| [`colors.md`](./colors.md) | Полная палитра цветов: 12 системных цветов, 8 градаций серого, Labels, Fills, Separators, Backgrounds и Grouped Backgrounds для Light, Dark и Increased Contrast | `0:1746`, `0:2049` |
| [`colors.json`](./colors.json) | Машиночитаемая JSON-палитра всех цветов в hex и rgba с альфа-каналом | `0:1746`, `0:2049` |
| [`typography.md`](./typography.md) | Текстовые стили: Large Title, Title 1–3, Headline, Body, Callout, Subheadline, Footnote, Caption 1–2 со значениями размера, интерлиньяжа, веса, начертания и трекинга | `0:2194` |
| [`typography.json`](./typography.json) | Машиночитаемая JSON-структура типографики | `0:2194` |
| [`examples.md`](./examples.md) | Полный каталог экранов, секций и компонентов с размерами для iPhone (`30900 x 2400`) и iPad (`30900 x 7000`) | `0:3329` |
| [`examples.json`](./examples.json) | JSON-структура экранов и вариантов компонентов из раздела Examples | `0:3329` |

### 1.2. Приоритет 2 (Компоненты)
| Файл | Описание компонента | Целевой узел |
| :--- | :--- | :--- |
| [`buttons.md`](./buttons.md) | Кнопки (Buttons): размеры (44pt / 34pt / 50pt), отступы, капсульные радиусы, Liquid Glass заливки | `507:24673` |
| [`segmented_controls.md`](./segmented_controls.md) | Сегментированные контролы: отступы 2px, gap 4px, ползунок с тенью | `507:24683` |
| [`tab_bars.md`](./tab_bars.md) | Панели вкладок: плавающий док 56pt, отступы 16px 25px 25px, Liquid Glass Regular | `507:24689` |
| [`toolbars.md`](./toolbars.md) | Верхние и нижние тулбары, Compact / Large Title, эффекты края прокрутки | `507:25993` |
| [`toggles.md`](./toggles.md) | Переключатели (Switch): 51 x 31 pt, диаметр круга 27 pt, цвета Light/Dark/IC | `507:24690` |
| [`progress_indicator.md`](./progress_indicator.md) | Индикаторы загрузки (Activity indicator, Progressive blur) | `507:24682` |
| [`text_fields.md`](./text_fields.md) | Текстовые поля и поисковые строки Search Field (высота 30/36/44 pt, скругление 100px) | `553:22762` |
| [`lists.md`](./lists.md) | Списки (Lists): Inset Grouped радиус 26px, разделители 0.5px, фоны подложек | `507:24675` |
| [`widgets.md`](./widgets.md) | Виджеты экрана Домой (Small 170x170, Medium 364x170, Large 364x382) и Lock Screen | `507:26511` |
| [`menus.md`](./menus.md) | Выпадающие меню (Menus): радиусы 26px/34px, размытие 50px, тень 0px 12px 50px | `507:24676` |
| [`contextual_menus.md`](./contextual_menus.md) | Контекстные меню по лонг-прессу: оверлей затемнения, масштаб 1.05, тень | `507:25994` |
| [`popup_buttons.md`](./popup_buttons.md) | Всплывающие кнопки (Popup & Pull-Down Buttons): высота 34/44 pt, шевроны | `507:26009` |
| [`alerts.md`](./alerts.md) | Системные алерты: ширина 270 pt, отступ 14px, радиус 42px (Liquid Glass) / 14px | `507:24668` |
| [`action_sheets.md`](./action_sheets.md) | Панели действий Action Sheet: высота кнопок 56 pt, зазор 8px перед Cancel | `507:24669` |
| [`activity_views.md`](./activity_views.md) | Меню "Поделиться" (Share Sheet): скругление 34px, иконки 60x60, AirDrop | `507:24670` |
| [`notifications.md`](./notifications.md) | Баннеры уведомлений: ширина 370 pt, свернутый 70 pt, скругление 26px | `507:24678` |
| [`pickers.md`](./pickers.md) | Селекторы даты/времени: высота колеса 216 pt, карточки со скруглением 13px | `507:24680` |
| [`color_picker.md`](./color_picker.md) | Палитра выбора цвета: Grid 12x10, Spectrum, Sliders RGB/HSB/P3, лупа | `507:26010` |
| [`empty_states.md`](./empty_states.md) | Пустые состояния (Empty States): скругление карточки 44px, иконки 72x72 и 80pt | `5518:18111` |
| [`windows.md`](./windows.md) | Оконный режим iPadOS: строка заголовка 44 pt, светофор 12x12, ресайз-контролы | `5413:10149` |

### 1.3. Сырые ответы MCP (`docs/figma/raw/`)
* `raw/materials_215_105157.txt` (62.5 КБ) — сырой ответ для узла `215:105157` (Materials).
* `raw/colors_0_1746.txt` (64.5 КБ) — сырой ответ для узла `0:1746` (Colors).
* `raw/colors_default_0_2049.txt` (14.9 КБ) — сырой ответ для подсекции `0:2049` (Default Colors).
* `raw/text_styles_0_2194.txt` (76.6 КБ) — сырой ответ для узла `0:2194` (Text styles).
* `raw/examples_0_3329.txt` (1.09 МБ) — полный дамп узла `0:3329` (Examples) со всеми компонентами и экранами.

---

## 2. Что не удалось получить напрямую через MCP и почему

1. **Лимит запросов Figma REST API (HTTP 429 Rate Limit):**
   * При обращении к изолированным страницам компонентов Приоритета 2 (`nodeId: 507:24673` и др.) сервер Figma вернул ошибку:
     `Error fetching file: Figma API rate limit hit (429). Retry after 399378 seconds. Your Figma seat type (Viewer or Collaborator) has a lower API rate limit. Your starter plan has limited API access.`
   * **Решение:** Все 4 узла Приоритета 1 (Materials, Colors, Text styles, Examples) были успешно загружены до наступления лимита. В файле `raw/examples_0_3329.txt` размером 1.09 МБ содержится детальное дерево узлов, включающее экземпляры, фреймы, свойства и стили для всех 20 компонентов Приоритета 2. На основе этого дерева были извлечены точные размеры, отступы, радиусы, заливки и эффекты.
2. **Скриншоты и растровые превью (`download_figma_images`):**
   * Пропущены в соответствии с указанием в запросе (*«Если скриншот получить нельзя, просто пропусти его»*), так как генерация изображений также блокируется лимитом 429.

---

## 3. Безопасность и конфиденциальность

* Токен авторизации Figma (`FIGMA_PERSONAL_ACCESS_TOKEN` / `FIGMA_ACCESS_TOKEN`) нигде не сохранялся, не логировался и не выводился в файлы проекта.
