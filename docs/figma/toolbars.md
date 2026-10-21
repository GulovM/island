# Toolbars (Панели инструментов)

> Источник: Figma Component Set `507:25993`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Верхние и нижние панели инструментов (Top Toolbars & Bottom Toolbars)

---

## 1. Размеры (Dimensions)
- **height_top**: `44 pt (Compact) / 54 pt / 96 pt (Large Title)`
- **height_bottom**: `44 pt / 49 pt`
- **width**: `Full width (402 pt на iPhone)`

## 2. Отступы (Paddings & Spacing)
- **top_toolbar**: `0px 16px 10px`
- **compact_toolbar**: `0px 10px 10px`
- **bottom_toolbar**: `12px 16px`

## 3. Радиусы скругления (Border Radius)
- **island_container**: `100px (плавающий остров)`
- **standard**: `0px (стык с краем экрана)`

## 4. Заливки и обводки (Fills & Strokes)
- **background_light**: `Materials/Regular Light: rgba(255, 255, 255, 0.6) + blur(50px)`
- **background_dark**: `Materials/Regular Dark: rgba(0, 0, 0, 0.41) + blur(50px)`
- **separator_line**: `0.5px #C6C6C8 (Light) / #38383A (Dark)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **backdrop_blur**: `blur(50px)`
- **scroll_edge_effect**: `Soft: blur(5px) + linear-gradient / Hard: blur(30px) + stroke 1px`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **rest**: Прозрачный фон до прокрутки контента
- **scrolled**: Активация Liquid Glass материала + размытие + разделитель
