//! Подложка стекла через Windows.UI.Composition: `HostBackdropBrush` с клипом по контуру тела
//! острова и тень `DropShadow` с маской той же формы. Весь `unsafe` WinRT/Win32 сосредоточен здесь.
//!
//! Прототип: `scratchpad/proto/src/main.rs`, режим 7 — рабочее поведение на Win11 26300.

use std::cell::Cell;

use windows::core::*;
use windows::Graphics::IGeometrySource2D;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGeometrySource2DInterop, IGeometrySource2DInterop_Impl,
};
use windows::Win32::System::WinRT::*;
use windows::System::DispatcherQueueController;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

/// Обёртка над `ID2D1Geometry`, реализующая `IGeometrySource2D` / `IGeometrySource2DInterop`
/// для передачи D2D-контура в `CompositionPath`. `TryGetGeometryUsingFactory` → `E_NOTIMPL`
/// (композитор использует свою фабрику, но наша геометрия привязана к основной).
#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct Geo(ID2D1Geometry);

impl windows::Graphics::IGeometrySource2D_Impl for Geo_Impl {}

impl IGeometrySource2DInterop_Impl for Geo_Impl {
    fn GetGeometry(&self) -> Result<ID2D1Geometry> {
        Ok(self.0.clone())
    }
    fn TryGetGeometryUsingFactory(&self, _f: Ref<'_, ID2D1Factory>) -> Result<ID2D1Geometry> {
        Err(E_NOTIMPL.into())
    }
}

/// Состояние подложки стекла: WinRT Composition-объекты живут в `App` всё время работы.
pub(crate) struct Backdrop {
    /// Контроллер очереди диспетчера (удерживаем, чтобы очередь не разрушилась).
    _dq: DispatcherQueueController,
    /// Корневой визуал дерева композиции.
    root: ContainerVisual,
    /// Общая `CompositionPathGeometry` (один экземпляр на клип размытия и маску тени).
    geo: CompositionPathGeometry,
    /// Визуал размытия (`HostBackdropBrush` + `GeometricClip`).
    _blur_v: SpriteVisual,
    /// Визуал тени (`DropShadow` + маска из `CompositionVisualSurface`).
    shadow_v: SpriteVisual,
    /// Визуал-источник формы для маски тени.
    shape_v: ShapeVisual,
    /// Поверхность для маски тени (размер обновляется при смене формы).
    vs: CompositionVisualSurface,
    /// Объект тени (настраивается при смене темы/размеров).
    ds: DropShadow,
    /// Цель композиции (удерживаем, чтобы дерево не разрушилось).
    _target: DesktopWindowTarget,
    /// Композитор (нужен для обновления `CompositionPath`).
    _comp: Compositor,
    /// Спрайт-шейп для маски тени (обновляется при смене геометрии).
    _sprite_shape: CompositionSpriteShape,
    /// Последний размер визуалов и маски: пересоздавать поверхность маски на каждом кадре анимации
    /// незачем, окно подложки меняет размер только при смене DPI.
    size: Cell<(f32, f32)>,
    /// Последние параметры тени (смещение, размытие, непрозрачность): `set_shadow` зовётся каждый кадр.
    shadow: Cell<(f32, f32, f32)>,
}

impl Backdrop {
    /// Создаёт подложку для окна `hwnd`. Очередь диспетчера — `DQTYPE_THREAD_CURRENT` +
    /// `DQTAT_COM_NONE`: главный поток уже в MTA (`CoInitializeEx`).
    pub(crate) unsafe fn new(hwnd: HWND) -> Result<Self> {
        let dq = CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        })?;

        let comp = Compositor::new()?;
        let interop: ICompositorDesktopInterop = comp.cast()?;
        let target = interop.CreateDesktopWindowTarget(hwnd, false)?;

        let root = comp.CreateContainerVisual()?;
        root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        target.SetRoot(&root)?;

        // Начальная геометрия-заглушка: точка. Реальная форма устанавливается через `set_shape`.
        let geo = comp.CreatePathGeometry()?;

        // 1. Визуал размытия (`HostBackdropBrush`), обрезанный `GeometricClip` по контуру тела.
        let blur_v = comp.CreateSpriteVisual()?;
        blur_v.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        blur_v.SetBrush(&comp.CreateHostBackdropBrush()?)?;
        blur_v.SetClip(&comp.CreateGeometricClipWithGeometry(&geo)?)?;
        root.Children()?.InsertAtTop(&blur_v)?;

        // 2. Тень по форме: маска из `ShapeVisual` через `CompositionVisualSurface`.
        let shape_v = comp.CreateShapeVisual()?;
        shape_v.SetSize(Vector2 { X: 1.0, Y: 1.0 })?;
        let sprite_shape = comp.CreateSpriteShapeWithGeometry(&geo)?;
        sprite_shape.SetFillBrush(
            &comp.CreateColorBrushWithColor(Color {
                A: 255,
                R: 0,
                G: 0,
                B: 0,
            })?,
        )?;
        shape_v.Shapes()?.Append(&sprite_shape)?;

        let vs = comp.CreateVisualSurface()?;
        vs.SetSourceVisual(&shape_v)?;
        vs.SetSourceSize(Vector2 { X: 1.0, Y: 1.0 })?;
        let sb = comp.CreateSurfaceBrushWithSurface(&vs)?;

        let ds = comp.CreateDropShadow()?;
        ds.SetBlurRadius(40.0)?;
        ds.SetOffset(Vector3 {
            X: 0.0,
            Y: 8.0,
            Z: 0.0,
        })?;
        ds.SetColor(Color {
            A: 255,
            R: 0,
            G: 0,
            B: 0,
        })?;
        ds.SetOpacity(0.35)?;
        ds.SetMask(&sb)?;

        let shadow_v = comp.CreateSpriteVisual()?;
        shadow_v.SetSize(Vector2 { X: 1.0, Y: 1.0 })?;
        shadow_v.SetShadow(&ds)?;
        // Тень — под размытием.
        root.Children()?.InsertAtBottom(&shadow_v)?;

        Ok(Self {
            _dq: dq,
            root,
            geo,
            _blur_v: blur_v,
            shadow_v,
            shape_v,
            vs,
            ds,
            _sprite_shape: sprite_shape,
            _target: target,
            _comp: comp,
            size: Cell::new((1.0, 1.0)),
            shadow: Cell::new((8.0, 40.0, 0.35)),
        })
    }

    /// Устанавливает контур подложки из `ID2D1Geometry` (тот же D2D-путь, что у тела острова).
    /// Размеры визуалов и маски тени обновляются до `w × h` (размер окна подложки в физических px).
    pub(crate) unsafe fn set_shape(
        &self,
        d2d_geo: &ID2D1Geometry,
        w: f32,
        h: f32,
    ) -> Result<()> {
        // Новая `CompositionPath` из обёртки `Geo` над переданной геометрией.
        let src: IGeometrySource2D = Geo(d2d_geo.clone()).into();
        let path = CompositionPath::Create(&src)?;
        self.geo.SetPath(&path)?;

        // Размеры визуала тени и поверхности маски — только когда они изменились.
        if self.size.get() != (w, h) {
            let sz = Vector2 { X: w, Y: h };
            self.shadow_v.SetSize(sz)?;
            self.shape_v.SetSize(sz)?;
            self.vs.SetSourceSize(sz)?;
            self.size.set((w, h));
        }
        Ok(())
    }

    /// Настраивает параметры тени: `dy` — смещение вниз (px), `blur` — радиус размытия (px),
    /// `opacity` — непрозрачность (0.0..1.0).
    pub(crate) fn set_shadow(&self, dy: f32, blur: f32, opacity: f32) -> Result<()> {
        if self.shadow.get() == (dy, blur, opacity) {
            return Ok(());
        }
        self.shadow.set((dy, blur, opacity));
        self.ds.SetBlurRadius(blur)?;
        self.ds.SetOffset(Vector3 {
            X: 0.0,
            Y: dy,
            Z: 0.0,
        })?;
        self.ds.SetOpacity(opacity)?;
        Ok(())
    }

    /// Включает/выключает видимость подложки (без скрытия самого окна, чтобы не мигало).
    pub(crate) fn set_visible(&self, visible: bool) -> Result<()> {
        self.root.SetIsVisible(visible)
    }
}
