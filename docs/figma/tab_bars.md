# Tab Bars (Панели вкладок)

> Источник: Figma Component Set `507:24689`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Нижняя панель навигации iPhone и плавающая панель iPad

---

## 1. Размеры (Dimensions)
- **height_iphone**: `83 pt (включая safe area bottom 34 pt)`
- **floating_bar_height**: `56 pt`
- **icon_size**: `28 x 28 pt`

## 2. Отступы (Paddings & Spacing)
- **outer_padding**: `16px 25px 25px`
- **buttons_padding**: `0px 2px`
- **align_items**: `center`
- **gap**: `20px`

## 3. Радиусы скругления (Border Radius)
- **floating_pill**: `1000px / 34px (Liquid Glass)`
- **dock_style**: `34px`

## 4. Заливки и обводки (Fills & Strokes)
- **glass_background_light**: `rgba(245, 245, 245, 0.6) + backdrop blur(50px)`
- **glass_background_dark**: `rgba(255, 255, 255, 0.06), rgba(0, 0, 0, 0.6) + blur(50px)`
- **border_stroke**: `0.5px rgba(255, 255, 255, 0.18)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **box_shadow**: `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **unselected_tab**: Цвет иконки/текста: #8E8E93 (System Gray)
- **selected_tab**: Акцентный цвет: #0088FF / #0091FF (System Blue) или Tinted Glass подложка
- **split_search**: Режим с интеграцией поиска: Tabs=4 + Search icon
