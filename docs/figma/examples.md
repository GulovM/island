# iOS 26 Examples Specification (Screens & Components)

> Источник: Figma Canvas `Examples` (Node ID: `0:3329`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все названия экранов, компонентов, вариантов и их точные размеры извлечены без изменений. Сырые данные сохранены в `docs/figma/raw/examples_0_3329.txt`.

---

## 1. Общие размеры холстов и экранов

* **Базовый фрейм iPhone:** `402 x 874 pt` (iPhone 16 / iPhone 15 Pro).
* **Базовый фрейм iPad:** `1210 x 834 pt` (iPad Pro 11″ альбомная ориентация).
* **Секция iPhone (`5742:36225`):** `30900 x 2400 pt`.
* **Секция iPad (`5742:36226`):** `30900 x 7000 pt`.

---

## 2. Секция iPhone (`5742:36225`)

Список представленных экранов и демонстрационных блоков компонентов:

| Название блока / экрана | Node ID | Тип элемента | Размеры (Ш x В) | Варианты / Экраны в наборе |
| :--- | :--- | :--- | :--- | :--- |
| **Examples/Notifications** | `770:21905` | `COMPONENT_SET` | `402 x 874 pt` | • `Expanded=True` (`754:64316`)<br>• `Expanded=False` (`754:64315`, `402 x 874 pt`) |
| **Examples/Lock Screen** | `754:64083` | `COMPONENT` | `402 x 874 pt` | Экран блокировки с виджетами, часами и уведомлениями |
| **Examples/Home Screen Widgets** | `775:13152` | `COMPONENT_SET` | `402 x 874 pt` | • `Size=Small` (`754:62974`, `402 x 874 pt`)<br>• `Size=Medium` (`754:62987`)<br>• `Size=Large` (`754:63000`) |
| **Examples/Home Screen Quick Actions** | `754:62883` | `COMPONENT` | `402 x 874 pt` | Меню быстрых действий приложения на домашнем экране |
| **Examples/Home Screen** | `754:62878` | `COMPONENT` | `402 x 874 pt` | Главный рабочий стол с иконками и доком |
| **Examples/Control Center** | `2570:20912` | `COMPONENT` | `402 x 874 pt` | Экран Пункта управления с переключателями и ползунками |
| **Examples/Toolbars - Top** | `5661:41970` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Default` (`5661:41989`)<br>• `Type=Compact Large Title` (`5661:41980`)<br>• `Type=Large Title` (`5661:41971`)<br>• `Type=Title 2 Line` (`5661:42453`)<br>• `Type=Title 2 Line Left` (`5661:42566`) |
| **Examples/Toolbars - Bottom** | `2517:14528` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Text` (`2517:14526`)<br>• `Type=Symbols` (`2517:14527`)<br>• `Type=Search` (`5432:25429`) |
| **Examples/Steppers** | `5661:34264` | `COMPONENT` | `402 x 874 pt` | Экран управления степперами и числовыми счетчиками |
| **Examples/Text Fields** | `5661:33949` | `COMPONENT` | `402 x 874 pt` | Экран с полями ввода текста, поисковыми строками и состояниями фокуса |
| **Examples/Tab Bar** | `5660:57680` | `COMPONENT_SET` | `402 x 874 pt` | • `Split Search=False` (`5660:57681`)<br>• `Split Search=True` (`2517:14488`) |
| **Examples/Steppers (Inline)** | `5661:43368` | `COMPONENT` | `402 x 874 pt` | Вариант степперов внутри списков |
| **Examples/Slider** | `5661:43611` | `COMPONENT` | `402 x 874 pt` | Экран со слайдерами (горизонтальные ползунки) |
| **Examples/Sheets (Modals)** | `770:21908` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Fullscreen` (`754:64541`)<br>• `Type=Fullscreen Stack` (`754:64543`)<br>• `Type=Inspector` (`754:64555`) |
| **Examples/Picker** | `5661:46038` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Compact` (`5661:46039`)<br>• `Type=Inline` (`5661:44434`) |
| **Examples/Menus** | `770:21901` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Menu` (`754:64091`)<br>• `Type=Edit Medium` (`5711:35806`) |
| **Examples/List** | `781:15374` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Inset` (`754:63381`)<br>• `Type=Full Width` (`754:63455`)<br>• `Type=Full Width + Section Headers` (`754:63509`) |
| **Examples/Keyboard** | `770:21899` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Text` (`754:63077`)<br>• `Type=Numeric` (`754:63090`)<br>• `Type=Emoji` (`754:63103`)<br>• `Type=Find` (`5532:28030`)<br>• `Type=Find and Replace` (`754:63116`) |
| **Examples/Face ID** | `770:21898` | `COMPONENT_SET` | `402 x 874 pt` | • `State=Authenticating` (`754:62794`)<br>• `State=Success` (`754:62809`) |
| **Examples/Empty States** | `5581:40610` | `COMPONENT_SET` | `402 x 874 pt` | • `Config=Title and Subtitle` (`5581:39343`)<br>• `Config=Symbol in Circle` (`5581:40611`)<br>• `Config=Large Symbol` (`5581:40767`) |
| **Examples/Context Menu** | `754:62668` | `COMPONENT` | `402 x 874 pt` | Контекстное меню с эффектом размытия фона и подсветкой элемента |
| **Examples/Color Picker** | `3147:15989` | `COMPONENT` | `402 x 874 pt` | Экран палитры выбора цвета с сеткой, спектром и ползунками |
| **Examples/Action Sheet** | `754:62559` | `COMPONENT` | `402 x 874 pt` | Нижний диалог выбора действий (Action Sheet) |
| **Examples/Alert** | `770:21495` | `COMPONENT_SET` | `402 x 874 pt` | • `Type=Side-by-side` (`754:43567`)<br>• `Type=Stacked` (`754:57652`) |
| **Examples/Activity View** | `754:26934` | `COMPONENT` | `402 x 874 pt` | Экран системного меню "Поделиться" (Share Sheet) |

---

## 3. Секция iPad (`5742:36226`)

Список представленных экранов и демонстрационных блоков компонентов:

| Название блока / экрана | Node ID | Тип элемента | Размеры (Ш x В) | Варианты / Экраны в наборе |
| :--- | :--- | :--- | :--- | :--- |
| **Examples/Window** | `5589:4058` | `COMPONENT` | `1210 x 834 pt` | Оконный режим iPadOS с элементами управления окном и ресайзом |
| **Examples/Notifications** | `770:21906` | `COMPONENT_SET` | `1210 x 834 pt` | • `Expanded=False` (`754:64333`)<br>• `Expanded=True` (`754:64334`) |
| **Examples/Lock Screen** | `754:64084` | `COMPONENT` | `1210 x 834 pt` | Экран блокировки iPad |
| **Examples/Home Screen Widgets** | `775:13153` | `COMPONENT_SET` | `1210 x 834 pt` | • `Size=Small` (`754:63013`)<br>• `Size=Medium` (`754:63029`)<br>• `Size=Large` (`754:63045`)<br>• `Size=Extra Large` (`754:63061`) |
| **Examples/Home Screen Quick Actions** | `754:62926` | `COMPONENT` | `1210 x 834 pt` | Меню быстрых действий на iPad |
| **Examples/Home Screen** | `754:62918` | `COMPONENT` | `1210 x 834 pt` | Домашний экран iPadOS |
| **Examples/Control Center** | `2607:22514` | `COMPONENT` | `1210 x 834 pt` | Пункт управления iPad |
| **Examples/Toolbars - Top** | `5732:63624` | `COMPONENT_SET` | `1210 x 834 pt` | • `Type=2 Line - Left Aligned` (`5732:63592`)<br>• `Type=2 Line - Center Aligned` (`5732:63625`)<br>• `Type=2 Line - Large Title Inline` (`5735:65908`)<br>• `Type=2 Line - Large Title` (`5735:66009`) |
| **Examples/Tab Bars** | `5735:66270` | `COMPONENT_SET` | `1210 x 834 pt` | • `Type=Without Search` (`2517:14496`)<br>• `Type=With Search` (`5735:66271`) |
| **Examples/Sidebar** | `5726:29555` | `COMPONENT_SET` | `1210 x 834 pt` | • `Type=List` (`754:64697`)<br>• `Type=List (Multi-level)` (`5726:29556`) |
| **Examples/Sheets (Modals)** | `754:64609` | `COMPONENT` | `1210 x 834 pt` | Модальное окно (Sheet) на iPad |
| **Examples/Popover** | `754:64525` | `COMPONENT` | `1210 x 834 pt` | Всплывающее окно (Popover) с указателем стрелки |
| **Examples/Menus** | `5719:26500` | `COMPONENT_SET` | `1210 x 834 pt` | • `Type=Edit Menu` (`754:64157`)<br>• `Type=Menu and Submenu` (`5719:26501`) |
| **Examples/List** | `754:63935` | `COMPONENT` | `1210 x 834 pt` | Полноэкранный двухуровневый список |
| **Examples/Keyboard** | `770:21900` | `COMPONENT_SET` | `1210 x 834 pt` | • `Type=Text (Lower Case)` (`754:63259`)<br>• `Type=Text (Upper Case)` (`754:63281`) |
| **Examples/Context Menu** | `754:62748` | `COMPONENT` | `1210 x 834 pt` | Контекстное меню iPad |
| **Examples/Color Picker** | `754:62648` | `COMPONENT` | `1210 x 834 pt` | Палитра выбора цвета в модальном окне |
| **Examples/Action Sheet** | `754:62590` | `COMPONENT` | `1210 x 834 pt` | Всплывающий Action Sheet iPad |
| **Examples/Alert** | `770:21750` | `COMPONENT_SET` | `1210 x 834 pt` | • `Keyboard=False` (`754:62509`)<br>• `Keyboard=True` (`754:62521`) |
| **Examples/Activity View** | `754:26926` | `COMPONENT` | `1210 x 834 pt` | Всплывающий Share Sheet на iPad |
