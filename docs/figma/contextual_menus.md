# Contextual Menus (Контекстные меню по лонг-прессу)

> Источник: Figma Component Set `507:25994`, подтвержденный инстансами в Canvas `Examples` (`0:3329`) и `Materials` (`215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые характеристики извлечены без округлений.

---

## Описание
Меню по долгому нажатию с затемнением фона и увеличением элемента

---

## 1. Размеры (Dimensions)
- **menu_width**: `250 pt`
- **preview_scale**: `Scale: 1.05`

## 2. Отступы (Paddings & Spacing)
- **menu_padding**: `6px`
- **item_padding**: `10px 16px`

## 3. Радиусы скругления (Border Radius)
- **menu**: `26px`
- **preview_item**: `Соответствует оригинальному элементу`

## 4. Заливки и обводки (Fills & Strokes)
- **dimming_overlay**: `rgba(0, 0, 0, 0.2) + backdrop blur(20px)`
- **menu_bg_light**: `rgba(255, 255, 255, 0.8) + blur(50px)`
- **menu_bg_dark**: `rgba(30, 30, 30, 0.85) + blur(50px)`

## 5. Слои эффектов (Effects: Shadows & Blur)
- **overlay_blur**: `backdropFilter: blur(20px)`
- **menu_shadow**: `0px 20px 60px 0px rgba(0, 0, 0, 0.3)`

## 6. Состояния и варианты (States: Normal, Pressed, Selected, Disabled)
- **animating_in**: Плавный scale от 0.9 к 1.0 с haptic feedback (UIImpactFeedbackGenerator)
- **active**: Меню открыто, фоновый контент недоступен для тапа
