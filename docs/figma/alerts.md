# Alerts (Системные диалоговые окна предупреждения)

> Источник: Figma Component Set `507:24668`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Модальные диалоговые окна критических уведомлений и подтверждений

---

## 1. Размеры (Dimensions)
- **width**: `270 pt`
- **button_height**: `44 pt`
- **min_height**: `160 pt`

## 2. Отступы (Paddings & Spacing)
- **content_padding**: `14px`
- **header_padding**: `19px 16px 14px`
- **button_padding**: `0px 16px`

## 3. Радиусы скругления (Border Radius)
- **alert_card**: `42px (в Liquid Glass стиле) / 14px (Classic)`

## 4. Заливки и обводки (Fills & Strokes)
- **overlay**: `rgba(0, 0, 0, 0.4) (dimming)`
- **card_bg_light**: `rgba(255, 255, 255, 0.8) + blur(50px)`
- **card_bg_dark**: `rgba(30, 30, 30, 0.85) + blur(50px)`
- **separator**: `0.5px rgba(60, 60, 67, 0.2)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **alert_shadow**: `0px 16px 50px 0px rgba(0, 0, 0, 0.25)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **side_by_side**: Две кнопки рядом (кнопка 135 pt каждая)
- **stacked**: Три и более кнопки друг под другом (высота каждой 44 pt)
