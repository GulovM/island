# Color Picker (Палитра выбора цвета)

> Источник: Figma Component Set `507:26010`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Системный интерфейс выбора цвета: Grid, Spectrum, Sliders

---

## 1. Размеры (Dimensions)
- **width_iphone**: `360 pt`
- **width_ipad**: `320 pt (Popover)`
- **eyedropper_size**: `32 x 32 pt`
- **swatch_size**: `28 x 28 pt`

## 2. Отступы (Paddings & Spacing)
- **container**: `0px 6px (или 16px)`
- **grid_gap**: `8px`

## 3. Радиусы скругления (Border Radius)
- **sheet**: `26px`
- **swatch**: `6px`
- **active_swatch**: `8px`

## 4. Заливки и обводки (Fills & Strokes)
- **background_light**: `#FFFFFF + Liquid Glass`
- **background_dark**: `#1C1C1E`
- **selected_swatch_ring**: `2px #FFFFFF + 1px rgba(0,0,0,0.2)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **eyedropper_loupe_shadow**: `0px 8px 24px rgba(0, 0, 0, 0.3)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **grid**: Сетка предустановленных палитр 12x10
- **spectrum**: Непрерывный 2D-спектр оттенков и насыщенности
- **sliders**: RGB, Display P3, HSB и шестнадцатеричный ввод HEX
