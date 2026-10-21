# Pickers (Колеса и селекторы даты/времени)

> Источник: Figma Component Set `507:24680`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Колесные барабанные селекторы (Wheel) и календарные пикеры

---

## 1. Размеры (Dimensions)
- **wheel_height**: `216 pt`
- **wheel_row_height**: `34 pt`
- **inline_calendar_width**: `320 pt`

## 2. Отступы (Paddings & Spacing)
- **calendar_padding**: `12px 16px`

## 3. Радиусы скругления (Border Radius)
- **card_container**: `13px / 26px`
- **selection_indicator**: `8px`

## 4. Заливки и обводки (Fills & Strokes)
- **background_light**: `#FFFFFF + Liquid Glass`
- **background_dark**: `#1C1C1E`
- **selection_lens**: `rgba(118, 118, 128, 0.12)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **card_shadow**: `0px 10px 60px 0px rgba(0, 0, 0, 0.1)`
- **wheel_gradient_fade**: `Маска угасания по верхнему и нижнему краю (fade out)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **compact**: Кнопка с текущим значением даты/времени
- **inline**: Развернутый барабан или календарь
- **scrolling**: Плавный инерционный скролл с привязкой к шагу (snap to item)
