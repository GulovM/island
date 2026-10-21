# Widgets (Виджеты Home & Lock Screen)

> Источник: Figma Component Set `507:26511`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Виджеты экрана 'Домой' и экрана блокировки

---

## 1. Размеры (Dimensions)
- **small_widget**: `170 x 170 pt`
- **medium_widget**: `364 x 170 pt`
- **large_widget**: `364 x 382 pt`
- **extra_large_ipad**: `762 x 382 pt`
- **lock_screen_circular**: `70 x 70 pt`
- **lock_screen_rectangular**: `160 x 70 pt`
- **lock_screen_inline**: `Высота 20 pt`

## 2. Отступы (Paddings & Spacing)
- **content_padding**: `16 pt`
- **lock_screen_padding**: `0px 32px`

## 3. Радиусы скругления (Border Radius)
- **home_widget**: `24px (Continuous corner radius)`
- **lock_screen_widget**: `1000px (Circular) / 16px (Rectangular)`

## 4. Заливки и обводки (Fills & Strokes)
- **widget_bg_light**: `Liquid Glass Regular Light + blur(50px) или #FFFFFF`
- **widget_bg_dark**: `Liquid Glass Regular Dark + blur(50px) или #1C1C1E`
- **lock_screen_fill**: `rgba(255, 255, 255, 0.2) + blur(20px)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **widget_shadow**: `0px 8px 30px 0px rgba(0, 0, 0, 0.12)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **normal**: Штатное отображение данных
- **empty_placeholder**: Скелетные плейсхолдеры
- **edit_jiggle**: Режим редактирования с кнопкой '-' в левом верхнем углу
