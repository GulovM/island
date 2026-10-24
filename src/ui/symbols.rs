//! Векторные значки в духе SF Symbols (iOS 26) вместо глифов Segoe Fluent Icons.
//!
//! Каждый значок задан на сетке 24×24 набором частей: заливки, обводки с круглыми концами и
//! вырезы. При первом обращении части сливаются в одну геометрию (обводки расширяются в
//! контур), поэтому полупрозрачный значок не темнеет там, где штрихи пересекаются. Готовая
//! геометрия живёт в `Gfx` (ресурс фабрики) и рисуется одной заливкой с трансформацией.
//! Ключ — код глифа из `icon`: вызовы `Canvas::glyph` остались прежними, а неизвестный код
//! рисуется шрифтом, как раньше.

use std::f32::consts::PI;

use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows_numerics::Matrix3x2;

use super::geom::{pt, Rect};

/// Кегль значка (сторона квадрата сетки 24×24) в логических пикселях: обычный и крупный.
pub const ICON: f32 = 11.5;
pub const ICON_BIG: f32 = 16.0;

/// Толщина линии значков на сетке 24: на кегле `ICON` это ~1 px, как основа полужирного текста.
const W: f32 = 2.0;
/// Точность спрямления кривых при слиянии частей (в единицах сетки): круги остаются гладкими и
/// на крупном кегле.
const TOLERANCE: f32 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seg {
    M(f32, f32),
    L(f32, f32),
    Q(f32, f32, f32, f32),
    C(f32, f32, f32, f32, f32, f32),
    A { r: f32, x: f32, y: f32, large: bool, cw: bool },
    Z,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Path(Vec<Seg>);

impl Path {
    /// Зеркало по вертикальной оси сетки (назад ↔ вперёд).
    fn mirror(&self) -> Path {
        let m = |x: f32| 24.0 - x;
        Path(
            self.0
                .iter()
                .map(|s| match *s {
                    Seg::M(x, y) => Seg::M(m(x), y),
                    Seg::L(x, y) => Seg::L(m(x), y),
                    Seg::Q(a, b, x, y) => Seg::Q(m(a), b, m(x), y),
                    Seg::C(a, b, c, d, x, y) => Seg::C(m(a), b, m(c), d, m(x), y),
                    Seg::A { r, x, y, large, cw } => Seg::A { r, x: m(x), y, large, cw: !cw },
                    Seg::Z => Seg::Z,
                })
                .collect(),
        )
    }
}

enum Part {
    Fill(Path),
    Stroke(Path, f32),
    Cut(Path),
    CutStroke(Path, f32),
}

// ---------- построение контуров ----------

/// Контур в синтаксисе SVG (`M L H V Q C A Z`, абсолютные и относительные команды).
fn svg(s: &str) -> Path {
    let b = s.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    let (mut cx, mut cy, mut sx, mut sy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut cmd = b'M';
    let skip = |i: &mut usize| {
        while *i < b.len() && (b[*i] == b' ' || b[*i] == b',' || b[*i] == b'\n') {
            *i += 1;
        }
    };
    let num = |i: &mut usize| -> f32 {
        skip(i);
        let st = *i;
        if *i < b.len() && (b[*i] == b'-' || b[*i] == b'+') {
            *i += 1;
        }
        let mut dot = false;
        while *i < b.len() && (b[*i].is_ascii_digit() || (b[*i] == b'.' && !dot)) {
            dot |= b[*i] == b'.';
            *i += 1;
        }
        s[st..*i].parse().unwrap_or(0.0)
    };
    loop {
        skip(&mut i);
        if i >= b.len() {
            break;
        }
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                out.push(Seg::M(x, y));
                (cx, cy, sx, sy) = (x, y, x, y);
                // Пары после M — неявные L.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                out.push(Seg::L(x, y));
                (cx, cy) = (x, y);
            }
            b'H' => {
                let x = num(&mut i) + ox;
                out.push(Seg::L(x, cy));
                cx = x;
            }
            b'V' => {
                let y = num(&mut i) + oy;
                out.push(Seg::L(cx, y));
                cy = y;
            }
            b'Q' => {
                let (a, c) = (num(&mut i) + ox, num(&mut i) + oy);
                let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                out.push(Seg::Q(a, c, x, y));
                (cx, cy) = (x, y);
            }
            b'C' => {
                let (a, c) = (num(&mut i) + ox, num(&mut i) + oy);
                let (d, e) = (num(&mut i) + ox, num(&mut i) + oy);
                let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                out.push(Seg::C(a, c, d, e, x, y));
                (cx, cy) = (x, y);
            }
            b'A' => {
                let r = num(&mut i);
                let _ry = num(&mut i);
                let _rot = num(&mut i);
                let large = num(&mut i) != 0.0;
                let cw = num(&mut i) != 0.0;
                let (x, y) = (num(&mut i) + ox, num(&mut i) + oy);
                out.push(Seg::A { r, x, y, large, cw });
                (cx, cy) = (x, y);
            }
            b'Z' => {
                out.push(Seg::Z);
                (cx, cy) = (sx, sy);
            }
            _ => break,
        }
    }
    Path(out)
}

/// Точка окружности: угол в градусах от верхней точки по часовой стрелке.
fn at(cx: f32, cy: f32, r: f32, deg: f32) -> (f32, f32) {
    let a = deg * PI / 180.0;
    (cx + r * a.sin(), cy - r * a.cos())
}

fn circle(cx: f32, cy: f32, r: f32) -> Path {
    Path(vec![
        Seg::M(cx, cy - r),
        Seg::A { r, x: cx, y: cy + r, large: false, cw: true },
        Seg::A { r, x: cx, y: cy - r, large: false, cw: true },
        Seg::Z,
    ])
}

/// Дуга по часовой стрелке от `from` до `to` (градусы от верхней точки).
fn arc(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Path {
    let (x0, y0) = at(cx, cy, r, from);
    let (x1, y1) = at(cx, cy, r, to);
    Path(vec![Seg::M(x0, y0), Seg::A { r, x: x1, y: y1, large: to - from > 180.0, cw: true }])
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0);
    let (rt, bt) = (x + w, y + h);
    Path(vec![
        Seg::M(x + r, y),
        Seg::L(rt - r, y),
        Seg::Q(rt, y, rt, y + r),
        Seg::L(rt, bt - r),
        Seg::Q(rt, bt, rt - r, bt),
        Seg::L(x + r, bt),
        Seg::Q(x, bt, x, bt - r),
        Seg::L(x, y + r),
        Seg::Q(x, y, x + r, y),
        Seg::Z,
    ])
}

fn poly(p: &[(f32, f32)], closed: bool) -> Path {
    let mut v: Vec<Seg> = p.iter().enumerate().map(|(i, &(x, y))| if i == 0 { Seg::M(x, y) } else { Seg::L(x, y) }).collect();
    if closed {
        v.push(Seg::Z);
    }
    Path(v)
}

fn line(a: (f32, f32), b: (f32, f32)) -> Path {
    poly(&[a, b], false)
}

/// Скруглённый треугольник «назад» (остриё слева) между `tip` и `base` по оси x.
fn tri_left(tip: f32, base: f32, top: f32, bottom: f32) -> Path {
    let cy = (top + bottom) / 2.0;
    let k = 1.3;
    let (dx, dy) = (base - tip, cy - top);
    let len = (dx * dx + dy * dy).sqrt();
    let (ux, uy) = (dx / len * k, dy / len * k);
    Path(vec![
        Seg::M(base, top + k),
        Seg::Q(base, top, base - ux, top + uy),
        Seg::L(tip + ux, cy - uy),
        Seg::Q(tip, cy, tip + ux, cy + uy),
        Seg::L(base - ux, bottom - uy),
        Seg::Q(base, bottom, base, bottom - k),
        Seg::Z,
    ])
}

/// Динамик: корпус с раструбом слева (место справа — под волны или косую черту).
fn speaker() -> Path {
    svg("M2.2 9.7 Q2.2 8.6 3.3 8.6 H6 L10 5 Q11.3 3.9 11.3 5.6 V18.4 Q11.3 20.1 10 19 L6 15.4 H3.3 Q2.2 15.4 2.2 14.3 Z")
}

fn speaker_waves(n: usize) -> Vec<Part> {
    let mut v = vec![Part::Fill(speaker())];
    for r in [5.0, 8.6, 12.2].into_iter().take(n) {
        v.push(Part::Stroke(arc(9.0, 12.0, r, 52.0, 128.0), W));
    }
    v
}

fn chevron(points: [(f32, f32); 3]) -> Vec<Part> {
    vec![Part::Stroke(poly(&points, false), 2.2)]
}

/// Пятиконечная звезда (вершина вверх).
fn star() -> Path {
    let pts: Vec<(f32, f32)> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { 9.6 } else { 4.3 };
            at(12.0, 12.8, r, i as f32 * 36.0)
        })
        .collect();
    poly(&pts, true)
}

/// Шестерёнка: восемь трапециевидных зубцов, скругление даёт обводка поверх заливки.
fn gear() -> Path {
    let (big, small) = (9.6, 7.4);
    let mut pts = Vec::new();
    for i in 0..8 {
        let a = i as f32 * 45.0;
        pts.push(at(12.0, 12.0, small, a - 14.0));
        pts.push(at(12.0, 12.0, big, a - 8.5));
        pts.push(at(12.0, 12.0, big, a + 8.5));
        pts.push(at(12.0, 12.0, small, a + 14.0));
        pts.push(at(12.0, 12.0, small - 0.15, a + 22.5));
    }
    poly(&pts, true)
}

/// Стрелка по часовой стрелке (обновить, перезагрузка): открытое кольцо с наконечником сверху.
fn arrow_clockwise() -> Vec<Part> {
    let (cx, cy, r, end) = (12.0, 12.8, 7.6, 336.0);
    let (px, py) = at(cx, cy, r, end);
    let a = end * PI / 180.0;
    let (tx, ty) = (a.cos(), a.sin());
    let (nx, ny) = (a.sin(), -a.cos());
    let head = poly(
        &[
            (px + 3.4 * tx, py + 3.4 * ty),
            (px - 0.6 * tx + 3.1 * nx, py - 0.6 * ty + 3.1 * ny),
            (px - 0.6 * tx - 3.1 * nx, py - 0.6 * ty - 3.1 * ny),
        ],
        true,
    );
    vec![Part::Stroke(arc(cx, cy, r, 42.0, end - 4.0), W), Part::Fill(head.clone()), Part::Stroke(head, 1.0)]
}

/// Части значка по коду глифа (`icon::*`); `None` — значка нет, рисуется шрифтом.
fn parts(code: u32) -> Option<Vec<Part>> {
    use Part::*;
    Some(match code {
        // wifi
        0xE701 => {
            let (cx, cy) = (12.0, 19.8);
            let mut v = vec![Fill(circle(12.0, 18.3, 2.15))];
            for r in [5.6, 9.9, 14.2] {
                v.push(Stroke(arc(cx, cy, r, -47.0, 47.0), 2.5));
            }
            v
        }
        // bluetooth
        0xE702 => vec![Stroke(svg("M6.6 7.7 L17 16.4 L12 20.8 V3.2 L17 7.6 L6.6 16.3"), 2.1)],
        // sun.max.fill (яркость, солнце)
        0xE706 => {
            let mut v = vec![Fill(circle(12.0, 12.0, 4.4))];
            for i in 0..8 {
                let a = i as f32 * 45.0;
                v.push(Stroke(line(at(12.0, 12.0, 7.5, a), at(12.0, 12.0, 9.9, a)), 2.2));
            }
            v
        }
        // moon.fill
        0xE708 => vec![Fill(circle(11.2, 12.9, 8.6)), Cut(circle(17.0, 8.3, 7.2))],
        // gearshape.fill
        0xE713 => vec![Fill(gear()), Stroke(gear(), 1.4), Cut(circle(12.0, 12.0, 3.4))],
        // lock.fill
        0xE72E => vec![
            Fill(rrect(5.0, 10.4, 14.0, 10.8, 2.8)),
            Stroke(svg("M8.3 10.8 V8 A3.7 3.7 0 0 1 15.7 8 V10.8"), 2.2),
            Cut(circle(12.0, 15.2, 1.55)),
            CutStroke(line((12.0, 15.6), (12.0, 17.7)), 1.5),
        ],
        // magnifyingglass
        0xE721 => vec![Stroke(circle(10.6, 10.6, 6.6), 2.1), Stroke(line((15.5, 15.5), (20.3, 20.3)), 2.6)],
        // speaker.wave.2.fill (громкость, динамик)
        0xE767 | 0xE7F5 | 0xE993 => speaker_waves(2),
        0xE992 => speaker_waves(1),
        0xE994 => speaker_waves(3),
        // speaker.slash.fill
        0xE74F => vec![
            Fill(speaker()),
            Stroke(arc(9.0, 12.0, 5.0, 52.0, 128.0), W),
            CutStroke(line((3.4, 3.4), (20.6, 20.6)), 4.6),
            Stroke(line((3.4, 3.4), (20.6, 20.6)), 2.1),
        ],
        // play.fill
        0xE768 | 0xF5B0 => vec![Fill(svg("M7.4 5.7 Q7.4 3.9 9 4.8 L19.4 10.9 Q20.9 12 19.4 13.1 L9 19.2 Q7.4 20.1 7.4 18.3 Z"))],
        // pause.fill
        0xE769 => vec![Fill(rrect(6.0, 4.8, 4.4, 14.4, 1.6)), Fill(rrect(13.6, 4.8, 4.4, 14.4, 1.6))],
        // power
        0xE7E8 => vec![Stroke(arc(12.0, 12.9, 8.1, 38.0, 322.0), 2.2), Stroke(line((12.0, 3.3), (12.0, 11.4)), 2.2)],
        // arrow.clockwise (перезагрузка, обновить)
        0xE777 | 0xE72C => arrow_clockwise(),
        // doc.on.doc
        0xE8C8 => vec![
            Stroke(rrect(4.2, 7.8, 11.4, 12.8, 2.6), 1.9),
            Stroke(svg("M8.4 7.8 V6.2 Q8.4 3.6 11 3.6 H17.2 Q19.8 3.6 19.8 6.2 V13.6 Q19.8 16.2 17.2 16.2 H15.6"), 1.9),
        ],
        // link
        0xE71B => vec![
            Stroke(svg("M10.6 7.4 L12.3 5.7 A4.25 4.25 0 0 1 18.3 11.7 L16.6 13.4"), W),
            Stroke(svg("M13.4 16.6 L11.7 18.3 A4.25 4.25 0 0 1 5.7 12.3 L7.4 10.6"), W),
            Stroke(line((9.7, 14.3), (14.3, 9.7)), W),
        ],
        // photo
        0xE8B9 => vec![
            Stroke(rrect(3.0, 5.0, 18.0, 14.0, 3.0), 1.9),
            Fill(svg("M5.8 17 L9.6 12.5 Q10.2 11.9 10.8 12.5 L13.4 15.3 L14.9 13.7 Q15.5 13.1 16.1 13.7 L18.4 16.3 V17 Z")),
            Fill(circle(15.6, 9.4, 1.7)),
        ],
        // pin.fill
        0xE718 => vec![
            Fill(svg("M8.2 3.4 H15.8 Q17 3.4 16.6 4.6 L15.6 7.6 V10.4 L18 13.2 Q18.8 14.6 17.2 14.6 H6.8 Q5.2 14.6 6 13.2 L8.4 10.4 V7.6 L7.4 4.6 Q7 3.4 8.2 3.4 Z")),
            Stroke(line((12.0, 14.0), (12.0, 20.8)), 1.9),
        ],
        // trash
        0xE74D => vec![
            Stroke(line((4.4, 6.6), (19.6, 6.6)), W),
            Stroke(svg("M9.4 6.6 V5.2 Q9.4 3.7 10.9 3.7 H13.1 Q14.6 3.7 14.6 5.2 V6.6"), 1.8),
            Stroke(svg("M6.3 6.8 L7.2 18.6 Q7.4 20.6 9.4 20.6 H14.6 Q16.6 20.6 16.8 18.6 L17.7 6.8"), 1.9),
            Stroke(line((10.1, 10.4), (10.1, 16.8)), 1.6),
            Stroke(line((13.9, 10.4), (13.9, 16.8)), 1.6),
        ],
        // plus
        0xE710 => vec![Stroke(line((12.0, 5.0), (12.0, 19.0)), 2.2), Stroke(line((5.0, 12.0), (19.0, 12.0)), 2.2)],
        // checkmark
        0xE73E => vec![Stroke(poly(&[(5.0, 12.8), (9.7, 17.4), (19.0, 6.8)], false), 2.4)],
        // clock
        0xE121 => vec![Stroke(circle(12.0, 12.0, 8.6), W), Stroke(poly(&[(12.0, 7.0), (12.0, 12.4), (15.4, 14.4)], false), W)],
        // calendar
        0xE787 => {
            let mut v = vec![
                Stroke(rrect(3.8, 5.2, 16.4, 15.2, 3.4), 1.9),
                Fill(svg("M3.8 10 V8.6 Q3.8 5.2 7.2 5.2 H16.8 Q20.2 5.2 20.2 8.6 V10 Z")),
                Stroke(line((8.2, 3.0), (8.2, 6.4)), W),
                Stroke(line((15.8, 3.0), (15.8, 6.4)), W),
            ];
            for (x, y) in [(8.4, 13.6), (12.0, 13.6), (15.6, 13.6), (8.4, 17.0), (12.0, 17.0)] {
                v.push(Fill(circle(x, y, 1.05)));
            }
            v
        }
        // folder.fill
        0xE8B7 => vec![Fill(svg(
            "M3 7.2 Q3 5 5.2 5 H9.1 Q10 5 10.6 5.6 L12 7 H18.8 Q21 7 21 9.2 V16.8 Q21 19 18.8 19 H5.2 Q3 19 3 16.8 Z",
        ))],
        // square.grid.2x2.fill
        0xE71D => vec![
            Fill(rrect(3.8, 3.8, 7.2, 7.2, 2.0)),
            Fill(rrect(13.0, 3.8, 7.2, 7.2, 2.0)),
            Fill(rrect(3.8, 13.0, 7.2, 7.2, 2.0)),
            Fill(rrect(13.0, 13.0, 7.2, 7.2, 2.0)),
        ],
        // list.bullet
        0xE8FD => vec![
            Fill(rrect(3.6, 4.9, 3.4, 3.4, 1.2)),
            Fill(rrect(9.4, 5.5, 11.0, 2.2, 1.1)),
            Fill(rrect(3.6, 10.3, 3.4, 3.4, 1.2)),
            Fill(rrect(9.4, 10.9, 11.0, 2.2, 1.1)),
            Fill(rrect(3.6, 15.7, 3.4, 3.4, 1.2)),
            Fill(rrect(9.4, 16.3, 11.0, 2.2, 1.1)),
        ],
        // backward.end.fill / forward.end.fill
        0xE892 => vec![Fill(rrect(4.4, 5.4, 2.7, 13.2, 1.35)), Fill(tri_left(8.4, 19.6, 5.6, 18.4))],
        0xE893 => vec![Fill(rrect(4.4, 5.4, 2.7, 13.2, 1.35).mirror()), Fill(tri_left(8.4, 19.6, 5.6, 18.4).mirror())],
        // backward.fill / forward.fill (перемотка)
        0xEB9E => vec![Fill(tri_left(1.8, 12.0, 6.2, 17.8)), Fill(tri_left(11.2, 21.4, 6.2, 17.8))],
        0xEB9D => vec![Fill(tri_left(1.8, 12.0, 6.2, 17.8).mirror()), Fill(tri_left(11.2, 21.4, 6.2, 17.8).mirror())],
        // music.note
        0xE8D6 => vec![
            Fill(svg("M17.6 3.3 Q18.9 3 18.9 4.3 V7.3 Q18.9 8.2 18 8.4 L11.4 9.9 V5.1 Z")),
            Stroke(line((10.6, 5.4), (10.6, 17.2)), 2.1),
            Fill(circle(8.1, 17.3, 3.3)),
        ],
        // xmark
        0xE711 => vec![Stroke(line((6.2, 6.2), (17.8, 17.8)), 2.2), Stroke(line((17.8, 6.2), (6.2, 17.8)), 2.2)],
        // clipboard
        0xE77F => vec![Stroke(rrect(5.0, 4.6, 14.0, 16.4, 2.8), 1.9), Fill(rrect(8.4, 2.6, 7.2, 4.0, 1.6))],
        // chevrons
        0xE76B => chevron([(14.8, 5.6), (8.4, 12.0), (14.8, 18.4)]),
        0xE76C => chevron([(9.2, 5.6), (15.6, 12.0), (9.2, 18.4)]),
        0xE70D => chevron([(5.6, 9.2), (12.0, 15.6), (18.4, 9.2)]),
        0xE70E => chevron([(5.6, 14.8), (12.0, 8.4), (18.4, 14.8)]),
        // star / star.fill
        0xE734 => vec![Stroke(star(), 1.8)],
        0xE735 => vec![Fill(star()), Stroke(star(), 1.6)],
        // pencil
        0xE70F => vec![
            Stroke(svg("M4.4 19.6 L5.5 15 L15.9 4.6 A2.4 2.4 0 0 1 19.4 8.1 L9 18.5 Z"), 1.9),
            Stroke(line((13.9, 6.6), (17.4, 10.1)), 1.9),
        ],
        // bell.fill
        0xEA8F => vec![
            Fill(svg("M12 3 C8.5 3 6.5 5.7 6.5 9 V12.6 L5 15.3 Q4.4 16.6 5.9 16.6 H18.1 Q19.6 16.6 19 15.3 L17.5 12.6 V9 C17.5 5.7 15.5 3 12 3 Z")),
            Fill(svg("M9.4 18 H14.6 A2.6 2.6 0 0 1 9.4 18 Z")),
        ],
        // circle.lefthalf.filled (тёмная тема)
        0xE793 => vec![Stroke(circle(12.0, 12.0, 8.6), W), Fill(svg("M12 3.4 A8.6 8.6 0 0 0 12 20.6 Z"))],
        // cloud.fill
        0xE753 => vec![Fill(svg("M7.1 18.6 H17 A4.3 4.3 0 0 0 17.5 10 A6 6 0 0 0 6.3 10.8 A3.95 3.95 0 0 0 7.1 18.6 Z"))],
        // arrow.right
        0xE72A => vec![
            Stroke(line((4.8, 12.0), (18.6, 12.0)), 2.2),
            Stroke(poly(&[(12.8, 6.0), (18.8, 12.0), (12.8, 18.0)], false), 2.2),
        ],
        // doc.text
        0xE8A5 => vec![
            Stroke(svg("M13.6 3.5 H7.6 Q5.4 3.5 5.4 5.7 V18.3 Q5.4 20.5 7.6 20.5 H16.4 Q18.6 20.5 18.6 18.3 V8.5 Z"), 1.9),
            Stroke(svg("M13.6 3.6 V7.1 Q13.6 8.4 14.9 8.4 H18.5"), 1.7),
            Stroke(line((8.7, 12.6), (15.3, 12.6)), 1.7),
            Stroke(line((8.7, 16.2), (13.4, 16.2)), 1.7),
        ],
        // bolt.fill
        0xE945 => {
            let p = svg("M13.6 2.6 L5.6 13.4 H11.6 L10.4 21.4 L18.4 10.6 H12.4 Z");
            vec![Fill(p.clone()), Stroke(p, 1.3)]
        }
        _ => return None,
    })
}

// ---------- геометрия Direct2D ----------

unsafe fn emit(p: &Path, sink: &ID2D1GeometrySink) {
    let mut open = false;
    for s in &p.0 {
        match *s {
            Seg::M(x, y) => {
                if open {
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                }
                sink.BeginFigure(pt(x, y), D2D1_FIGURE_BEGIN_FILLED);
                open = true;
            }
            Seg::L(x, y) => sink.AddLine(pt(x, y)),
            Seg::Q(a, b, x, y) => sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: pt(a, b), point2: pt(x, y) }),
            Seg::C(a, b, c, d, x, y) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: pt(a, b), point2: pt(c, d), point3: pt(x, y) }),
            Seg::A { r, x, y, large, cw } => sink.AddArc(&D2D1_ARC_SEGMENT {
                point: pt(x, y),
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: if cw { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            }),
            Seg::Z => {
                if open {
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    open = false;
                }
            }
        }
    }
    if open {
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
    }
}

unsafe fn path_geo(f: &ID2D1Factory1, p: &Path) -> Option<ID2D1Geometry> {
    let geo = f.CreatePathGeometry().ok()?;
    let sink = geo.Open().ok()?;
    sink.SetFillMode(D2D1_FILL_MODE_WINDING);
    emit(p, &sink);
    sink.Close().ok()?;
    geo.cast().ok()
}

unsafe fn widen(f: &ID2D1Factory1, round: &ID2D1StrokeStyle1, p: &Path, w: f32) -> Option<ID2D1Geometry> {
    let src = path_geo(f, p)?;
    let geo = f.CreatePathGeometry().ok()?;
    let sink = geo.Open().ok()?;
    src.Widen(w, round, None, TOLERANCE, &sink).ok()?;
    sink.Close().ok()?;
    geo.cast().ok()
}

unsafe fn combine(f: &ID2D1Factory1, a: &ID2D1Geometry, b: &ID2D1Geometry, mode: D2D1_COMBINE_MODE) -> Option<ID2D1Geometry> {
    let geo = f.CreatePathGeometry().ok()?;
    let sink = geo.Open().ok()?;
    a.CombineWithGeometry(b, mode, None, TOLERANCE, &sink).ok()?;
    sink.Close().ok()?;
    geo.cast().ok()
}

/// Сливает части значка в одну геометрию на сетке 24×24.
pub(crate) unsafe fn build(f: &ID2D1Factory1, round: &ID2D1StrokeStyle1, code: u32) -> Option<ID2D1Geometry> {
    let mut acc: Option<ID2D1Geometry> = None;
    for part in parts(code)? {
        let (geo, cut) = match &part {
            Part::Fill(p) => (path_geo(f, p)?, false),
            Part::Stroke(p, w) => (widen(f, round, p, *w)?, false),
            Part::Cut(p) => (path_geo(f, p)?, true),
            Part::CutStroke(p, w) => (widen(f, round, p, *w)?, true),
        };
        acc = Some(match (acc.take(), cut) {
            (None, false) => combine(f, &geo, &geo, D2D1_COMBINE_MODE_UNION)?,
            // Вырез раньше первой заливки вырезать не из чего.
            (None, true) => continue,
            (Some(a), false) => combine(f, &a, &geo, D2D1_COMBINE_MODE_UNION)?,
            (Some(a), true) => combine(f, &a, &geo, D2D1_COMBINE_MODE_EXCLUDE)?,
        });
    }
    acc
}

/// Код глифа: строка из одного символа.
pub(crate) fn code_of(glyph: &str) -> Option<u32> {
    let mut it = glyph.chars();
    let c = it.next()?;
    it.next().is_none().then_some(c as u32)
}

/// Трансформация: квадрат сетки 24×24 по центру `r` со стороной `size`, поверх `base`.
pub(crate) fn place(base: &Matrix3x2, r: Rect, size: f32) -> Matrix3x2 {
    let k = size / 24.0;
    let (tx, ty) = (r.cx() - 12.0 * k, r.cy() - 12.0 * k);
    Matrix3x2 {
        M11: k * base.M11,
        M12: k * base.M12,
        M21: k * base.M21,
        M22: k * base.M22,
        M31: tx * base.M11 + ty * base.M21 + base.M31,
        M32: tx * base.M12 + ty * base.M22 + base.M32,
    }
}

/// Рисует значок `glyph` заливкой текущей кистью `brush`. `false` — векторного значка нет.
pub(crate) unsafe fn draw(g: &crate::gfx::Gfx, glyph: &str, r: Rect, size: f32) -> bool {
    let Some(code) = code_of(glyph) else { return false };
    let geo = {
        let mut cache = g.symbols.borrow_mut();
        cache.entry(code).or_insert_with(|| build(&g.d2d, &g.round, code)).clone()
    };
    let Some(geo) = geo else { return false };
    let mut base = Matrix3x2::default();
    g.dc.GetTransform(&mut base);
    g.dc.SetTransform(&place(&base, r, size));
    g.dc.FillGeometry(&geo, &g.brush, None);
    g.dc.SetTransform(&base);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_parses_absolute_relative_and_implicit_lines() {
        let p = svg("M1 2 L3 4 h2 v-1 Z m1 1 2 2");
        assert_eq!(p.0[0], Seg::M(1.0, 2.0));
        assert_eq!(p.0[1], Seg::L(3.0, 4.0));
        assert_eq!(p.0[2], Seg::L(5.0, 4.0));
        assert_eq!(p.0[3], Seg::L(5.0, 3.0));
        assert_eq!(p.0[4], Seg::Z);
        // После Z относительная точка отсчитывается от начала фигуры, пары после m — линии.
        assert_eq!(p.0[5], Seg::M(2.0, 3.0));
        assert_eq!(p.0[6], Seg::L(4.0, 5.0));
        let a = svg("M12 3.4 A8.6 8.6 0 0 0 12 20.6 Z");
        assert_eq!(a.0[1], Seg::A { r: 8.6, x: 12.0, y: 20.6, large: false, cw: false });
    }

    #[test]
    fn mirror_flips_x_and_sweep() {
        let m = arc(12.0, 12.0, 5.0, 0.0, 90.0).mirror();
        assert!(matches!(m.0[1], Seg::A { cw: false, .. }));
        let Seg::A { x, .. } = m.0[1] else { panic!() };
        assert!((x - 7.0).abs() < 1e-4);
    }

    #[test]
    fn every_icon_glyph_has_a_symbol() {
        use super::super::icon::*;
        for g in [
            WIFI, BLUETOOTH, BRIGHTNESS, MOON, SETTINGS, LOCK, SEARCH, VOLUME, MUTE, VOLUME1, VOLUME2, VOLUME3, PLAY,
            PLAY_SOLID, PAUSE, POWER, RESTART, COPY, LINK, PICTURE, PIN, DELETE, ADD, CHECK, CLOCK, CALENDAR, FOLDER,
            APPS, PREV, NEXT, REWIND, FAST_FORWARD, MUSIC, CLOSE, PASTE, LEFT, RIGHT, STAR, REFRESH, DOWN, UP, EDIT,
            STAR_FILL, SETTINGS_GEAR, RINGER, SPEAKER, CONTRAST, SUN, CLOUD, FORWARD, TEXT_DOC, BOLT,
        ] {
            assert!(parts(code_of(g).unwrap()).is_some(), "нет значка для U+{:04X}", code_of(g).unwrap());
        }
        assert!(code_of("ab").is_none() && code_of("").is_none());
    }

    #[test]
    fn place_centers_grid_in_rect() {
        let base = super::super::matrix(2.0, 10.0, 0.0);
        let m = place(&base, Rect::new(0.0, 0.0, 24.0, 24.0), 12.0);
        // Центр сетки (12, 12) → центр прямоугольника (12, 12) → масштаб 2 и сдвиг 10.
        let (x, y) = (12.0 * m.M11 + 12.0 * m.M21 + m.M31, 12.0 * m.M12 + 12.0 * m.M22 + m.M32);
        assert!((x - 34.0).abs() < 1e-4 && (y - 24.0).abs() < 1e-4);
    }

    /// Лист всех значков в PNG для глазной проверки: `ISLAND_DUMP=<папка> cargo test symbols_sheet -- --ignored`.
    #[test]
    #[ignore]
    fn symbols_sheet() {
        use windows::Win32::Graphics::Imaging::*;
        use windows::Win32::System::Com::*;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let dir = std::path::PathBuf::from(std::env::var("ISLAND_DUMP").unwrap());
            let f: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap();
            let round = f
                .CreateStrokeStyle(
                    &D2D1_STROKE_STYLE_PROPERTIES1 {
                        startCap: D2D1_CAP_STYLE_ROUND,
                        endCap: D2D1_CAP_STYLE_ROUND,
                        dashCap: D2D1_CAP_STYLE_ROUND,
                        lineJoin: D2D1_LINE_JOIN_ROUND,
                        miterLimit: 10.0,
                        dashStyle: D2D1_DASH_STYLE_SOLID,
                        dashOffset: 0.0,
                        transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
                    },
                    None,
                )
                .unwrap();
            let codes: Vec<u32> = (0xE000..0xF600).filter(|&c| parts(c).is_some()).collect();
            let cols = 10usize;
            // Три полосы: крупно 48 px, кегль острова (~14 px физ.) на тёмном и на светлом.
            let (cell_big, cell_s) = (64u32, 24u32);
            let rows = codes.len().div_ceil(cols) as u32;
            let (w, h) = (cols as u32 * cell_big, rows * cell_big + rows * cell_s * 2);
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let bmp = wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad).unwrap();
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                ..Default::default()
            };
            let rt = f.CreateWicBitmapRenderTarget(&bmp, &props).unwrap();
            let white = rt.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None).unwrap();
            let black = rt.CreateSolidColorBrush(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, None).unwrap();
            rt.BeginDraw();
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.11, g: 0.12, b: 0.13, a: 1.0 }));
            let band_light = rows * cell_big + rows * cell_s;
            rt.FillRectangle(&D2D_RECT_F { left: 0.0, top: band_light as f32, right: w as f32, bottom: h as f32 }, &white);
            let ident = super::super::matrix(1.0, 0.0, 0.0);
            for (i, &c) in codes.iter().enumerate() {
                let geo = build(&f, &round, c).unwrap_or_else(|| panic!("U+{c:04X}"));
                let (col, row) = ((i % cols) as f32, (i / cols) as f32);
                let big = Rect::new(col * cell_big as f32, row * cell_big as f32, cell_big as f32, cell_big as f32);
                rt.SetTransform(&place(&ident, big, 48.0));
                rt.FillGeometry(&geo, &white, None);
                for (band, brush) in [(rows * cell_big, &white), (band_light, &black)] {
                    let r = Rect::new(col * cell_big as f32, band as f32 + row * cell_s as f32, cell_big as f32, cell_s as f32);
                    rt.SetTransform(&place(&ident, r, 14.4));
                    rt.FillGeometry(&geo, brush, None);
                }
            }
            rt.SetTransform(&ident);
            rt.EndDraw(None, None).unwrap();
            let mut px = vec![0u8; (w * h * 4) as usize];
            bmp.CopyPixels(std::ptr::null(), w * 4, &mut px).unwrap();
            let out = dir.join("symbols.png");
            crate::img::save_png(&out, w, h, &px).expect("png");
            // save_png шифрует файл (DPAPI) — для просмотра возвращаем обычный PNG.
            let plain = crate::sys::unseal(&std::fs::read(&out).unwrap()).unwrap();
            std::fs::write(&out, plain).unwrap();
        }
    }
}
