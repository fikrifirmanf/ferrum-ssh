//! Liquid glass.
//!
//! The window is a stack, back to front:
//!
//! 1. **Field**: a painted backdrop of one base color and soft glows. The ember "heat" glow
//!    glides on a spring to whatever has focus ([`heat`]).
//! 2. **Panes**: chrome, panels and the dense terminal pane, drawn as real glass over the field
//!    by `embun-render`.
//! 3. **Controls**: buttons, segment lenses and chips, glass over the panes.
//! 4. **Overlays**: menus, palettes and dialogs, glass over everything below, each painted
//!    as its own crop so it sits above the content it covers.
//!
//! egui draws all text, icons and wells on top. Steps 2 to 4 run on the GPU at the end of the
//! frame ([`Gpu::end_frame`]); the result is the window background. A shape the GPU cannot take
//! (clipped by a scroll area, inside a popup, past the per-pass limit, or before the first GPU
//! frame) is painted by egui as tinted glass instead, so nothing ever leaks outside its pane.

use eframe::egui_wgpu::{self, wgpu};
use egui::{Color32, Context, Id, LayerId, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use embun_render::{Backdrop, Frame, GlassLayers, GlassParams, GlassRenderer, GlassShape};

use super::motion;

/// How thick a piece of glass is. Thicker glass hides more of what is behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tier {
    /// Title bar, status bar, toolbars: the thinnest glass.
    Chrome,
    /// Sidebar, inspector, lists, cards.
    Panel,
    /// Menus, palettes, dialogs, tooltips.
    Overlay,
    /// Terminal and other long-reading surfaces: nearly solid.
    Dense,
    /// A plain glass control.
    Control,
    /// A filled control in this color, such as the primary button.
    Prominent(Color32),
}

/// What the whole frame's glass looks like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// 0 = clear, 0.5 = default, 1 = tinted. The Settings slider.
    pub level: f32,
    pub reduce_transparency: bool,
    pub dark: bool,
    pub base: Color32,
    pub heat: Color32,
    pub heat_alpha: f32,
    pub steel: Color32,
    pub steel_alpha: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            level: embun_render::DEFAULT_LEVEL,
            reduce_transparency: false,
            dark: true,
            base: Color32::from_rgb(0x0b, 0x0b, 0x0e),
            heat: Color32::from_rgb(0xff, 0x8a, 0x4c),
            heat_alpha: 0.20,
            steel: Color32::from_rgb(0x8f, 0xb6, 0xdc),
            steel_alpha: 0.14,
        }
    }
}

/// Format of the textures egui shows.
const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Shapes per GPU pass; the rest are painted. embun allows 64.
const PER_PASS: usize = 60;

#[derive(Clone)]
struct FrameGlass {
    ready: bool,
    look: Look,
    /// Heat glow center and size, in points, after its spring.
    heat: Rect,
    panes: Vec<GlassShape>,
    controls: Vec<GlassShape>,
    overlays: Vec<GlassShape>,
    heat_target: Option<Rect>,
}

impl Default for FrameGlass {
    fn default() -> Self {
        Self {
            ready: false,
            look: Look::default(),
            heat: Rect::NOTHING,
            panes: Vec::new(),
            controls: Vec::new(),
            overlays: Vec::new(),
            heat_target: None,
        }
    }
}

#[derive(Clone, Copy)]
struct Textures {
    background: egui::TextureId,
    overlay: egui::TextureId,
}

fn frame_id() -> Id {
    Id::unique("liquid-glass-frame")
}

fn textures_id() -> Id {
    Id::unique("liquid-glass-textures")
}

fn heat_target_id() -> Id {
    Id::unique("liquid-glass-heat-target")
}

fn with_frame<R>(ctx: &Context, f: impl FnOnce(&mut FrameGlass) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<FrameGlass>(frame_id())))
}

/// The look of the current frame.
pub fn look(ctx: &Context) -> Look {
    with_frame(ctx, |f| f.look)
}

/// Starts a frame: clears last frame's shapes, moves the heat glow and paints the backdrop.
pub fn begin_frame(ctx: &Context, look: Look) {
    let textures = ctx.data(|d| d.get_temp::<Textures>(textures_id()));
    let screen = ctx.viewport_rect();
    let target = ctx
        .data(|d| d.get_temp::<Rect>(heat_target_id()))
        .unwrap_or_else(|| default_heat(screen));
    let id = Id::unique("liquid-glass-heat");
    let heat = Rect::from_center_size(
        pos2(
            motion::spring(ctx, id.with("x"), target.center().x, motion::Spring::SMOOTH),
            motion::spring(ctx, id.with("y"), target.center().y, motion::Spring::SMOOTH),
        ),
        vec2(
            motion::spring(ctx, id.with("w"), target.width(), motion::Spring::SMOOTH),
            motion::spring(ctx, id.with("h"), target.height(), motion::Spring::SMOOTH),
        ),
    );
    with_frame(ctx, |f| {
        *f = FrameGlass {
            ready: textures.is_some(),
            look,
            heat,
            ..FrameGlass::default()
        };
    });
    let painter = ctx.layer_painter(LayerId::background());
    match textures {
        Some(t) => {
            painter.add(
                egui::epaint::RectShape::filled(screen, 0, Color32::WHITE)
                    .with_texture(t.background, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0))),
            );
        }
        None => painter.extend(field_shapes(screen, heat, look)),
    }
}

fn default_heat(screen: Rect) -> Rect {
    Rect::from_center_size(
        screen.lerp_inside(vec2(0.3, 0.3)),
        vec2(screen.width() * 0.56, screen.height() * 0.52),
    )
}

/// Moves the ember heat glow behind `rect` (the focused host, the active tab, an open dialog).
/// The last call in a frame wins; the glow glides there on a spring.
pub fn heat(ctx: &Context, rect: Rect) {
    let screen = ctx.viewport_rect();
    let size = vec2(
        (rect.width() * 1.6).clamp(screen.width() * 0.3, screen.width() * 0.7),
        (rect.height() * 3.0).clamp(screen.height() * 0.3, screen.height() * 0.6),
    );
    let glow = Rect::from_center_size(rect.center(), size);
    ctx.data_mut(|d| d.insert_temp(heat_target_id(), glow));
    with_frame(ctx, |f| f.heat_target = Some(glow));
}

fn to_shape(rect: Rect, radius: f32, tier: Tier, look: &Look, group: u32) -> GlassShape {
    let mut shape = GlassShape::new(
        [rect.center().x, rect.center().y],
        [rect.width() / 2.0, rect.height() / 2.0],
        radius,
    )
    .appearance(if look.dark { 1.0 } else { 0.0 })
    .group(group);
    let rgb = |c: Color32| {
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
        ]
    };
    shape = match tier {
        Tier::Prominent(c) => shape.tint(rgb(c), 1.0),
        Tier::Dense => shape.tint(rgb(dense_color(look)), 0.9),
        // Panes carry a little of the theme's ink so text sits on calm glass, as proposed.
        Tier::Panel => shape.tint(rgb(dense_color(look)), if look.dark { 0.55 } else { 0.35 }),
        Tier::Chrome => shape.tint(rgb(dense_color(look)), if look.dark { 0.40 } else { 0.25 }),
        Tier::Overlay => shape.tint(rgb(overlay_color(look)), 0.35),
        _ => shape,
    };
    shape
}

fn dense_color(look: &Look) -> Color32 {
    if look.dark {
        Color32::from_rgb(12, 12, 14)
    } else {
        Color32::from_rgb(251, 250, 248)
    }
}

fn overlay_color(look: &Look) -> Color32 {
    if look.dark {
        Color32::from_rgb(36, 36, 42)
    } else {
        Color32::from_rgb(255, 255, 255)
    }
}

fn gpu_can_take(ui: &Ui, rect: Rect) -> bool {
    ui.layer_id().order == egui::Order::Background && ui.clip_rect().expand(0.5).contains_rect(rect)
}

/// A pane of glass behind `rect`: chrome, panel or dense. Call before drawing its content.
pub fn pane(ui: &Ui, rect: Rect, radius: f32, tier: Tier) {
    let ctx = ui.ctx();
    let taken = gpu_can_take(ui, rect)
        && with_frame(ctx, |f| {
            if !f.ready || f.panes.len() >= PER_PASS {
                return false;
            }
            let shape = to_shape(rect, radius, tier, &f.look, f.panes.len() as u32);
            f.panes.push(shape);
            true
        });
    if !taken {
        ui.painter()
            .extend(fallback(rect, radius, tier, &look(ctx)));
    }
}

/// A glass control face. `swell` (0..1, from a spring) grows it the way pressed glass bulges.
/// Returns the face rect actually drawn.
pub fn control(ui: &Ui, rect: Rect, radius: f32, tier: Tier, swell: f32) -> Rect {
    let ctx = ui.ctx();
    let face = rect.expand(swell * 2.5);
    let taken = gpu_can_take(ui, face)
        && with_frame(ctx, |f| {
            if !f.ready || f.controls.len() >= PER_PASS {
                return false;
            }
            let shape = to_shape(
                face,
                radius + swell * 2.5,
                tier,
                &f.look,
                1000 + f.controls.len() as u32,
            );
            f.controls.push(shape);
            true
        });
    if !taken {
        ui.painter()
            .extend(fallback(face, radius + swell * 2.5, tier, &look(ctx)));
    }
    face
}

/// Glass behind a whole panel. Call first inside a panel's `show`, passing the frame's inner
/// margin, so the pane covers the panel and not only its content area.
pub fn pane_behind(ui: &Ui, margin: egui::Margin, radius: f32, tier: Tier) {
    let r = ui.max_rect();
    let rect = Rect::from_min_max(
        r.min - vec2(margin.left as f32, margin.top as f32),
        r.max + vec2(margin.right as f32, margin.bottom as f32),
    );
    pane(ui, rect, radius, tier);
}

/// The faint wash a glass control gains on hover.
pub fn hover_wash(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_white_alpha(16)
    } else {
        Color32::from_white_alpha(110)
    }
}

/// The selection lens: one raised glass capsule per `id` that glides between targets like a
/// drop of liquid, stretching along its travel. Call it from whichever item is selected.
pub fn lens(ui: &Ui, id: Id, target: Rect, radius: f32) -> Rect {
    let rect = motion::liquid_rect(ui.ctx(), id, target);
    control(ui, rect, radius, Tier::Control, 0.0)
}

/// Glass shapes for an overlay at `rect`, to place under its content.
fn overlay_shapes(ctx: &Context, rect: Rect, radius: f32) -> Vec<Shape> {
    let textures = ctx.data(|d| d.get_temp::<Textures>(textures_id()));
    let screen = ctx.viewport_rect();
    let look = look(ctx);
    let taken = textures.filter(|_| {
        with_frame(ctx, |f| {
            if !f.ready || f.overlays.len() >= PER_PASS {
                return false;
            }
            let shape = to_shape(
                rect,
                radius,
                Tier::Overlay,
                &f.look,
                2000 + f.overlays.len() as u32,
            );
            f.overlays.push(shape);
            true
        })
    });
    match taken {
        Some(t) => {
            let uv = Rect::from_min_max(
                pos2(
                    (rect.min.x - screen.min.x) / screen.width(),
                    (rect.min.y - screen.min.y) / screen.height(),
                ),
                pos2(
                    (rect.max.x - screen.min.x) / screen.width(),
                    (rect.max.y - screen.min.y) / screen.height(),
                ),
            );
            vec![
                shadow(rect, radius, &look, Tier::Overlay),
                egui::epaint::RectShape::filled(rect, radius, Color32::WHITE)
                    .with_texture(t.overlay, uv)
                    .into(),
            ]
        }
        None => fallback(rect, radius, Tier::Overlay, &look),
    }
}

/// Lays out `add` inside an overlay pane of glass: menus, palettes, dialogs, tooltips.
pub fn overlay_frame<R>(
    ui: &mut Ui,
    radius: f32,
    margin: egui::Margin,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let slot = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::NONE.inner_margin(margin).show(ui, add);
    let rect = inner.response.rect;
    let shapes = overlay_shapes(ui.ctx(), rect, radius);
    ui.painter().set(slot, Shape::Vec(shapes));
    inner
}

/// Lays out `add` on a pane of glass with `margin` inside it. Use for panels and cards.
pub fn pane_frame<R>(
    ui: &mut Ui,
    tier: Tier,
    radius: f32,
    margin: egui::Margin,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let slot = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::NONE.inner_margin(margin).show(ui, add);
    let rect = inner.response.rect;
    let ctx = ui.ctx().clone();
    let taken = gpu_can_take(ui, rect)
        && with_frame(&ctx, |f| {
            if !f.ready || f.panes.len() >= PER_PASS {
                return false;
            }
            let shape = to_shape(rect, radius, tier, &f.look, f.panes.len() as u32);
            f.panes.push(shape);
            true
        });
    if !taken {
        ui.painter()
            .set(slot, Shape::Vec(fallback(rect, radius, tier, &look(&ctx))));
    }
    inner
}

// ---------- Painted glass ----------

/// Fill of each tier when egui paints the glass itself.
pub fn tier_fill(tier: Tier, look: &Look) -> Color32 {
    let (rgb, alpha) = match (tier, look.dark) {
        (Tier::Chrome, true) => ([28, 28, 33], 0.55),
        (Tier::Panel, true) => ([30, 30, 35], 0.62),
        (Tier::Overlay, true) => ([36, 36, 42], 0.86),
        (Tier::Dense, true) => ([12, 12, 14], 0.92),
        (Tier::Control, true) => ([255, 255, 255], 0.08),
        (Tier::Chrome, false) => ([255, 255, 255], 0.50),
        (Tier::Panel, false) => ([255, 255, 255], 0.60),
        (Tier::Overlay, false) => ([255, 255, 255], 0.86),
        (Tier::Dense, false) => ([251, 250, 248], 0.94),
        (Tier::Control, false) => ([255, 255, 255], 0.70),
        (Tier::Prominent(c), _) => ([c.r(), c.g(), c.b()], 1.0),
    };
    // The level slider thickens painted glass the same way it thickens real glass.
    let alpha = if look.reduce_transparency {
        1.0
    } else {
        (alpha * (0.85 + 0.3 * look.level)).min(1.0)
    };
    let c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    if alpha >= 1.0 {
        // Solid: the tint composited over the base color.
        let a = match (tier, look.dark) {
            (Tier::Control, true) => 0.08,
            (Tier::Prominent(_), _) => 1.0,
            _ => 0.75,
        };
        return look.base.lerp_to_gamma(c, a);
    }
    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (alpha * 255.0) as u8)
}

fn shadow(rect: Rect, radius: f32, look: &Look, tier: Tier) -> Shape {
    let (offset, blur, alpha) = match tier {
        Tier::Overlay => (24, 60, if look.dark { 128 } else { 51 }),
        Tier::Panel => (12, 32, if look.dark { 80 } else { 26 }),
        _ => (0, 0, 0),
    };
    if alpha == 0 {
        return Shape::Noop;
    }
    // egui's shadow mesh folds into diagonal streaks when the blur is wider than the shape,
    // so a small overlay (a rail label, a toast) gets a shadow scaled to its size.
    let fit = (rect.size().min_elem() * 0.8).max(4.0) as u8;
    let blur = blur.min(fit);
    let offset = offset.min(blur / 2) as i8;
    egui::epaint::Shadow {
        offset: [0, offset],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(alpha),
    }
    .as_shape(rect, radius)
    .into()
}

/// Glass painted by egui: tint, shadow, a one-point rim and a highlight along the top edge.
pub fn fallback(rect: Rect, radius: f32, tier: Tier, look: &Look) -> Vec<Shape> {
    let radius = radius.min(rect.height() / 2.0).min(rect.width() / 2.0);
    let fill = tier_fill(tier, look);
    let (rim, top) = if look.dark {
        (Color32::from_white_alpha(22), Color32::from_white_alpha(40))
    } else {
        (
            Color32::from_black_alpha(18),
            Color32::from_white_alpha(230),
        )
    };
    let mut shapes = vec![
        shadow(rect, radius, look, tier),
        Shape::rect_filled(rect, radius, fill),
        Shape::rect_stroke(rect, radius, Stroke::new(1.0, rim), StrokeKind::Inside),
    ];
    if rect.width() > radius * 2.0 + 2.0 {
        shapes.push(Shape::hline(
            (rect.left() + radius)..=(rect.right() - radius),
            rect.top() + 1.0,
            Stroke::new(1.0, top),
        ));
    }
    shapes
}

/// A card inside a pane: glass over glass, painted so it stays put while its list scrolls.
pub fn card_shapes(rect: Rect, radius: f32, look: &Look) -> Vec<Shape> {
    let (fill, rim, top) = if look.reduce_transparency {
        let fill = if look.dark {
            look.base.lerp_to_gamma(Color32::WHITE, 0.10)
        } else {
            Color32::WHITE
        };
        (
            fill,
            Color32::from_gray(if look.dark { 70 } else { 210 }),
            Color32::TRANSPARENT,
        )
    } else if look.dark {
        (
            Color32::from_white_alpha(9),
            Color32::from_white_alpha(20),
            Color32::from_white_alpha(34),
        )
    } else {
        (
            Color32::from_white_alpha(115),
            Color32::from_black_alpha(14),
            Color32::from_white_alpha(235),
        )
    };
    let mut shapes = vec![
        Shape::rect_filled(rect, radius, fill),
        Shape::rect_stroke(rect, radius, Stroke::new(1.0, rim), StrokeKind::Inside),
    ];
    if rect.width() > radius * 2.0 + 2.0 && top.a() > 0 {
        shapes.push(Shape::hline(
            (rect.left() + radius)..=(rect.right() - radius),
            rect.top() + 1.0,
            Stroke::new(1.0, top),
        ));
    }
    shapes
}

/// An `egui::Frame` that looks like [`card_shapes`], for code that lays out with frames.
pub fn card_frame(ctx: &Context, radius: u8) -> egui::Frame {
    let look = look(ctx);
    let (fill, rim) = if look.reduce_transparency {
        if look.dark {
            (
                look.base.lerp_to_gamma(Color32::WHITE, 0.10),
                Color32::from_gray(70),
            )
        } else {
            (Color32::WHITE, Color32::from_gray(210))
        }
    } else if look.dark {
        (Color32::from_white_alpha(9), Color32::from_white_alpha(20))
    } else {
        (
            Color32::from_white_alpha(115),
            Color32::from_black_alpha(14),
        )
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, rim))
        .corner_radius(radius)
}

/// An `egui::Frame` for recessed, code-like content (previews, command output).
pub fn well_frame(ctx: &Context, radius: u8) -> egui::Frame {
    egui::Frame::new()
        .fill(well_fill(&look(ctx)))
        .corner_radius(radius)
}

/// Fill of a recessed well: inputs, segment tracks, switch tracks, progress tracks.
pub fn well_fill(look: &Look) -> Color32 {
    match (look.dark, look.reduce_transparency) {
        (true, false) => Color32::from_black_alpha(64),
        (true, true) => look.base.lerp_to_gamma(Color32::BLACK, 0.3),
        (false, false) => Color32::from_white_alpha(190),
        (false, true) => Color32::WHITE,
    }
}

/// The well's inner shadow: a soft dark line just inside the top edge.
pub fn well_shadow(rect: Rect, radius: f32, look: &Look) -> Shape {
    let c = if look.dark {
        Color32::from_black_alpha(90)
    } else {
        Color32::from_black_alpha(22)
    };
    if rect.width() <= radius * 2.0 + 2.0 {
        return Shape::Noop;
    }
    Shape::hline(
        (rect.left() + radius)..=(rect.right() - radius),
        rect.top() + 1.0,
        Stroke::new(1.0, c),
    )
}

/// The field painted by egui, for the first frame and for machines without a GPU frame yet.
fn field_shapes(screen: Rect, heat: Rect, look: Look) -> Vec<Shape> {
    let mut shapes = vec![Shape::rect_filled(screen, 0, look.base)];
    for glow in glows(screen, heat, &look) {
        shapes.push(glow_mesh(glow).into());
    }
    shapes
}

#[derive(Clone, Copy, PartialEq)]
struct Glow {
    center: Pos2,
    radius: egui::Vec2,
    color: Color32,
    alpha: f32,
}

fn glows(screen: Rect, heat: Rect, look: &Look) -> [Glow; 3] {
    [
        Glow {
            center: heat.center(),
            radius: heat.size() / 2.0,
            color: look.heat,
            alpha: look.heat_alpha,
        },
        Glow {
            center: screen.lerp_inside(vec2(0.08, 0.95)),
            radius: screen.size() * 0.30,
            color: look.steel,
            alpha: look.steel_alpha,
        },
        Glow {
            center: screen.lerp_inside(vec2(0.95, 0.10)),
            radius: screen.size() * 0.25,
            color: look.steel,
            alpha: look.steel_alpha * 0.7,
        },
    ]
}

fn glow_mesh(g: Glow) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    let ring = |t: f32| {
        let a = (1.0 - t / 0.7).clamp(0.0, 1.0) * g.alpha;
        Color32::from_rgba_unmultiplied(g.color.r(), g.color.g(), g.color.b(), (a * 255.0) as u8)
    };
    const SEGMENTS: usize = 48;
    const RINGS: [f32; 5] = [0.0, 0.18, 0.36, 0.54, 0.72];
    for (r, t) in RINGS.iter().enumerate() {
        for s in 0..SEGMENTS {
            let angle = s as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let p = g.center + vec2(angle.cos() * g.radius.x, angle.sin() * g.radius.y) * *t;
            mesh.colored_vertex(p, ring(*t));
            if r > 0 {
                let i = (r * SEGMENTS + s) as u32;
                let j = (r * SEGMENTS + (s + 1) % SEGMENTS) as u32;
                let pi = i - SEGMENTS as u32;
                let pj = j - SEGMENTS as u32;
                mesh.add_triangle(pi, pj, i);
                mesh.add_triangle(pj, j, i);
            }
        }
    }
    mesh
}

/// The field as RGBA8 sRGB pixels, `w`×`h`, covering `screen`.
fn field_pixels(w: u32, h: u32, screen: Rect, heat: Rect, look: &Look) -> Vec<u8> {
    let glows = glows(screen, heat, look);
    let base = [
        look.base.r() as f32,
        look.base.g() as f32,
        look.base.b() as f32,
    ];
    let mut out = vec![0u8; (w * h * 4) as usize];
    let sx = screen.width() / w as f32;
    let sy = screen.height() / h as f32;
    for y in 0..h {
        for x in 0..w {
            let p = pos2(
                screen.left() + (x as f32 + 0.5) * sx,
                screen.top() + (y as f32 + 0.5) * sy,
            );
            let mut c = base;
            for g in &glows {
                let d = vec2(
                    (p.x - g.center.x) / g.radius.x.max(1.0),
                    (p.y - g.center.y) / g.radius.y.max(1.0),
                )
                .length();
                let a = (1.0 - d / 0.7).clamp(0.0, 1.0) * g.alpha;
                let gc = [g.color.r() as f32, g.color.g() as f32, g.color.b() as f32];
                for i in 0..3 {
                    c[i] += (gc[i] - c[i]) * a;
                }
            }
            // A little ordered dither keeps the soft glows from banding.
            let dither = (((x * 7 + y * 13) % 16) as f32 / 16.0 - 0.5) * 1.2;
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                out[i + k] = (c[k] + dither).round().clamp(0.0, 255.0) as u8;
            }
            out[i + 3] = 255;
        }
    }
    out
}

// ---------- GPU ----------

struct Target {
    backdrop: Backdrop,
    view: wgpu::TextureView,
}

fn level0(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: 0,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn blank_backdrop(renderer: &GlassRenderer, rs: &egui_wgpu::RenderState, size: [u32; 2]) -> Target {
    let pixels = vec![0u8; (size[0] * size[1] * 4) as usize];
    let backdrop = renderer.create_backdrop(&rs.device, &rs.queue, &pixels, size[0], size[1]);
    let view = level0(backdrop.texture());
    Target { backdrop, view }
}

#[derive(PartialEq, Clone)]
struct RenderKey {
    sizes: ([u32; 2], [u32; 2]),
    look: Look,
    focused: bool,
    heat: [i32; 4],
    panes: Vec<GlassShape>,
    controls: Vec<GlassShape>,
    overlays: Vec<GlassShape>,
}

/// GPU state for the glass, owned by the app.
#[derive(Default)]
pub struct Gpu {
    /// Draws into backdrops (sRGB) that later passes read from.
    renderer: Option<GlassRenderer>,
    /// Draws the layers egui shows. egui reads textures as gamma-encoded values, so these
    /// targets are plain `Rgba8Unorm` and embun writes gamma into them.
    display: Option<GlassRenderer>,
    field: Option<(Target, [u32; 2], [i32; 4], Look)>,
    panes: Option<(Target, [u32; 2])>,
    composite: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    overlay: Option<(wgpu::Texture, wgpu::TextureView, [u32; 2])>,
    textures: Option<Textures>,
    last: Option<RenderKey>,
    /// Focus on the last frame, to read the system preference again when focus returns.
    was_focused: Option<bool>,
}

impl Gpu {
    /// Renders this frame's glass into the textures the next paint shows. Call last in `ui`.
    pub fn end_frame(&mut self, ctx: &Context, frame: &eframe::Frame) {
        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        let f = with_frame(ctx, std::mem::take);
        if std::env::var_os("FERRUM_GLASS_DEBUG").is_some() {
            eprintln!(
                "glass: ready={} panes={} controls={} overlays={} heat={:?}",
                f.ready,
                f.panes.len(),
                f.controls.len(),
                f.overlays.len(),
                f.heat
            );
        }
        let screen = ctx.viewport_rect();
        let ppp = ctx.pixels_per_point();
        if screen.width() < 1.0 || screen.height() < 1.0 {
            return;
        }
        let logical = [screen.width().ceil() as u32, screen.height().ceil() as u32];
        let physical = [
            (screen.width() * ppp).round().max(1.0) as u32,
            (screen.height() * ppp).round().max(1.0) as u32,
        ];
        let field_size = [logical[0].div_ceil(4).max(1), logical[1].div_ceil(4).max(1)];
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused && self.was_focused == Some(false) {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let reduce = system_query();
                if SYSTEM_REDUCED.swap(reduce, std::sync::atomic::Ordering::Relaxed) != reduce {
                    ctx.request_repaint();
                }
            });
        }
        self.was_focused = Some(focused);
        let heat_key = [
            f.heat.min.x.round() as i32,
            f.heat.min.y.round() as i32,
            f.heat.max.x.round() as i32,
            f.heat.max.y.round() as i32,
        ];
        let key = RenderKey {
            sizes: (logical, physical),
            look: f.look,
            focused,
            heat: heat_key,
            panes: f.panes.clone(),
            controls: f.controls.clone(),
            overlays: f.overlays.clone(),
        };
        if self.last.as_ref() == Some(&key) && self.textures.is_some() {
            return;
        }

        let renderer = self
            .renderer
            .get_or_insert_with(|| GlassRenderer::new(&rs.device, Backdrop::FORMAT));
        let display = self
            .display
            .get_or_insert_with(|| GlassRenderer::new(&rs.device, DISPLAY_FORMAT));
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ferrum glass"),
            });

        // 1. The field, low resolution: it is blurred anyway.
        let field_stale = !matches!(&self.field, Some((_, s, h, l)) if *s == field_size && *h == heat_key && *l == f.look);
        if field_stale {
            let pixels = field_pixels(field_size[0], field_size[1], screen, f.heat, &f.look);
            match &self.field {
                Some((t, s, _, _)) if *s == field_size => {
                    rs.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: t.backdrop.texture(),
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        &pixels,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(field_size[0] * 4),
                            rows_per_image: Some(field_size[1]),
                        },
                        wgpu::Extent3d {
                            width: field_size[0],
                            height: field_size[1],
                            depth_or_array_layers: 1,
                        },
                    );
                    renderer.regenerate_mips(&mut encoder, &t.backdrop);
                }
                _ => {
                    let backdrop = renderer.create_backdrop(
                        &rs.device,
                        &rs.queue,
                        &pixels,
                        field_size[0],
                        field_size[1],
                    );
                    let view = level0(backdrop.texture());
                    self.field = Some((Target { backdrop, view }, field_size, heat_key, f.look));
                }
            }
            if let Some(field) = &mut self.field {
                field.2 = heat_key;
                field.3 = f.look;
            }
        }
        if !matches!(&self.panes, Some((_, s)) if *s == logical) {
            self.panes = Some((blank_backdrop(renderer, rs, logical), logical));
        }
        let resized = !matches!(&self.composite, Some((_, _, s)) if *s == physical);
        if resized {
            let target = |label| {
                let texture = rs.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: physical[0],
                        height: physical[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DISPLAY_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                (texture, view, physical)
            };
            self.composite = Some(target("ferrum glass"));
            self.overlay = Some(target("ferrum glass overlays"));
        }
        let (
            Some((field, _, _, _)),
            Some((panes, _)),
            Some((_, composite_view, _)),
            Some((_, overlay_view, _)),
        ) = (&self.field, &self.panes, &self.composite, &self.overlay)
        else {
            return;
        };

        let params = |layers: GlassLayers| {
            if focused {
                GlassParams::from_level(f.look.level, layers, f.look.reduce_transparency)
            } else {
                GlassParams::unfocused(f.look.level, layers, f.look.reduce_transparency)
            }
        };
        let viewport = [screen.width(), screen.height()];
        let quiet = GlassLayers {
            merge: false,
            ..GlassLayers::default()
        };
        // 2. Panes over the field.
        renderer.render(
            &rs.queue,
            &mut encoder,
            &panes.view,
            &field.backdrop,
            &Frame {
                viewport,
                scale_factor: 1.0,
                backdrop_scale: field_size[0] as f32 / screen.width(),
                scroll: 0.0,
                shapes: &f.panes,
                params: params(GlassLayers {
                    shadow: true,
                    ..quiet
                }),
            },
        );
        renderer.regenerate_mips(&mut encoder, &panes.backdrop);
        // 3. Controls over the panes, at full resolution: this is the window background.
        display.render(
            &rs.queue,
            &mut encoder,
            composite_view,
            &panes.backdrop,
            &Frame {
                viewport,
                scale_factor: ppp,
                backdrop_scale: logical[0] as f32 / screen.width(),
                scroll: 0.0,
                shapes: &f.controls,
                params: params(GlassLayers {
                    merge: true,
                    ..quiet
                }),
            },
        );
        // 4. Overlays over the panes. They read the same backdrop as step 3, whose uniforms
        // must reach the GPU first.
        rs.queue.submit([encoder.finish()]);
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ferrum glass overlays"),
            });
        if !f.overlays.is_empty() {
            display.render(
                &rs.queue,
                &mut encoder,
                overlay_view,
                &panes.backdrop,
                &Frame {
                    viewport,
                    scale_factor: ppp,
                    backdrop_scale: logical[0] as f32 / screen.width(),
                    scroll: 0.0,
                    shapes: &f.overlays,
                    params: params(GlassLayers {
                        rim: true,
                        shadow: true,
                        ..quiet
                    }),
                },
            );
        }
        rs.queue.submit([encoder.finish()]);

        let mut egui_renderer = rs.renderer.write();
        match self.textures {
            Some(t) if resized => {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    composite_view,
                    wgpu::FilterMode::Linear,
                    t.background,
                );
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    overlay_view,
                    wgpu::FilterMode::Linear,
                    t.overlay,
                );
            }
            Some(_) => {}
            None => {
                let t = Textures {
                    background: egui_renderer.register_native_texture(
                        &rs.device,
                        composite_view,
                        wgpu::FilterMode::Linear,
                    ),
                    overlay: egui_renderer.register_native_texture(
                        &rs.device,
                        overlay_view,
                        wgpu::FilterMode::Linear,
                    ),
                };
                self.textures = Some(t);
                ctx.data_mut(|d| d.insert_temp(textures_id(), t));
                // The first GPU frame is ready; draw again so it replaces the painted field.
                ctx.request_repaint();
            }
        }
        self.last = Some(key);
    }
}

// ---------- System preference ----------

static SYSTEM_REDUCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Reads the operating system's Reduce transparency preference away from the UI thread.
/// `Gpu::end_frame` reads it again whenever the window regains focus, so a change made in
/// System Settings applies without a restart.
pub fn detect_system_preference() {
    std::thread::spawn(|| {
        SYSTEM_REDUCED.store(system_query(), std::sync::atomic::Ordering::Relaxed);
    });
}

pub fn system_reduces_transparency() -> bool {
    SYSTEM_REDUCED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Asks AppKit, as embun does. `defaults read` misses the setting when the key was never
/// written and can lag behind System Settings.
#[cfg(target_os = "macos")]
fn system_query() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
}

#[cfg(target_os = "windows")]
fn system_query() -> bool {
    std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "/v",
            "EnableTransparency",
        ])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("0x0"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system_query() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ready: bool, add: impl FnMut(&mut Ui)) -> Context {
        let ctx = Context::default();
        if ready {
            ctx.data_mut(|d| {
                d.insert_temp(
                    textures_id(),
                    Textures {
                        background: egui::TextureId::User(1),
                        overlay: egui::TextureId::User(2),
                    },
                )
            });
        }
        let mut add = add;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            begin_frame(ui.ctx(), Look::default());
            add(ui);
        });
        out.textures_delta.clear();
        ctx
    }

    #[test]
    fn without_a_gpu_frame_glass_is_painted_and_never_queued() {
        let ctx = frame(false, |ui| {
            pane(
                ui,
                Rect::from_min_size(pos2(8.0, 8.0), vec2(200.0, 400.0)),
                14.0,
                Tier::Panel,
            );
        });
        assert!(with_frame(&ctx, |f| f.panes.is_empty()));
    }

    #[test]
    fn with_a_gpu_frame_panes_and_controls_are_queued_for_embun() {
        let ctx = frame(true, |ui| {
            pane(
                ui,
                Rect::from_min_size(pos2(8.0, 8.0), vec2(200.0, 400.0)),
                14.0,
                Tier::Panel,
            );
            control(
                ui,
                Rect::from_min_size(pos2(20.0, 20.0), vec2(80.0, 32.0)),
                16.0,
                Tier::Prominent(Color32::from_rgb(255, 138, 76)),
                0.0,
            );
        });
        with_frame(&ctx, |f| {
            assert_eq!(f.panes.len(), 1);
            assert_eq!(f.controls.len(), 1);
            assert_eq!(
                f.controls[0].tint[3], 1.0,
                "primary buttons are fully tinted glass"
            );
        });
    }

    #[test]
    fn glass_clipped_by_a_scroll_area_is_painted_instead_of_leaking() {
        let ctx = frame(true, |ui| {
            ui.set_clip_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)));
            control(
                ui,
                Rect::from_min_size(pos2(60.0, 80.0), vec2(80.0, 32.0)),
                16.0,
                Tier::Control,
                0.0,
            );
        });
        assert!(with_frame(&ctx, |f| f.controls.is_empty()));
    }

    #[test]
    fn reduce_transparency_paints_every_tier_solid() {
        let look = Look {
            reduce_transparency: true,
            ..Look::default()
        };
        for tier in [Tier::Chrome, Tier::Panel, Tier::Overlay, Tier::Dense] {
            assert_eq!(tier_fill(tier, &look).a(), 255, "{tier:?}");
        }
    }

    #[test]
    fn the_level_slider_thickens_painted_glass() {
        let clear = tier_fill(
            Tier::Panel,
            &Look {
                level: 0.0,
                ..Look::default()
            },
        );
        let tinted = tier_fill(
            Tier::Panel,
            &Look {
                level: 1.0,
                ..Look::default()
            },
        );
        assert!(tinted.a() > clear.a());
    }

    #[test]
    fn the_heat_glow_brightens_the_field_where_it_sits() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let heat = Rect::from_center_size(pos2(100.0, 50.0), vec2(160.0, 80.0));
        let px = field_pixels(100, 50, screen, heat, &Look::default());
        let at = |x: u32, y: u32| px[((y * 100 + x) * 4) as usize];
        assert!(
            at(25, 12) > at(75, 12) + 20,
            "heat center should be warmer than the far side"
        );
    }

    /// The AppKit query runs off the main thread, as `detect_system_preference` does.
    #[cfg(target_os = "macos")]
    #[test]
    fn system_transparency_query_runs_off_the_main_thread() {
        let reduce = std::thread::spawn(system_query)
            .join()
            .expect("query thread");
        eprintln!("system Reduce transparency: {reduce}");
    }
}
