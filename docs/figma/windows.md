# Windows & Stage Manager (Окна iPadOS)

> Источник: Figma Component Set `5413:10149`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Оконный режим iPadOS, заголовки окон, Stage Manager и элементы ресайза

---

## 1. Размеры (Dimensions)
- **min_window_width**: `480 pt`
- **min_window_height**: `360 pt`
- **header_bar_height**: `44 pt`
- **traffic_lights_size**: `12 x 12 pt`
- **resize_handle_size**: `24 x 24 pt`

## 2. Отступы (Paddings & Spacing)
- **header_padding**: `0px 10px 10px`
- **menubar_gap**: `8px`

## 3. Радиусы скругления (Border Radius)
- **window**: `20px / 26px (Continuous curvature)`
- **traffic_lights**: `1000px`
- **resize_handle**: `4px`

## 4. Заливки и обводки (Fills & Strokes)
- **window_bg_light**: `#FFFFFF + Liquid Glass`
- **window_bg_dark**: `#1C1C1E + Liquid Glass`
- **border**: `1px rgba(255, 255, 255, 0.2) (Light) / 1px rgba(255, 255, 255, 0.1) (Dark)`
- **traffic_close**: `#FF5F56`
- **traffic_minimize**: `#FFBD2E`
- **traffic_maximize**: `#27C93F`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **window_shadow_active**: `0px 24px 80px 0px rgba(0, 0, 0, 0.35)`
- **window_shadow_inactive**: `0px 12px 40px 0px rgba(0, 0, 0, 0.2)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **active**: Окно на переднем плане, полный контраст, яркая тень
- **inactive**: Фоновое окно, уменьшенная тень, легкое диммирование
- **resizing**: Отображение направляющих и полупрозрачного контура
