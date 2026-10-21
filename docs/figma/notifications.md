# Notifications (Баннеры уведомлений)

> Источник: Figma Component Set `507:24678`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Баннеры уведомлений Lock Screen и Dynamic Island

---

## 1. Размеры (Dimensions)
- **width**: `370 pt (iPhone)`
- **height_collapsed**: `70 pt`
- **height_expanded**: `160 pt - 280 pt`
- **app_icon_size**: `38 x 38 pt`

## 2. Отступы (Paddings & Spacing)
- **collapsed**: `12px 14px`
- **expanded**: `0px 8px (или 16px 16px)`

## 3. Радиусы скругления (Border Radius)
- **banner**: `26px (Liquid Glass)`
- **app_icon**: `8px`

## 4. Заливки и обводки (Fills & Strokes)
- **background_light**: `rgba(255, 255, 255, 0.6) + blur(50px) (Liquid Glass Regular)`
- **background_dark**: `rgba(0, 0, 0, 0.41) + blur(50px)`
- **border**: `0.5px rgba(255, 255, 255, 0.2)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **banner_shadow**: `0px 8px 32px 0px rgba(0, 0, 0, 0.12)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **collapsed**: Иконка, заголовок, время, 1 строка текста
- **expanded**: Развернутый контент, кнопки быстрых ответов
- **stack**: Эффект стопки (stack of 3) со смещением по Y (8px) и scale (0.95, 0.9)
