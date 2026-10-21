# Toggles (Переключатели / Switch)

> Источник: Figma Component Set `507:24690`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Системный переключатель On/Off

---

## 1. Размеры (Dimensions)
- **width**: `51 pt`
- **height**: `31 pt`
- **thumb_diameter**: `27 pt`

## 2. Отступы (Paddings & Spacing)
- **thumb_offset**: `2px`
- **mode**: `row, justifyContent: space-between, alignItems: center`

## 3. Радиусы скругления (Border Radius)
- **track**: `1000px (15.5 pt капсула)`
- **thumb**: `1000px (13.5 pt круг)`

## 4. Заливки и обводки (Fills & Strokes)
- **track_on_light**: `#34C759 (System Green Light)`
- **track_on_dark**: `#30D158 (System Green Dark)`
- **track_on_ic**: `#008932 (IC Light) / #4AE968 (IC Dark)`
- **track_off_light**: `#E5E5EA (System Gray 5 Light)`
- **track_off_dark**: `#3A3A3C (System Gray 4 Dark)`
- **thumb**: `#FFFFFF`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **thumb_shadow**: `0px 3px 8px 0px rgba(0, 0, 0, 0.15), 0px 3px 1px 0px rgba(0, 0, 0, 0.06)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **off**: Track: Gray, Thumb слева (offset: 2px)
- **on**: Track: Green, Thumb справа (offset: 22px)
- **disabled**: Opacity: 0.4
