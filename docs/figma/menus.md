# Menus (Меню и выпадающие списки)

> Источник: Figma Component Set `507:24676`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Контекстные меню и выпадающие списки действий

---

## 1. Размеры (Dimensions)
- **min_width**: `250 pt`
- **max_width**: `300 pt`
- **item_height**: `44 pt`

## 2. Отступы (Paddings & Spacing)
- **container_padding**: `20px 26px (или 8px 0px)`
- **item_padding**: `10px 16px`
- **gap**: `4px`

## 3. Радиусы скругления (Border Radius)
- **menu_container**: `26px / 34px (Liquid Glass)`
- **item_highlight**: `10px`

## 4. Заливки и обводки (Fills & Strokes)
- **background_light**: `rgba(245, 245, 245, 0.6) + blur(50px)`
- **background_dark**: `rgba(28, 28, 28, 0.9) + blur(50px)`
- **destructive_label**: `#FF383C (Light) / #FF4245 (Dark)`
- **separator**: `0.5px rgba(60, 60, 67, 0.15)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **box_shadow**: `0px 12px 50px 0px rgba(0, 0, 0, 0.22)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **default**: Обычный пункт меню
- **highlighted**: Фон rgba(0, 0, 0, 0.06) (Light) / rgba(255, 255, 255, 0.1) (Dark)
- **destructive**: Красный цвет шрифта и иконки
- **disabled**: Opacity: 0.35
