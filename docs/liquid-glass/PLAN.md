# Redesign "Liquid Glass" (iOS 26) для острова — план для исполнителя

Ветка `feat/liquid-glass`. Уровень A: без захвата экрана и без преломления; стекло = размытие DWM
(окно `glass_wnd`, `wnd::enable_blur_behind`) + слои, нарисованные в Direct2D.
Источник чисел: `docs/figma/*.md` и `*.json` (выгрузка Figma «iOS and iPadOS 26»). Выдумывать значения нельзя:
чего нет в выгрузке (блики кромки, преломление) — подбирается на глаз и помечается комментарием «подобрано».

## Общие правила (обязательны для каждой задачи)

1. Комментарии, сообщения и строки интерфейса — по-русски, в стиле окружающего кода (плотность комментариев,
   имена, идиомы). Английские варианты строк — в `src/i18n.rs` рядом с русскими.
2. Новых зависимостей в `Cargo.toml` не добавлять. `unsafe` — только там, где он уже есть (Win32/D2D).
3. Не коммитить, не менять ветку. Не трогать `docs/figma/`, `CHANGELOG.md`, `Cargo.toml` (кроме явно указанных).
4. Менять только файлы, перечисленные в задаче; нужно ещё один — объясни зачем в отчёте.
5. Чистую логику (числа, цвета, формулы) выноси в чистые функции с юнит-тестами по образцу `src/glass.rs`.
6. Перед отчётом выполни: `cargo clippy --all-targets -- -D warnings` и `cargo test`. Всё должно пройти.
   Если падало и до тебя — скажи. Не запускай `cargo fmt` на весь проект: проверь `git diff`, чтобы не было лишних правок.
7. Не ломай то, что уже работает: настройки `glass`, `glass_opacity`, `corner_radius`, поведение без стекла
   (`cfg.glass == false`) и без «Эффектов прозрачности» (`real_glass() == false`) остаются прежними.
8. Ответ-отчёт: список изменённых файлов, что сделано по пунктам задачи, что НЕ сделано и почему, результат
   cargo-команд (последние строки), спорные места.

## Дизайн-решения (приняты)

- Два стиля палитры, выбирает пользователь (настройка `palette_style`): `windows` (как сейчас: фон и акцент из
  Windows/пользовательские) и `ios` (системные цвета iOS: акцент Blue `#0091FF` тёмная / `#0088FF` светлая, метки,
  заливки, разделители из `docs/figma/colors.md`). По умолчанию `windows`, чтобы текущие пользователи ничего не потеряли.
- Материал тела острова выбирается пользователем (`material`: ultrathin|thin|regular|thick|chrome),
  по умолчанию `regular`. Ползунок прозрачности `glass_opacity` остаётся и поверх материала работает как сейчас.
- Не больше одного слоя стекла (само тело) + капля выбранной вкладки; остальные поверхности — не стекло.
- Текст на стекле не тусклее токена Label Secondary; основной — Label Primary.
- Уважать «Эффекты прозрачности» Windows (уже есть `self.transparency`).

## Задачи (выполняются строго по порядку, каждая принимается отдельно)

### T1. Токены `src/ios.rs`
Новый модуль (подключить в `src/main.rs`), только чистые данные и функции, без Win32.
- `Material` {Ultrathin, Thin, Regular, Thick, Chrome} и `fn material(m, dark: bool) -> MaterialSpec`:
  `blur_px` и список заливок `[(r,g,b,a)]` строго из `docs/figma/materials.json`/`materials.md` (раздел 1).
  Для светлой и тёмной тем свои значения. Также `fn body_tint(m, dark) -> (rgb, alpha)` — одна итоговая заливка тела
  (для Regular dark это `rgba(0,0,0,0.41)`; для остальных — по таблице; если в таблице две заливки — композиция
  «поверх» в порядке Figma, формулу опиши комментарием).
- Системные цвета: `enum SysColor` (Red…Brown), `fn sys(c, dark) -> Rgb` (Default Light/Dark из `colors.md`).
- Токены меток/заливок/разделителей/фонов: Label primary…quaternary, Fill primary…quaternary, Separator opaque/non-opaque,
  Background primary/secondary/tertiary (Dark Base) — функции `(dark) -> D2D1_COLOR_F`/`Rgb+alpha`.
- Радиусы: `R_LARGE=34`, `R_MEDIUM=26`, `R_SMALL_CAPSULE=1000` (капсула), тень стекла `GLASS_SHADOW`
  (`0, 8, 40, rgba(0,0,0,0.12)`), толщины и пр. из спецификации.
- Типографика: `enum TextRole` (LargeTitle…Caption2) и `fn type_spec(role) -> (size, line, weight, tracking_em)` по
  `typography.md`; плюс `fn island_scale() -> f32` — коэффициент перевода размеров iOS в размеры острова
  (в острове основной текст 9 px при iOS Body 17 → 9/17).
- Тесты фиксируют ключевые значения таблиц (все 5 материалов × 2 темы, 12 системных цветов × 2 темы,
  Label/Fill/Separator) и инварианты (альфы 0..1, blur 25 или 50, плотность растёт Ultrathin→Thick).
Файлы: `src/ios.rs` (новый), `src/main.rs`.

### T2. Конфиг и палитра
- `src/config.rs`: ключи `palette_style` (`windows`|`ios`, по умолчанию `windows`) и `material` (по умолчанию `regular`).
  Следуй существующему способу чтения/записи/валидации ключей и тестам конфигурации; неизвестное значение → умолчание.
- `src/theme.rs`: `derive` получает стиль. Для `ios`: акцент = `ios::sys(Blue)`, текст/dim = Label primary/secondary
  (по яркости фона), card/track/chip/inset = Fill tokens, hairline = Separator non-opaque; остальное как сейчас.
  Для `windows` результат `derive` не меняется (тест `default_palette_matches_original_look` должен проходить как есть).
- Места, где вызывается `theme::derive`/`theme::set` (`src/app.rs`, `src/state.rs`): передай стиль из конфига.
Файлы: `src/config.rs`, `src/theme.rs`, `src/app.rs`, `src/state.rs` (по необходимости).

### T3. Материал тела острова
- `src/glass.rs`: чистые функции слоёв — `body_layers(material, dark, opacity_pct, blur, p, k) -> BodyLayers`
  (тинт-цвет+альфа, подсветка сверху, параметры кромки, тень) с тестами. `body_alpha` и `MIN_BODY_ALPHA` не ломать:
  для `regular`+dark результат должен совпасть с текущим (0.41).
- `src/render.rs`: тело рисуется по `BodyLayers`: (1) тинт материала вместо заливки цветом фона (в стиле `windows`
  цвет фона острова остаётся тинтом, как сейчас; в стиле `ios` — тинт материала); (2) мягкая тень `0 8 40` ТОЛЬКО
  вниз и по бокам (верх упирается в край экрана): в D2D — эффект Shadow либо несколько прозрачных заливок со
  смещением; проверь, что окно (`WIN_LOGICAL`) оставляет под телом запас, иначе тень режется — в этом случае
  объясни и предложи минимальное изменение, не делай его молча; (3) зеркальная кромка: блик слева-сверху и
  мягкая дуга справа-снизу (улучшение текущих `rim_*`); (4) внутреннее осветление сверху (`sheen`).
- Без стекла/без blur — старое поведение.
- ВАЖНО: сейчас режим стекла выключен намеренно («режим стекла на доработке»): `config.rs::parse` игнорирует ключ
  `glass` (`("", "glass", _) => {}`), `Default.glass = false`, переключатель в настройках неактивен
  (`settings.rs`, `ui.switch(..., false)`). Эта работа и есть «доработка»: в T3 верни чтение ключа `glass`
  (Num → bool), в шаблоне и `Default` поставь `glass = 1` (включено), обнови тесты конфига (их ожидания про
  «всегда выключен» поменяй осознанно и перечисли в отчёте). Уже сохранённые `glass = 0` остаются выключенными.
  Переключатель в настройках включается в T6.
Файлы: `src/glass.rs`, `src/render.rs`, `src/theme.rs` (токены кромки/тени), `src/config.rs` (ключ glass), при необходимости `src/app.rs`.

### T4. Компоненты
Привести к языку iOS 26 (пропорции из `docs/figma/toggles.md`, `buttons.md`, `lists.md`, `segmented_controls.md`),
в масштабе острова:
- Капля выбранной вкладки (`render.rs`, блок `pill`) — капсула «Liquid Glass Small»: заливка Fill + кромка + мягкая тень.
- `ui/widgets.rs::switch`: пропорции 51:31, круг 27/31 высоты, ручка с мягкой тенью; цвет «включено» — акцент
  (в `ios` — Green из `ios::sys` как у системных тумблеров).
- `slider_thin`: дорожка и ручка по токенам, ручка — белая с тенью.
- `ui/canvas.rs::card` и `hairline`: радиус из токенов (в масштабе острова), граница — Separator non-opaque.
- Кнопки/чипы: капсульные, press-масштаб пружиной (`Spring`).
Файлы: `src/render.rs`, `src/ui/widgets.rs`, `src/ui/canvas.rs`, `src/ui/color.rs`.

### T5. Типографика
- `src/ui/fonts.rs`, `src/gfx.rs`: форматы по ролям (`ios::TextRole`) с весом 590 (Segoe UI Variable — вариативный)
  там, где в spec «Emphasized/Headline», трекинг через `IDWriteTextLayout1::SetCharacterSpacing` (в layout-кэше).
  Размеры в острове = `size_iOS * island_scale()` с округлением до 0.5. SF Pro недоступен — остаётся Segoe UI Variable.
  Не ломать раскладку: ширины вкладок (`measure_tabs`) и места с жёсткой вёрсткой должны помещаться;
  при смене метрик подстрой отступы минимально. Если для `IDWriteTextLayout1` нужен feature в `Cargo.toml` —
  это единственное допустимое изменение `Cargo.toml`.
Файлы: `src/ui/fonts.rs`, `src/gfx.rs`, при необходимости `src/wnd.rs`.

### T6. Настройки и доработка
- `src/tabs/settings.rs`, `src/i18n.rs`: переключатель «Стиль цветов: Windows | iOS», выбор материала
  (5 вариантов, чипы), применение на лету без перезапуска, сохранение в конфиг.
- Доступность: при выключенных «Эффектах прозрачности» Windows материал не размывает и становится плотнее
  (уже есть `body_alpha`) — убедись, что новые слои тоже отключаются.
- `README.md`: короткий раздел про стили/материалы (по-русски). `CHANGELOG.md` не трогать.
Файлы: `src/tabs/settings.rs`, `src/i18n.rs`, `README.md`, `src/app.rs` (применение настроек).

### T7. Финальная проверка (без правок, только отчёт)
`cargo build --release`, `cargo test`, `cargo clippy --all-targets -- -D warnings`; список мест, где остались
жёстко заданные цвета/радиусы вне токенов; список отклонений от spec.

### T13. Подложка стекла через Windows.UI.Composition (HostBackdropBrush) точно по контуру тела
Проблема: DWM-размытие (accent 3/4, `DWMWA_WINDOW_CORNER_PREFERENCE`) не берёт форму из `SetWindowRgn`, поэтому
сейчас подложка — прямоугольник со скруглением 8 px, вписанный в тело с отступом; по краям тела видна полоса
неразмытого фона, а углы подложки не совпадают с углами тела. Решение проверено прототипом на Win11 26300
(рабочий образец: `D:/GoTemp/claude/d--Desktop-Files-mainnew-island/943148af-4ab3-4003-980c-8bbe763acb2d/scratchpad/proto/src/main.rs`, режим 7): окно подложки рисует `HostBackdropBrush`
(системное размытие того, что под окном) в `SpriteVisual`, обрезанном `CompositionGeometricClip` по
`CompositionPathGeometry` из того же D2D-контура, что и тело острова; тень — `DropShadow` с маской той же формы.
Окно с `WS_EX_LAYERED | WS_EX_TRANSPARENT` пропускает клики в чужие окна и рисует размытие корректно.

1. `Cargo.toml`: добавить в features `windows` ровно эти: `"UI"`, `"UI_Composition"`, `"UI_Composition_Desktop"`,
   `"System"`, `"Graphics"`, `"Win32_System_WinRT"`, `"Win32_System_WinRT_Composition"`,
   `"Win32_System_WinRT_Graphics_Direct2D"`. Других изменений `Cargo.toml` нет.
2. Новый модуль `src/backdrop.rs` (весь `unsafe` WinRT/Win32 здесь), по образцу прототипа:
   - `struct Backdrop` — держит `DispatcherQueueController` (создаётся `CreateDispatcherQueueController` с
     `DQTYPE_THREAD_CURRENT` + `DQTAT_COM_NONE`: главный поток уже в MTA), `Compositor`, `DesktopWindowTarget`
     (`ICompositorDesktopInterop::CreateDesktopWindowTarget(hwnd, false)`), корневой `ContainerVisual`,
     общий `CompositionPathGeometry` (один на клип и на маску тени), `SpriteVisual` с `CreateHostBackdropBrush()`
     и `CreateGeometricClipWithGeometry(&geo)`, `SpriteVisual` тени (`DropShadow` + `SetMask` из
     `CompositionSurfaceBrush` ← `CompositionVisualSurface` ← `ShapeVisual` с `CompositionSpriteShape` той же
     геометрии, заливка чёрная). Тень вставляется под размытие (`InsertAtBottom`).
   - `Backdrop::new(hwnd) -> Result<Self>`; `set_shape(&self, geo: &ID2D1Geometry, w: f32, h: f32) -> Result<()>` —
     новая `CompositionPath` (обёртка `#[implement(IGeometrySource2D, IGeometrySource2DInterop)]` над
     `ID2D1Geometry`, `TryGetGeometryUsingFactory` → `E_NOTIMPL`) и `geo.SetPath(..)`; размеры визуалов, маски
     и `VisualSurface` — `w × h` (размер окна подложки); `set_shadow(&self, dy, blur, opacity)`;
     `set_visible(&self, bool)` — `root.SetIsVisible`.
   - Объекты WinRT живут в `App` всё время работы; при ошибке создания — `log::info` и откат к старому пути (п. 5).
3. Окно подложки (`src/app.rs`, создание `glass_wnd`): стили `WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP |
   WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT`, сразу
   `SetLayeredWindowAttributes(glass_wnd, COLORREF(0), 255, LWA_ALPHA)` и `DwmSetWindowAttribute(glass_wnd,
   DWMWINDOWATTRIBUTE(17) /* DWMWA_USE_HOSTBACKDROPBRUSH */, TRUE)`. Accent (`enable_blur_behind`) и
   `enable_round_blur` для этого пути НЕ вызывать.
4. `src/render.rs::apply_blur` для нового пути: окно подложки совпадает с главным окном
   (`win_x, win_y, win_w, win_h`, ставится сразу под `self.hwnd` через `SetWindowPos(.., Some(self.hwnd), ..)`
   только когда меняется ключ), форма — `body_path(g, x0, aw, ah, ear, r, false)` (тот же контур, что тело,
   координаты окна в физических px), передаётся в `set_shape` только при смене формы (ключ из округлённых
   `x0, aw, ah, ear, r`). Выкл (`on == false`) — `set_visible(false)` (окно не прятать, чтобы не мигало);
   `hide_glass` — как сейчас прячет окно. Тень: `dy = ios::GLASS_SHADOW` смещение × масштаб острова,
   `blur = blur_radius × масштаб`, `opacity` 0.35 тёмная / 0.18 светлая тема (подобрано). Сигнатуру
   `apply_blur` можно расширить (нужны `g` и тема).
5. Откат: если `Backdrop::new` не удался (Windows 10 до 1903 и т.п.) — прежний путь Windows 10 (accent-размытие +
   `SetWindowRgn` по `glass::body_outline`) без изменений. Ветку `blur_round` (DWM-скругление 8 px) и
   `glass::blur_plate`, `wnd::enable_round_blur` с их тестами удалить: новый путь их заменяет.
6. `real_glass()` и правила `body_alpha` не менять. `self.blur_ok` = новый путь создан ИЛИ старый accent принят.
7. `src/main.rs`: `mod backdrop;`. Памяти не терять: никаких `Box::leak`/`forget`.
Файлы: `Cargo.toml`, `src/backdrop.rs` (новый), `src/app.rs`, `src/render.rs`, `src/glass.rs`, `src/wnd.rs`,
`src/main.rs`, при необходимости `src/window.rs`.
Проверка Claude: снимки поверх яркого фона — подложка без полос и выступов по всему контуру, «ушки» и нижние
углы совпадают с телом, клики под островом проходят в окна ниже.

## Ссылки на выгрузку
`docs/figma/materials.md`, `colors.md`, `typography.md`, `toggles.md`, `buttons.md`, `lists.md`, `tab_bars.md`,
`segmented_controls.md`, `README.md` (индекс). Реф-сайт https://liquidglassdesign.com/what-is-liquid-glass —
только качественные рекомендации: стекло лишь для плавающего слоя, не больше 1–2 слоёв, уважать Reduce Transparency.
