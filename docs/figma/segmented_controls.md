# Segmented Controls (Сегментированные переключатели)

> Источник: Figma Component Set `507:24683`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Элемент переключения между взаимоисключающими вкладками/режимами

---

## 1. Размеры (Dimensions)
- **height**: `32 pt (стандартный iOS), 36 pt (увеличенный)`
- **width**: `Растягивается по контейнеру (horizontal: fill)`

## 2. Отступы (Paddings & Spacing)
- **outer_padding**: `2px`
- **gap**: `4px`
- **bottom_spacing**: `0px 0px 24px`

## 3. Радиусы скругления (Border Radius)
- **container**: `100px (полная капсула) или 8.91px (Classic)`
- **thumb_segment**: `100px (полная капсула) или 6.93px`

## 4. Заливки и обводки (Fills & Strokes)
- **container_light**: `rgba(118, 118, 128, 0.12) или Liquid Glass Regular`
- **container_dark**: `rgba(118, 118, 128, 0.24)`
- **selected_thumb_light**: `#FFFFFF`
- **selected_thumb_dark**: `#636366 (Grays/Gray 2)`
- **strokes**: `None или 0.5px rgba(0, 0, 0, 0.04)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **selected_thumb_shadow**: `0px 3px 8px 0px rgba(0, 0, 0, 0.12), 0px 3px 1px 0px rgba(0, 0, 0, 0.04)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **unselected**: Цвет текста: Labels/Secondary (rgba(60, 60, 67, 0.6) / rgba(235, 235, 245, 0.6))
- **selected**: Цвет текста: Labels/Primary (#000000 / #FFFFFF), подложка с тенью
- **disabled**: Opacity: 0.38
