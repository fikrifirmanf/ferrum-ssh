//! Terminal color schemes and the application's egui visual style.

use egui::{Color32, CornerRadius, Stroke, Visuals};

use crate::model::UiTheme;

#[derive(Clone, Debug)]
pub struct TermScheme {
    pub name: &'static str,
    pub bg: Color32,
    pub fg: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub ansi: [Color32; 16],
}

const fn h(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

macro_rules! scheme {
    ($name:expr, $bg:expr, $fg:expr, $cur:expr, $sel:expr, [$($c:expr),* $(,)?]) => {
        TermScheme { name: $name, bg: h($bg), fg: h($fg), cursor: h($cur), selection: h($sel), ansi: [$(h($c)),*] }
    };
}

pub fn schemes() -> Vec<TermScheme> {
    vec![
        scheme!(
            "Ferrum Night",
            0x0e0e10,
            0xe8e6e1,
            0xff8a4c,
            0x2e2019,
            [
                0x1c1c1f, 0xf7808c, 0x6fd6a0, 0xebc85a, 0x7fb0e8, 0xc9a0e8, 0x63d2d8, 0xc8c6c0,
                0x6e6e76, 0xff9aa4, 0x8ce4b6, 0xf5d97f, 0x9cc4ef, 0xdab8f0, 0x86e0e4, 0xf2f0eb
            ]
        ),
        scheme!(
            "Ferrum Day",
            0xfbfaf7,
            0x1c1b19,
            0xb5430f,
            0xf7e4d8,
            [
                0x1c1b19, 0xbe1e3a, 0x14774a, 0x7f6000, 0x28608f, 0x7d3f98, 0x0b7680, 0x8a867d,
                0x5c5a54, 0xd8324f, 0x1d8f5b, 0x9a7600, 0x3576ab, 0x9353ad, 0x118c97, 0x1c1b19
            ]
        ),
        scheme!(
            "Dracula",
            0x282a36,
            0xf8f8f2,
            0xf8f8f2,
            0x44475a,
            [
                0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff
            ]
        ),
        scheme!(
            "Nord",
            0x2e3440,
            0xd8dee9,
            0xd8dee9,
            0x434c5e,
            [
                0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4
            ]
        ),
        scheme!(
            "Solarized Dark",
            0x002b36,
            0x839496,
            0x93a1a1,
            0x073642,
            [
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3
            ]
        ),
        scheme!(
            "Solarized Light",
            0xfdf6e3,
            0x657b83,
            0x586e75,
            0xeee8d5,
            [
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3
            ]
        ),
        scheme!(
            "Gruvbox Dark",
            0x282828,
            0xebdbb2,
            0xebdbb2,
            0x504945,
            [
                0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
                0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2
            ]
        ),
        scheme!(
            "One Dark",
            0x282c34,
            0xabb2bf,
            0x528bff,
            0x3e4451,
            [
                0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                0x5c6370, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff
            ]
        ),
        scheme!(
            "Monokai",
            0x272822,
            0xf8f8f2,
            0xf8f8f0,
            0x49483e,
            [
                0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2,
                0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5
            ]
        ),
        scheme!(
            "Tokyo Night",
            0x1a1b26,
            0xc0caf5,
            0xc0caf5,
            0x33467c,
            [
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5
            ]
        ),
        scheme!(
            "Catppuccin Mocha",
            0x1e1e2e,
            0xcdd6f4,
            0xf5e0dc,
            0x45475a,
            [
                0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
                0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8
            ]
        ),
        scheme!(
            "Catppuccin Latte",
            0xeff1f5,
            0x4c4f69,
            0xdc8a78,
            0xccd0da,
            [
                0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
                0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc
            ]
        ),
        scheme!(
            "GitHub Dark",
            0x0d1117,
            0xc9d1d9,
            0x58a6ff,
            0x264f78,
            [
                0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4,
                0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff
            ]
        ),
        scheme!(
            "GitHub Light",
            0xffffff,
            0x24292f,
            0x0969da,
            0xddf4ff,
            [
                0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
                0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f
            ]
        ),
        scheme!(
            "Night Owl",
            0x011627,
            0xd6deeb,
            0x80a4c2,
            0x1d3b53,
            [
                0x011627, 0xef5350, 0x22da6e, 0xc5e478, 0x82aaff, 0xc792ea, 0x21c7a8, 0xffffff,
                0x575656, 0xef5350, 0x22da6e, 0xffeb95, 0x82aaff, 0xc792ea, 0x7fdbca, 0xffffff
            ]
        ),
        scheme!(
            "Ayu Mirage",
            0x1f2430,
            0xcbccc6,
            0xffcc66,
            0x34455a,
            [
                0x191e2a, 0xff3333, 0xbae67e, 0xffa759, 0x73d0ff, 0xd4bfff, 0x95e6cb, 0xc7c7c7,
                0x686868, 0xf27983, 0xa6cc70, 0xffcc66, 0x5ccfe6, 0xd4bfff, 0x95e6cb, 0xffffff
            ]
        ),
        scheme!(
            "Material",
            0x263238,
            0xeeffff,
            0xffcc00,
            0x37474f,
            [
                0x000000, 0xf07178, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xffffff,
                0x546e7a, 0xf07178, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xffffff
            ]
        ),
        scheme!(
            "Classic Terminal",
            0x000000,
            0xc0c0c0,
            0xc0c0c0,
            0x404040,
            [
                0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa,
                0x555555, 0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff
            ]
        ),
    ]
}

pub fn scheme_by_name(name: &str) -> TermScheme {
    let all = schemes();
    all.iter()
        .find(|s| s.name == name)
        .cloned()
        .unwrap_or_else(|| all[0].clone())
}

/// Resolve the xterm 256-color palette entry.
pub fn indexed_color(scheme: &TermScheme, idx: u8) -> Color32 {
    match idx {
        0..=15 => scheme.ansi[idx as usize],
        16..=231 => {
            let i = idx - 16;
            let steps = [0u8, 95, 135, 175, 215, 255];
            Color32::from_rgb(
                steps[(i / 36) as usize],
                steps[((i / 6) % 6) as usize],
                steps[(i % 6) as usize],
            )
        }
        _ => {
            let v = 8 + (idx - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

/// Palette used by the custom chrome (sidebar, cards, panels).
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    /// Text colour that stays readable on top of `accent` fills.
    pub on_accent: Color32,
    /// Border for inputs and buttons; at least 3:1 against the surface.
    pub control_border: Color32,
    /// Third text level: placeholders, separators, group counts. Still 4.5:1 on the page.
    pub faint: Color32,
    /// Data color for measured values, so meters never compete with the accent.
    pub steel: Color32,
    /// Well behind code-like content such as the equivalent command line.
    pub well: Color32,
    /// Translucent glow around a focused control.
    pub ring: Color32,
    /// Dim layer behind overlays.
    pub scrim: Color32,
}

impl Palette {
    /// Linear blend between two palettes, used for the theme crossfade.
    pub fn lerp(self, to: Palette, t: f32) -> Palette {
        let mix = |a: Color32, b: Color32| a.lerp_to_gamma(b, t);
        Palette {
            bg: mix(self.bg, to.bg),
            sidebar: mix(self.sidebar, to.sidebar),
            panel: mix(self.panel, to.panel),
            card: mix(self.card, to.card),
            card_hover: mix(self.card_hover, to.card_hover),
            border: mix(self.border, to.border),
            text: mix(self.text, to.text),
            text_dim: mix(self.text_dim, to.text_dim),
            accent: mix(self.accent, to.accent),
            accent_soft: mix(self.accent_soft, to.accent_soft),
            danger: mix(self.danger, to.danger),
            success: mix(self.success, to.success),
            warning: mix(self.warning, to.warning),
            on_accent: mix(self.on_accent, to.on_accent),
            control_border: mix(self.control_border, to.control_border),
            faint: mix(self.faint, to.faint),
            steel: mix(self.steel, to.steel),
            well: mix(self.well, to.well),
            ring: mix(self.ring, to.ring),
            scrim: mix(self.scrim, to.scrim),
        }
    }
}

/// Near-black or white, whichever has more contrast against `bg`.
pub fn readable_on(bg: Color32) -> Color32 {
    fn lum(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    let (dark, light) = (h(0x0b0e18), Color32::WHITE);
    let ratio = |a: f32, b: f32| (a.max(b) + 0.05) / (a.min(b) + 0.05);
    let l = lum(bg);
    if ratio(l, lum(dark)) >= ratio(l, lum(light)) {
        dark
    } else {
        light
    }
}

/// Accent text and focus must remain readable in the current interface theme.
/// Stored RGB preferences are unchanged; the rendered tone adapts to its surface.
fn accent_on(color: Color32, surfaces: &[Color32], ink: Color32) -> Color32 {
    let mut result = color;
    for _ in 0..24 {
        if surfaces.iter().all(|s| contrast_ratio(result, *s) >= 4.5) {
            break;
        }
        result = result.lerp_to_gamma(ink, 0.1);
    }
    result
}

fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    fn lum(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    let (a, b) = (lum(a), lum(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

pub const DEFAULT_ACCENT: [u8; 3] = [255, 138, 76];
/// Ferrum's previous default. Vaults saved with it move to ember once; see `Settings::migrate_accent`.
pub const COBALT_ACCENT: [u8; 3] = [120, 154, 255];
pub const PATINA_ACCENT: [u8; 3] = [88, 205, 180];

/// Reusable Forge geometry, in egui points.
pub mod tokens {
    pub const CONTROL: f32 = 36.0;
    pub const PAGE: i8 = 24;
    pub const RADIUS: u8 = 8;
    pub const RADIUS_LARGE: u8 = 12;
    pub const TITLE: f32 = 22.0;
    pub const CAPTION: f32 = 12.0;
    pub const SIDEBAR: f32 = 216.0;
    pub const RAIL: f32 = 56.0;
    pub const INSPECTOR: f32 = 320.0;
    pub const ROW: f32 = 56.0;
    pub const COMPACT_ROW: f32 = 42.0;
    pub const BAR: f32 = 44.0;
    pub const STATUS: f32 = 28.0;
}

pub fn palette(theme: UiTheme, rgb: [u8; 3]) -> Palette {
    let (
        bg,
        sidebar,
        panel,
        hover,
        border,
        text,
        dim,
        faint,
        control,
        success,
        warning,
        danger,
        steel,
        well,
    ) = match theme {
        UiTheme::Dark => (
            0x141416, 0x101012, 0x1c1c1f, 0x26262a, 0x2e2e33, 0xf2f0eb, 0xadaba5, 0x918f89,
            0x6e6e76, 0x6fd6a0, 0xebc85a, 0xf7808c, 0x8fb6dc, 0x0e0e10,
        ),
        UiTheme::Midnight => (
            0x0b0d12, 0x07090d, 0x12151c, 0x1b1f28, 0x242934, 0xeef1f6, 0xa6aebd, 0x8790a2,
            0x66708a, 0x6fd6a0, 0xebc85a, 0xf7808c, 0x8fb6dc, 0x07090d,
        ),
        UiTheme::Light => (
            0xf6f4ef, 0xedeae3, 0xffffff, 0xf1eee8, 0xdfdbd2, 0x1c1b19, 0x5c5a54, 0x66635c,
            0x8a867d, 0x14774a, 0x7f6000, 0xbe1e3a, 0x28608f, 0xfbfaf7,
        ),
    };
    // The three named accents have tuned tones per theme; anything else adapts below.
    let (tuned, tuned_soft) = match (theme, rgb) {
        (UiTheme::Dark, DEFAULT_ACCENT) => (0xff8a4c, Some(0x2e2019)),
        (UiTheme::Midnight, DEFAULT_ACCENT) => (0xff9259, Some(0x2a1c16)),
        (UiTheme::Light, DEFAULT_ACCENT) => (0xb5430f, Some(0xf7e4d8)),
        (UiTheme::Light, COBALT_ACCENT) => (0x2f52c9, Some(0xdee5fb)),
        (UiTheme::Light, PATINA_ACCENT) => (0x0f6f5c, Some(0xd5eee8)),
        (_, COBALT_ACCENT) => (0x789aff, Some(0x222a42)),
        (_, PATINA_ACCENT) => (0x58cdb4, Some(0x16302b)),
        _ => (
            ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | rgb[2] as u32,
            None,
        ),
    };
    let accent = accent_on(h(tuned), &[h(bg), h(sidebar), h(panel), h(hover)], h(text));
    let mut soft = match tuned_soft {
        Some(soft) => h(soft),
        None => h(bg).lerp_to_gamma(accent, 0.12),
    };
    // Dark custom accents can make a light selection too dark for secondary text.
    for _ in 0..24 {
        if contrast_ratio(h(dim), soft) >= 4.5 && contrast_ratio(accent, soft) >= 4.5 {
            break;
        }
        soft = soft.lerp_to_gamma(h(bg), 0.2);
    }
    let ring = Color32::from_rgba_unmultiplied(
        accent.r(),
        accent.g(),
        accent.b(),
        if theme == UiTheme::Light { 56 } else { 72 },
    );
    Palette {
        bg: h(bg),
        sidebar: h(sidebar),
        panel: h(panel),
        card: h(panel),
        card_hover: h(hover),
        border: h(border),
        text: h(text),
        text_dim: h(dim),
        accent,
        accent_soft: soft,
        danger: h(danger),
        success: h(success),
        warning: h(warning),
        on_accent: if rgb == DEFAULT_ACCENT && theme != UiTheme::Light {
            h(0x1c0d05)
        } else {
            readable_on(accent)
        },
        control_border: h(control),
        faint: h(faint),
        steel: h(steel),
        well: h(well),
        ring,
        scrim: if theme == UiTheme::Light {
            Color32::from_rgba_unmultiplied(40, 36, 30, 82)
        } else {
            Color32::from_rgba_unmultiplied(6, 6, 8, 153)
        },
    }
}

/// The palette currently applied to `ctx`, so widgets do not need it threaded through.
pub fn current(ctx: &egui::Context) -> Palette {
    ctx.data(|d| d.get_temp::<Palette>(egui::Id::unique("forge-palette")))
        .unwrap_or_else(|| palette(UiTheme::Dark, DEFAULT_ACCENT))
}

pub fn apply_style(ctx: &egui::Context, theme: UiTheme, accent: [u8; 3]) {
    apply_palette(ctx, theme, palette(theme, accent));
}

pub fn apply_palette(ctx: &egui::Context, theme: UiTheme, p: Palette) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("forge-palette"), p));
    let mut v = if theme == UiTheme::Light {
        Visuals::light()
    } else {
        Visuals::dark()
    };
    let radius = CornerRadius::same(tokens::RADIUS);
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.well;
    v.text_edit_bg_color = Some(p.card);
    v.faint_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(2.0, p.accent);
    v.window_corner_radius = CornerRadius::same(tokens::RADIUS_LARGE);
    v.menu_corner_radius = radius;
    v.window_stroke = Stroke::new(1.0, p.control_border);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(if theme == UiTheme::Light { 46 } else { 128 }),
    };
    v.window_shadow = v.popup_shadow;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.bg_fill = p.card;
    v.widgets.noninteractive.corner_radius = radius;
    v.widgets.inactive.bg_fill = p.card_hover;
    v.widgets.inactive.weak_bg_fill = p.card;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    v.widgets.inactive.corner_radius = radius;
    v.widgets.hovered.bg_fill = p.card_hover;
    v.widgets.hovered.weak_bg_fill = p.card_hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.control_border);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.hovered.corner_radius = radius;
    v.widgets.active.bg_fill = p.accent_soft;
    v.widgets.active.weak_bg_fill = p.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.expansion = 0.0;
    v.widgets.active.corner_radius = radius;
    v.widgets.open.weak_bg_fill = p.card_hover;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.open.corner_radius = radius;
    ctx.set_visuals(v);
    let animation_time = if crate::ui::motion::reduced(ctx) {
        0.0
    } else {
        crate::ui::motion::CONTROL
    };
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 8.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = tokens::CONTROL;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.animation_time = animation_time;
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        s.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(tokens::TITLE, crate::fonts::semibold()),
        );
        s.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(tokens::CAPTION),
        );
        s.text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
    });
}

/// Brand-ish badge color for detected operating systems.
pub fn os_color(os: Option<&str>) -> Color32 {
    match os.unwrap_or("").to_lowercase().as_str() {
        s if s.contains("ubuntu") => h(0xe95420),
        s if s.contains("debian") => h(0xd70a53),
        s if s.contains("centos") => h(0x932279),
        s if s.contains("fedora") => h(0x3c6eb4),
        s if s.contains("red hat") || s.contains("rhel") => h(0xee0000),
        s if s.contains("arch") => h(0x1793d1),
        s if s.contains("alpine") => h(0x0d597f),
        s if s.contains("amazon") => h(0xff9900),
        s if s.contains("suse") => h(0x73ba25),
        s if s.contains("freebsd") => h(0xab2b28),
        s if s.contains("darwin") || s.contains("mac") => h(0x8e8e93),
        s if s.contains("raspbian") => h(0xc51a4a),
        s if s.contains("windows") => h(0x0078d4),
        _ => h(0x5865f2),
    }
}

#[cfg(test)]
mod contrast_tests {
    use super::*;

    fn ratio(a: Color32, b: Color32) -> f32 {
        fn lum(c: Color32) -> f32 {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        }
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn custom_accent_text_and_focus_adapt_without_changing_saved_rgb() {
        for theme in [UiTheme::Dark, UiTheme::Light, UiTheme::Midnight] {
            for rgb in [
                DEFAULT_ACCENT,
                COBALT_ACCENT,
                PATINA_ACCENT,
                [99, 132, 255],
                [255, 255, 255],
                [0, 0, 0],
                [245, 166, 35],
                [60, 60, 200],
            ] {
                let palette = palette(theme, rgb);
                assert!(
                    ratio(palette.accent, palette.card) >= 4.5,
                    "{theme:?} {rgb:?}"
                );
                for surface in [
                    palette.bg,
                    palette.sidebar,
                    palette.card_hover,
                    palette.accent_soft,
                ] {
                    assert!(
                        ratio(palette.accent, surface) >= 4.5,
                        "accent across surfaces: {theme:?} {rgb:?}"
                    );
                }
                assert!(
                    ratio(palette.on_accent, palette.accent) >= 4.5,
                    "button: {theme:?} {rgb:?}"
                );
                assert!(
                    ratio(palette.text_dim, palette.accent_soft) >= 4.5,
                    "selection: {theme:?} {rgb:?}"
                );
            }
        }
    }

    #[test]
    fn text_meets_aa_on_every_surface_in_every_theme() {
        for theme in [UiTheme::Dark, UiTheme::Midnight, UiTheme::Light] {
            let p = palette(theme, DEFAULT_ACCENT);
            for (name, surface) in [
                ("bg", p.bg),
                ("sidebar", p.sidebar),
                ("panel", p.panel),
                ("card", p.card),
                ("hover", p.card_hover),
                ("selected", p.accent_soft),
            ] {
                for (tn, text) in [("text", p.text), ("text_dim", p.text_dim)] {
                    let r = ratio(text, surface);
                    assert!(r >= 4.5, "{theme:?}: {tn} on {name} is {r:.2}:1");
                }
            }
            for (name, fg) in [
                ("danger", p.danger),
                ("success", p.success),
                ("warning", p.warning),
            ] {
                let r = ratio(fg, p.card);
                assert!(r >= 4.5, "{theme:?}: {name} on card is {r:.2}:1");
            }
            let r = ratio(p.control_border, p.card);
            assert!(r >= 3.0, "{theme:?}: control border on card is {r:.2}:1");
        }
    }

    #[test]
    fn button_text_is_readable_on_any_accent() {
        for rgb in [
            [99, 132, 255],
            [76, 211, 138],
            [245, 166, 35],
            [232, 93, 117],
            [160, 120, 255],
            [60, 60, 200],
        ] {
            let accent = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            let r = ratio(readable_on(accent), accent);
            assert!(r >= 4.5, "{rgb:?}: {r:.2}:1");
        }
    }
}
