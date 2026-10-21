# Progress Indicators (Индикаторы прогресса и загрузки)

> Источник: Figma Component Set `507:24682`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Activity Indicators (круговые спиннеры) и линейные Progress Bars

---

## 1. Размеры (Dimensions)
- **spinner_small**: `20 x 20 pt`
- **spinner_large**: `37 x 37 pt`
- **progress_bar_height**: `4 pt`

## 2. Отступы (Paddings & Spacing)
- **container**: `21px 24px 19px`

## 3. Радиусы скругления (Border Radius)
- **progress_bar**: `100px`
- **spinner_petals**: `100px`

## 4. Заливки и обводки (Fills & Strokes)
- **track**: `rgba(120, 120, 128, 0.2) (Light) / rgba(120, 120, 128, 0.36) (Dark)`
- **fill**: `#0088FF (Light) / #0091FF (Dark)`
- **spinner**: `#8E8E93`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **progressive_blur**: `Градиентный backdrop blur с маской размытия`
- **glow**: `0px 0px 8px rgba(0, 145, 255, 0.3)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **determinate**: Процент заполнения 0% - 100%
- **indeterminate**: Непрерывная анимация вращения 8-12 лепестков
