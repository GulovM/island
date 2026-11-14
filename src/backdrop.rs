//! Подложка стекла через Windows.UI.Composition: `HostBackdropBrush` с клипом по контуру тела
//! острова и тень `DropShadow` с маской той же формы. Весь `unsafe` WinRT/Win32 сосредоточен здесь.
//!
//! Прототип: `scratchpad/proto/src/main.rs`, режим 7 — рабочее поведение на Win11 26300.

use std::cell::Cell;

use windows::core::*;
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl,
};
use windows::Graphics::IGeometrySource2D;
use windows::System::{DispatcherQueue, DispatcherQueueController};
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::WinRT::Composition::{
    ICompositionCapabilitiesInteropFactory, ICompositorDesktopInterop,
};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGeometrySource2DInterop, IGeometrySource2DInterop_Impl, IGraphicsEffectD2D1Interop,
    IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
};
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA};
use windows_numerics::{Vector2, Vector3};

const DEFAULT_BLUR_SIGMA: f32 = 25.0;
const D2D1_BORDER_MODE_HARD: u32 = 1;

/// D2D1 GaussianBlurEffect для `CompositionEffectBrush`: сам `HostBackdropBrush` только сэмплирует
/// рабочий стол без размытия, поэтому его оборачивают в `CLSID_D2D1GaussianBlur`.
#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct GaussianBlurEffect {
    name: HSTRING,
    blur_amount: f32,
    border_mode: u32,
    optimization: u32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for GaussianBlurEffect_Impl {}

impl IGraphicsEffect_Impl for GaussianBlurEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.clone())
    }
    fn SetName(&self, _name: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for GaussianBlurEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        let s = unsafe { name.to_string().unwrap_or_default() };
        let prop = match s.as_str() {
            "BlurAmount" => D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
            "Optimization" => D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION.0 as u32,
            "BorderMode" => D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32,
            _ => return Err(E_INVALIDARG.into()),
        };
        unsafe {
            *index = prop;
            *mapping = GRAPHICS_EFFECT_PROPERTY_MAPPING(1); // GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT
        }
        Ok(())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(3)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        let insp = match index {
            i if i == D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32 => {
                PropertyValue::CreateSingle(self.blur_amount)?
            }
            i if i == D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION.0 as u32 => {
                PropertyValue::CreateUInt32(self.optimization)?
            }
            i if i == D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32 => {
                PropertyValue::CreateUInt32(self.border_mode)?
            }
            _ => return Err(E_INVALIDARG.into()),
        };
        insp.cast()
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

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
    /// Корневой визуал дерева композиции.
    root: ContainerVisual,
    /// Общая `CompositionPathGeometry` (один экземпляр на клип размытия и маску тени).
    geo: CompositionPathGeometry,
    /// Визуал размытия (`GaussianBlurEffect(HostBackdropBrush)` + `GeometricClip`).
    _blur_v: SpriteVisual,
    /// Кисть эффекта размытия (позволяет менять `Blur.BlurAmount` при смене материала/масштаба).
    blur_brush: CompositionEffectBrush,
    /// Ключ свойства `"Blur.BlurAmount"` для `CompositionPropertySet::InsertScalar`.
    prop_blur: HSTRING,
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
    /// Текущая σ размытия `GaussianBlurEffect`.
    blur_sigma: Cell<f32>,
}

/// Доступен ли композитный `HostBackdropBrush` (поддерживается DWM начиная с Windows 11, сборка 22000).
pub const fn use_host_backdrop(build: u32) -> bool {
    build >= 22000
}

/// σ гауссова размытия для выбранного материала и масштаба интерфейса.
pub(crate) fn blur_sigma(material: crate::ios::Material, dark: bool, scale: f32) -> f32 {
    let blur_px = crate::ios::material(material, dark).blur_px;
    (blur_px * 0.5 * scale.max(0.5)).clamp(1.0, 120.0)
}

/// Проверяет, поддерживает ли DWM/GPU композиционные эффекты (в т.ч. `GaussianBlurEffect` на
/// `HostBackdropBrush`). На RDP / Basic Display Driver / VM без шейдеров возвращает `false`.
pub(crate) unsafe fn effects_supported(hwnd: HWND) -> bool {
    let caps = factory::<CompositionCapabilities, ICompositionCapabilitiesInteropFactory>()
        .and_then(|f| f.GetForWindow(hwnd))
        .or_else(|_| CompositionCapabilities::GetForCurrentView());
    match caps {
        Ok(c) => c.AreEffectsSupported().unwrap_or(true),
        Err(_) => true,
    }
}

/// Включает `DWMWA_USE_HOSTBACKDROPBRUSH` на окне `hwnd` (+ best-effort `LWA_ALPHA` для совместимости
/// с `WS_EX_LAYERED` при сквозном `WS_EX_TRANSPARENT`).
pub(crate) unsafe fn enable_host_backdrop(hwnd: HWND) -> Result<()> {
    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
    let on = BOOL(1);
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_HOSTBACKDROPBRUSH,
        &on as *const _ as _,
        std::mem::size_of::<BOOL>() as u32,
    )
}

unsafe fn ensure_dispatcher_queue(slot: &mut Option<DispatcherQueueController>) -> Result<()> {
    if DispatcherQueue::GetForCurrentThread().is_ok() || slot.is_some() {
        return Ok(());
    }
    let dq = CreateDispatcherQueueController(DispatcherQueueOptions {
        dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
        threadType: DQTYPE_THREAD_CURRENT,
        apartmentType: DQTAT_COM_NONE,
    })?;
    *slot = Some(dq);
    Ok(())
}

impl Backdrop {
    /// Создаёт подложку для окна `hwnd`. Очередь диспетчера — `DQTYPE_THREAD_CURRENT` +
    /// `DQTAT_COM_NONE`: главный поток уже в MTA (`CoInitializeEx`). Контроллер очереди хранится в
    /// `dq_slot` на уровне `App`, чтобы повторное создание/восстановление `Backdrop` не падало с `0x8001010E`.
    pub(crate) unsafe fn new(
        hwnd: HWND,
        dq_slot: &mut Option<DispatcherQueueController>,
    ) -> Result<Self> {
        if !use_host_backdrop(crate::sys::windows_build_number()) {
            return Err(E_NOTIMPL.into());
        }
        if !effects_supported(hwnd) {
            return Err(E_NOTIMPL.into());
        }
        ensure_dispatcher_queue(dq_slot)?;

        let comp = Compositor::new()?;
        let interop: ICompositorDesktopInterop = comp.cast()?;
        let target = interop.CreateDesktopWindowTarget(hwnd, false)?;

        let root = comp.CreateContainerVisual()?;
        // Явный размер `root`: `DesktopWindowTarget` не является `Visual`, поэтому `RelativeSizeAdjustment`
        // у `root` на некоторых сборках Win11 может давать 0×0 при первичном построении дерева.
        root.SetSize(Vector2 { X: 1.0, Y: 1.0 })?;
        target.SetRoot(&root)?;

        // Начальная геометрия-заглушка: точка. Реальная форма устанавливается через `set_shape`.
        let geo = comp.CreatePathGeometry()?;

        // 1. Визуал размытия (`GaussianBlurEffect` поверх `HostBackdropBrush`), обрезанный `GeometricClip`.
        let prop_blur = HSTRING::from("Blur.BlurAmount");
        let source_name = HSTRING::from("source");
        let src_param: IGraphicsEffectSource =
            CompositionEffectSourceParameter::Create(&source_name)?.cast()?;
        let effect: IGraphicsEffect = GaussianBlurEffect {
            name: HSTRING::from("Blur"),
            blur_amount: DEFAULT_BLUR_SIGMA,
            border_mode: D2D1_BORDER_MODE_HARD,
            optimization: D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32,
            source: src_param,
        }
        .into();
        let animatable = windows_collections::IIterable::<HSTRING>::from(vec![prop_blur.clone()]);
        let effect_factory = comp.CreateEffectFactoryWithProperties(&effect, &animatable)?;
        let blur_brush = effect_factory.CreateBrush()?;
        blur_brush.SetSourceParameter(&source_name, &comp.CreateHostBackdropBrush()?)?;

        let blur_v = comp.CreateSpriteVisual()?;
        blur_v.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
        blur_v.SetBrush(&blur_brush)?;
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
            root,
            geo,
            _blur_v: blur_v,
            blur_brush,
            prop_blur,
            shadow_v,
            shape_v,
            vs,
            ds,
            _sprite_shape: sprite_shape,
            _target: target,
            _comp: comp,
            size: Cell::new((1.0, 1.0)),
            shadow: Cell::new((8.0, 40.0, 0.35)),
            blur_sigma: Cell::new(DEFAULT_BLUR_SIGMA),
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

        // Размеры корневого визуала, визуала тени и поверхности маски — только когда они изменились.
        if self.size.get() != (w, h) {
            let sz = Vector2 { X: w, Y: h };
            self.root.SetSize(sz)?;
            self.shadow_v.SetSize(sz)?;
            self.shape_v.SetSize(sz)?;
            self.vs.SetSourceSize(sz)?;
            self.size.set((w, h));
        }
        Ok(())
    }

    /// Обновляет силу размытия `GaussianBlurEffect` (`Blur.BlurAmount` = σ).
    pub(crate) fn set_blur(&self, sigma: f32) -> Result<()> {
        if (self.blur_sigma.get() - sigma).abs() < 0.05 {
            return Ok(());
        }
        self.blur_brush.Properties()?.InsertScalar(&self.prop_blur, sigma)?;
        self.blur_sigma.set(sigma);
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

#[cfg(test)]
mod tests {
    use super::{blur_sigma, use_host_backdrop};
    use crate::ios::Material;

    #[test]
    fn host_backdrop_build_threshold() {
        assert!(!use_host_backdrop(0));
        assert!(!use_host_backdrop(19041));
        assert!(!use_host_backdrop(19045));
        assert!(!use_host_backdrop(21999));
        assert!(use_host_backdrop(22000));
        assert!(use_host_backdrop(22631));
        assert!(use_host_backdrop(26100));
    }

    #[test]
    fn blur_sigma_scales_and_clamps() {
        assert_eq!(blur_sigma(Material::Regular, true, 1.0), 25.0);
        assert_eq!(blur_sigma(Material::Chrome, false, 1.0), 12.5);
        assert_eq!(blur_sigma(Material::Regular, true, 1.5), 37.5);
        // Минимальный и максимальный пороги защищают от вырожденного размытия.
        assert_eq!(blur_sigma(Material::Chrome, false, 0.01), 6.25);
        assert_eq!(blur_sigma(Material::Regular, true, 10.0), 120.0);
    }
}
