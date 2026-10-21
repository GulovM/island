# Buttons (Кнопки)

> Источник: Figma Component Set `507:24673`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Системные кнопки iOS 26 и Liquid Glass кнопки

---

## 1. Размеры (Dimensions)
- **regular**: `Высота 44 pt (минимальная сенсорная зона 44 x 44 pt)`
- **small**: `Высота 34 pt / 28 pt`
- **large**: `Высота 50 pt / 54 pt (Prominent)`

## 2. Отступы (Paddings & Spacing)
- **standard**: `0px 16px (горизонтальные отступы)`
- **compact**: `0px 12px`
- **toolbar_group**: `0px 4px, gap: 20px (или 0px 2px, gap: 12px)`

## 3. Радиусы скругления (Border Radius)
- **capsule**: `1000px / 296px (Liquid Glass pill)`
- **rounded_rect**: `12px / 14px`
- **small_ui**: `296px (для 48x48)`

## 4. Заливки и обводки (Fills & Strokes)
- **primary_light**: `Заливка: Tint + Shadow (#0091FF, #999999, #FFFFFF, rgba(255, 255, 255, 0.75))`
- **primary_dark**: `Заливка: Tint + Shadow (#0091FF, #999999, #FFFFFF, rgba(255, 255, 255, 0.75))`
- **default_light**: `Заливка: #F7F7F7, #DDDDDD, rgba(255, 255, 255, 0.65)`
- **default_dark**: `Заливка: rgba(255, 255, 255, 0.06), rgba(0, 0, 0, 0.6), rgba(204, 204, 204, 0.5), opacity: 0.67`
- **strokes**: `Для Tinted/Glass: 1px градиентная или semi-transparent обводка rgba(255, 255, 255, 0.2)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **box_shadow**: `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`
- **active_pressed_shadow**: `0px 4px 20px 0px rgba(0, 0, 0, 0.16)`
- **backdrop_filter**: `blur(50px)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **normal**: Полная непрозрачность (opacity: 1.0), стандартная тень
- **pressed**: Scale: 0.96-0.98, тень уменьшается до 0px 4px 20px, легкое затемнение overlay rgba(0,0,0,0.08)
- **selected**: State=Primary: активный акцентный цвет #0091FF
- **disabled**: Opacity: 0.35, тени отсутствуют, взаимодействие отключено
