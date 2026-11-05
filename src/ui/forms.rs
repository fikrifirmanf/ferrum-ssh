//! Forge form kit: one input look, one focus treatment, shared by every screen.
//!
//! Inputs sit on the card surface with a control border. Focus warms the border, adds a soft
//! ring and grows a 2 pt "heat line" along the bottom edge from its center.

use std::str::FromStr;

use egui::{
    Align, Align2, Color32, Direction, FontId, Id, Layout, Margin, Rect, Response, RichText, Sense,
    Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, Widget, WidgetInfo, pos2, vec2,
};

use super::motion;
use crate::themes::{Palette, current, tokens};

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t)
}

/// Icon glyphs live in the private-use area and mean nothing to a screen reader.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|c| !('\u{E000}'..='\u{F8FF}').contains(c))
        .collect::<String>()
        .trim()
        .to_string()
}

fn focus_ring(ui: &Ui, rect: Rect, radius: u8, pal: Palette) {
    ui.painter().rect_stroke(
        rect.expand(2.0),
        radius.saturating_add(2),
        Stroke::new(2.0, pal.accent),
        StrokeKind::Outside,
    );
}

// ---------- Buttons ----------

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// Every button in the app: colors ease on hover and the face sinks 1 pt while pressed.
pub struct Btn {
    text: String,
    kind: Kind,
    height: f32,
    min_width: f32,
    full: bool,
    color: Option<Color32>,
    a11y: Option<String>,
}

impl Btn {
    pub fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind,
            height: tokens::CONTROL,
            min_width: 0.0,
            full: false,
            color: None,
            a11y: None,
        }
    }
    pub fn primary(text: impl Into<String>) -> Self {
        Self::new(Kind::Primary, text)
    }
    pub fn secondary(text: impl Into<String>) -> Self {
        Self::new(Kind::Secondary, text)
    }
    pub fn ghost(text: impl Into<String>) -> Self {
        Self::new(Kind::Ghost, text)
    }
    pub fn danger(text: impl Into<String>) -> Self {
        Self::new(Kind::Danger, text)
    }
    /// A square, icon-only ghost button. It needs the name a screen reader will announce.
    pub fn icon(icon: &str, name: &str) -> Self {
        let mut button = Self::new(Kind::Ghost, icon);
        button.height = 32.0;
        button.a11y = Some(name.to_string());
        button
    }
    pub fn small(mut self) -> Self {
        self.height = 28.0;
        self
    }
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
    pub fn full(mut self) -> Self {
        self.full = true;
        self
    }
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }
}

impl Widget for Btn {
    fn ui(self, ui: &mut Ui) -> Response {
        let pal = current(ui.ctx());
        let size = if self.height < 32.0 { 13.0 } else { 14.0 };
        let font = FontId::new(size, crate::fonts::medium());
        let galley = ui
            .painter()
            .layout_no_wrap(self.text.clone(), font, Color32::PLACEHOLDER);
        let chars = self.text.chars().count();
        let square = chars <= 2 && self.kind == Kind::Ghost;
        let pad = if self.height < 32.0 { 10.0 } else { 14.0 };
        let width = if self.full {
            ui.available_width()
        } else if square {
            self.height
        } else {
            (galley.size().x + pad * 2.0).max(self.min_width)
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(width, self.height), Sense::click());
        let name = self.a11y.clone().unwrap_or_else(|| plain(&self.text));
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &name));
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let enabled = ui.is_enabled();
        let ctx = ui.ctx();
        let hover = motion::ease(
            ctx,
            resp.id.with("h"),
            resp.hovered() && enabled,
            motion::FAST,
        );
        let press = motion::ease(
            ctx,
            resp.id.with("p"),
            resp.is_pointer_button_down_on() && enabled,
            motion::FAST,
        );
        let face = rect.translate(vec2(0.0, press));
        let (fill, stroke, ink) = match (self.kind, enabled) {
            (Kind::Primary, true) => {
                let lift = if ui.visuals().dark_mode {
                    Color32::WHITE
                } else {
                    Color32::BLACK
                };
                (
                    lerp(pal.accent, lift, 0.10 * hover),
                    Color32::TRANSPARENT,
                    pal.on_accent,
                )
            }
            (Kind::Secondary, true) => (
                lerp(pal.card, pal.card_hover, hover),
                pal.control_border,
                pal.text,
            ),
            (Kind::Ghost, true) => (
                pal.card_hover.gamma_multiply(hover),
                Color32::TRANSPARENT,
                lerp(pal.text_dim, pal.text, hover),
            ),
            (Kind::Danger, true) => (
                pal.card_hover.gamma_multiply(hover),
                Color32::TRANSPARENT,
                pal.danger,
            ),
            (_, false) => (pal.card_hover, pal.border, pal.text_dim),
        };
        let ink = self.color.filter(|_| enabled).unwrap_or(ink);
        let radius = if square { 6 } else { tokens::RADIUS };
        let p = ui.painter();
        p.rect(
            face,
            radius,
            fill,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        );
        p.galley(face.center() - galley.size() / 2.0, galley, ink);
        if resp.has_focus() {
            focus_ring(ui, face, radius, pal);
        }
        resp
    }
}

// ---------- Text ----------

pub fn label(ui: &mut Ui, text: &str, pal: Palette) -> Response {
    ui.label(
        RichText::new(text)
            .size(tokens::CAPTION)
            .family(crate::fonts::medium())
            .color(pal.text_dim),
    )
}

/// A field label with an optional note at the far edge, 16 pt tall so fields stay tight.
pub fn label_row(ui: &mut Ui, title: &str, note: &str, pal: Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let p = ui.painter();
    p.text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        title,
        FontId::new(tokens::CAPTION, crate::fonts::medium()),
        pal.text_dim,
    );
    p.text(
        rect.right_center(),
        Align2::RIGHT_CENTER,
        note,
        FontId::proportional(tokens::CAPTION),
        pal.text_dim,
    );
}

pub fn help(ui: &mut Ui, text: &str, pal: Palette) -> Response {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(tokens::CAPTION)
                .color(pal.text_dim),
        )
        .wrap(),
    )
}

pub fn error_line(ui: &mut Ui, text: &str, pal: Palette) -> Response {
    ui.add(
        egui::Label::new(
            RichText::new(format!("{}  {text}", crate::icons::WARNING_CIRCLE))
                .size(tokens::CAPTION)
                .color(pal.danger),
        )
        .wrap(),
    )
}

/// A page or card heading.
pub fn heading(ui: &mut Ui, text: &str, size: f32) -> Response {
    ui.label(
        RichText::new(text)
            .size(size)
            .family(crate::fonts::semibold()),
    )
}

pub fn chip(ui: &mut Ui, text: &str, mono: bool, pal: Palette) -> Response {
    let font = if mono {
        FontId::monospace(tokens::CAPTION)
    } else {
        FontId::proportional(tokens::CAPTION)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, pal.text_dim);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 4, pal.card_hover);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, pal.text_dim);
    resp
}

/// A keyboard hint, such as the shortcut beside Find a command.
pub fn kbd(ui: &mut Ui, text: &str, pal: Palette) -> Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        FontId::monospace(tokens::CAPTION),
        pal.text_dim,
    );
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 10.0, 20.0), Sense::hover());
    ui.painter().rect(
        rect,
        4,
        pal.card_hover,
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, pal.text_dim);
    resp
}

/// A status dot with its word, so color is never the only signal.
pub fn dot(ui: &mut Ui, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    resp
}

// ---------- Input chrome ----------

struct Chrome {
    focused: bool,
    hovered: bool,
    error: bool,
    inherited: bool,
    enabled: bool,
}

fn rounded_outline(rect: Rect, radius: f32) -> Vec<egui::Pos2> {
    let mut points = Vec::with_capacity(44);
    let corners = [
        (rect.right_top() + vec2(-radius, radius), -90.0_f32),
        (rect.right_bottom() + vec2(-radius, -radius), 0.0),
        (rect.left_bottom() + vec2(radius, -radius), 90.0),
        (rect.left_top() + vec2(radius, radius), 180.0),
    ];
    points.push(rect.left_top() + vec2(radius, 0.0));
    for (center, start) in corners {
        for step in 0..=8 {
            let angle = (start + step as f32 * 11.25).to_radians();
            points.push(center + radius * vec2(angle.cos(), angle.sin()));
        }
    }
    points.push(rect.left_top() + vec2(radius, 0.0));
    points
}

fn paint_chrome(ui: &Ui, id: Id, rect: Rect, c: Chrome, pal: Palette) {
    let ctx = ui.ctx();
    let heat = motion::ease(ctx, id.with("heat"), c.focused, motion::HEAT);
    let hover = motion::ease(ctx, id.with("hover"), c.hovered && !c.focused, motion::FAST);
    let hot = if c.error { pal.danger } else { pal.accent };
    let fill = if c.enabled {
        lerp(pal.card, pal.card_hover, hover)
    } else {
        pal.card_hover
    };
    let rest = if c.error {
        pal.danger
    } else if c.enabled {
        pal.control_border
    } else {
        pal.border
    };
    let border = lerp(rest, hot, heat);
    let radius = tokens::RADIUS;
    let p = ui.painter();
    if heat > 0.0 {
        let ring = if c.error {
            pal.danger.gamma_multiply(0.28)
        } else {
            pal.ring
        };
        p.rect_stroke(
            rect,
            radius,
            Stroke::new(3.0, ring.gamma_multiply(heat)),
            StrokeKind::Outside,
        );
    }
    p.rect_filled(rect, radius, fill);
    if c.inherited && heat <= 0.0 && !c.error {
        p.add(Shape::dashed_line(
            &rounded_outline(rect.shrink(0.5), radius as f32),
            Stroke::new(1.0, border),
            4.0,
            3.0,
        ));
    } else {
        p.rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    }
    if heat > 0.0 {
        let half = (rect.width() / 2.0 - 8.0) * heat;
        p.hline(
            (rect.center().x - half)..=(rect.center().x + half),
            rect.bottom() - 0.5,
            Stroke::new(2.0, hot),
        );
    }
}

#[derive(Clone, Copy, Default)]
pub struct Input<'a> {
    label: Option<&'a str>,
    hint: &'a str,
    help: Option<&'a str>,
    error: Option<&'a str>,
    icon: Option<&'a str>,
    suffix: Option<&'a str>,
    width: Option<f32>,
    salt: Option<&'a str>,
    mono: bool,
    password: bool,
    inherited: bool,
    invalid: bool,
}

impl<'a> Input<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label: Some(label),
            ..Default::default()
        }
    }
    /// An input with no visible label. `name` is still announced to screen readers.
    pub fn bare(name: &'a str) -> Self {
        Self {
            salt: Some(name),
            ..Default::default()
        }
    }
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }
    pub fn help(mut self, help: &'a str) -> Self {
        self.help = Some(help);
        self
    }
    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }
    /// Marks the text as wrong without adding a message line.
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = Some(suffix);
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    pub fn password(mut self) -> Self {
        self.password = true;
        self
    }
    /// Draws the dashed edge that says this value comes from a group or default.
    pub fn inherited(mut self, inherited: bool) -> Self {
        self.inherited = inherited;
        self
    }
    /// Keeps widget ids stable when several unlabeled inputs repeat in a list.
    pub fn salt(mut self, salt: &'a str) -> Self {
        self.salt = Some(salt);
        self
    }

    pub fn show(self, ui: &mut Ui, value: &mut String) -> Response {
        let pal = current(ui.ctx());
        let width = self
            .width
            .unwrap_or_else(|| ui.available_width())
            .min(ui.available_width());
        let id = ui.scope_id().with((
            "forge-input",
            self.salt.unwrap_or(""),
            self.label.unwrap_or(""),
        ));
        let name = self.label.or(self.salt).unwrap_or(self.hint).to_string();
        let bare = self.label.is_none() && self.help.is_none() && self.error.is_none();
        let height = if bare { tokens::CONTROL } else { 0.0 };
        ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 6.0;
            if let Some(text) = self.label {
                label_row(ui, text, "", pal);
            }
            let resp = self.field(ui, id, width, value, &name, pal);
            if let Some(error) = self.error {
                error_line(ui, error, pal);
            } else if let Some(text) = self.help {
                help(ui, text, pal);
            }
            resp
        })
        .inner
    }

    fn field(
        self,
        ui: &mut Ui,
        id: Id,
        width: f32,
        value: &mut String,
        name: &str,
        pal: Palette,
    ) -> Response {
        let (rect, _) = ui.allocate_exact_size(vec2(width, tokens::CONTROL), Sense::hover());
        let edit_id = id.with("edit");
        let enabled = ui.is_enabled();
        let focused = ui.memory(|m| m.has_focus(edit_id));
        paint_chrome(
            ui,
            id,
            rect,
            Chrome {
                focused,
                hovered: enabled && ui.rect_contains_pointer(rect),
                error: self.error.is_some() || self.invalid,
                inherited: self.inherited,
                enabled,
            },
            pal,
        );
        let body = ui.interact(rect, id.with("body"), Sense::click());
        if body.clicked() && enabled {
            ui.memory_mut(|m| m.request_focus(edit_id));
        }
        body.on_hover_cursor(egui::CursorIcon::Text);
        let inner = rect.shrink2(vec2(12.0, 0.0));
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(id.with("row"))
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 8.0;
        if let Some(icon) = self.icon {
            child.label(RichText::new(icon).size(16.0).color(pal.text_dim));
        }
        let visible_key = id.with("visible");
        let mut visible = ui.data(|d| d.get_temp::<bool>(visible_key).unwrap_or(false));
        let reserve = self
            .suffix
            .map(|s| {
                ui.painter()
                    .layout_no_wrap(
                        s.to_string(),
                        FontId::proportional(tokens::CAPTION),
                        pal.text_dim,
                    )
                    .size()
                    .x
                    + 8.0
            })
            .unwrap_or(0.0)
            + if self.password { 28.0 } else { 0.0 };
        let mut edit = TextEdit::singleline(value)
            .id(edit_id)
            .frame(egui::Frame::NONE)
            .desired_width((child.available_width() - reserve).max(16.0))
            .align(Align2::LEFT_CENTER)
            .password(self.password && !visible)
            .hint_text(RichText::new(self.hint).color(pal.faint));
        if self.mono {
            edit = edit.font(FontId::monospace(13.0));
        }
        if self.inherited {
            edit = edit.text_color(pal.text_dim);
        }
        let resp = child.add(edit);
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, enabled, name));
        if let Some(suffix) = self.suffix {
            child.label(
                RichText::new(suffix)
                    .size(tokens::CAPTION)
                    .color(pal.text_dim),
            );
        }
        if self.password {
            let tip = if visible {
                "Hide password"
            } else {
                "Show password"
            };
            let icon = if visible {
                crate::icons::EYE_SLASH
            } else {
                crate::icons::EYE
            };
            child.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(Btn::icon(icon, tip).height(28.0))
                    .on_hover_text(tip)
                    .clicked()
                {
                    visible = !visible;
                    ui.data_mut(|d| d.insert_temp(visible_key, visible));
                    resp.request_focus();
                }
            });
        }
        resp
    }
}

/// A labeled box whose empty value means "use the inherited one".
pub fn opt_text(ui: &mut Ui, input: Input<'_>, value: &mut Option<String>) -> Response {
    let mut text = value.clone().unwrap_or_default();
    let resp = input.inherited(value.is_none()).show(ui, &mut text);
    if resp.changed() {
        *value = (!text.is_empty()).then_some(text);
    }
    resp
}

/// A numeric box whose empty value means "use the inherited one".
pub fn opt_number<T>(ui: &mut Ui, input: Input<'_>, value: &mut Option<T>) -> Response
where
    T: FromStr + ToString,
{
    let mut text = value.as_ref().map(ToString::to_string).unwrap_or_default();
    let resp = input.inherited(value.is_none()).mono().show(ui, &mut text);
    if resp.changed() {
        *value = text.trim().parse().ok();
    }
    resp
}

/// A numeric box that always holds a value; text that does not parse is ignored.
pub fn number<T>(ui: &mut Ui, input: Input<'_>, value: &mut T) -> Response
where
    T: FromStr + ToString,
{
    let mut text = value.to_string();
    let resp = input.mono().show(ui, &mut text);
    if resp.changed() {
        if let Ok(parsed) = text.trim().parse() {
            *value = parsed;
        }
    }
    resp
}

/// A multi-line box with the same chrome as [`Input`].
pub fn text_area(
    ui: &mut Ui,
    title: &str,
    value: &mut String,
    rows: usize,
    mono: bool,
    hint: &str,
) -> Response {
    let pal = current(ui.ctx());
    let id = ui.scope_id().with(("forge-area", title));
    let edit_id = id.with("edit");
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        if !title.is_empty() {
            label(ui, title, pal);
        }
        let width = ui.available_width();
        let layer = ui.painter().add(Shape::Noop);
        let mut edit = TextEdit::multiline(value)
            .id(edit_id)
            .frame(egui::Frame::NONE)
            .desired_rows(rows)
            .desired_width(width)
            .hint_text(RichText::new(hint).color(pal.faint));
        if mono {
            edit = edit.font(FontId::monospace(13.0));
        }
        let resp = egui::Frame::NONE
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width((width - 24.0).max(40.0));
                ui.add(edit)
            });
        let rect = resp.response.rect;
        let inner = resp.inner;
        let enabled = ui.is_enabled();
        // The box is known only after its text is laid out, so paint behind it afterwards.
        let heat = motion::ease(ui.ctx(), id.with("heat"), inner.has_focus(), motion::HEAT);
        let hot = lerp(
            if enabled {
                pal.control_border
            } else {
                pal.border
            },
            pal.accent,
            heat,
        );
        ui.painter().set(
            layer,
            Shape::Rect(egui::epaint::RectShape::new(
                rect,
                tokens::RADIUS,
                pal.card,
                Stroke::new(1.0, hot),
                StrokeKind::Inside,
            )),
        );
        if heat > 0.0 {
            let half = (rect.width() / 2.0 - 8.0) * heat;
            ui.painter().rect_stroke(
                rect,
                tokens::RADIUS,
                Stroke::new(3.0, pal.ring.gamma_multiply(heat)),
                StrokeKind::Outside,
            );
            ui.painter().hline(
                (rect.center().x - half)..=(rect.center().x + half),
                rect.bottom() - 0.5,
                Stroke::new(2.0, pal.accent),
            );
        }
        inner
    })
    .inner
}

/// `user @ host : port` in one control, so a host reads the way people type it.
#[derive(Clone, Copy)]
pub struct AddressHints<'a> {
    pub user: &'a str,
    pub host: &'a str,
    pub port: &'a str,
}

fn address_id() -> Id {
    Id::unique("forge-address")
}

/// The host part of the address line. A screen shows one address line at a time.
pub fn address_host_id() -> Id {
    address_id().with("host")
}

pub fn address_line(
    ui: &mut Ui,
    user: Option<&mut Option<String>>,
    host: &mut String,
    port: &mut Option<u16>,
    hints: AddressHints<'_>,
    error: bool,
) {
    const HEIGHT: f32 = 44.0;
    let pal = current(ui.ctx());
    let id = address_id();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let ids = [id.with("user"), id.with("host"), id.with("port")];
    let focused = ui.memory(|m| ids.iter().any(|edit| m.has_focus(*edit)));
    paint_chrome(
        ui,
        id,
        rect,
        Chrome {
            focused,
            hovered: false,
            error,
            inherited: false,
            enabled: ui.is_enabled(),
        },
        pal,
    );
    let font = FontId::monospace(14.0);
    let glyph = 12.0;
    let total = rect.width() - if user.is_some() { glyph * 2.0 } else { glyph };
    let (user_w, port_w) = if user.is_some() {
        ((total * 0.29).max(72.0), (total * 0.14).max(56.0))
    } else {
        (0.0, (total * 0.2).max(56.0))
    };
    let host_w = total - user_w - port_w;
    let mut x = rect.left();
    let cell = |x: f32, w: f32| Rect::from_min_size(pos2(x, rect.top()), vec2(w, HEIGHT));
    let box_edit =
        |ui: &mut Ui, area: Rect, edit_id: Id, text: &mut String, hint: &str, name: &str| {
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(edit_id.with("cell"))
                    .max_rect(area.shrink2(vec2(12.0, 0.0)))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            let resp = child.add(
                TextEdit::singleline(text)
                    .id(edit_id)
                    .font(font.clone())
                    .frame(egui::Frame::NONE)
                    .align(Align2::LEFT_CENTER)
                    .desired_width(area.width() - 24.0)
                    .hint_text(RichText::new(hint).color(pal.faint)),
            );
            resp.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, true, name));
            resp
        };
    let sep = |ui: &Ui, x: f32, text: &str| {
        ui.painter().text(
            pos2(x + glyph / 2.0, rect.center().y),
            Align2::CENTER_CENTER,
            text,
            font.clone(),
            pal.faint,
        );
    };
    if let Some(user) = user {
        let mut text = user.clone().unwrap_or_default();
        if box_edit(
            ui,
            cell(x, user_w),
            ids[0],
            &mut text,
            hints.user,
            "Username",
        )
        .changed()
        {
            *user = (!text.is_empty()).then_some(text);
        }
        x += user_w;
        sep(ui, x, "@");
        x += glyph;
    }
    box_edit(
        ui,
        cell(x, host_w),
        ids[1],
        host,
        hints.host,
        "Hostname or IP",
    );
    x += host_w;
    sep(ui, x, ":");
    x += glyph;
    let mut text = port.map(|p| p.to_string()).unwrap_or_default();
    if box_edit(ui, cell(x, port_w), ids[2], &mut text, hints.port, "Port").changed() {
        *port = text.trim().parse().ok();
    }
}

// ---------- Choice controls ----------

/// Equal-width options with a thumb that slides to the choice.
pub fn segmented<T: PartialEq + Clone>(
    ui: &mut Ui,
    salt: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    segmented_sized(ui, salt, value, options, 28.0, 13.0, 14.0)
}

pub fn segmented_tiny<T: PartialEq + Clone>(
    ui: &mut Ui,
    salt: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    segmented_sized(ui, salt, value, options, 20.0, 12.0, 10.0)
}

fn segmented_sized<T: PartialEq + Clone>(
    ui: &mut Ui,
    salt: &str,
    value: &mut T,
    options: &[(T, &str)],
    height: f32,
    text_size: f32,
    pad: f32,
) -> bool {
    let pal = current(ui.ctx());
    let id = ui.scope_id().with(("forge-seg", salt));
    let font = FontId::new(text_size, crate::fonts::medium());
    let widest = options
        .iter()
        .map(|(_, text)| {
            ui.painter()
                .layout_no_wrap(text.to_string(), font.clone(), pal.text)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let inset = 3.0;
    let cell = widest + pad * 2.0;
    let wanted = cell * options.len() as f32 + inset * 2.0;
    let width = wanted.min(ui.available_width());
    let cell = (width - inset * 2.0) / options.len() as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height + inset * 2.0), Sense::hover());
    let track = rect.shrink(0.5);
    ui.painter().rect(
        track,
        tokens::RADIUS,
        pal.sidebar,
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
    let index = options
        .iter()
        .position(|(option, _)| option == value)
        .unwrap_or(0);
    let at = motion::value(ui.ctx(), id.with("thumb"), index as f32, motion::CONTROL);
    let thumb = Rect::from_min_size(
        rect.min + vec2(inset + at * cell, inset),
        vec2(cell, height),
    );
    ui.painter().rect(
        thumb,
        6,
        pal.card_hover,
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
    let mut changed = false;
    for (i, (option, text)) in options.iter().enumerate() {
        let cell_rect = Rect::from_min_size(
            rect.min + vec2(inset + i as f32 * cell, inset),
            vec2(cell, height),
        );
        let resp = ui.interact(cell_rect, id.with(i), Sense::click());
        let selected = option == value;
        resp.widget_info(|| {
            WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, *text)
        });
        let warm = motion::ease(
            ui.ctx(),
            id.with((i, "ink")),
            selected || resp.hovered(),
            motion::FAST,
        );
        ui.painter().text(
            cell_rect.center(),
            Align2::CENTER_CENTER,
            *text,
            font.clone(),
            lerp(pal.text_dim, pal.text, warm),
        );
        if resp.has_focus() {
            focus_ring(ui, cell_rect, 6, pal);
        }
        if resp.clicked() && !selected {
            *value = option.clone();
            changed = true;
        }
    }
    changed
}

/// 34 by 20 pt: the knob and fill move on one clock.
pub fn switch(ui: &mut Ui, on: &mut bool, name: &str) -> Response {
    let pal = current(ui.ctx());
    let (rect, mut resp) = ui.allocate_exact_size(vec2(34.0, 20.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp.widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, name));
    let t = motion::ease(ui.ctx(), resp.id.with("t"), *on, motion::CONTROL);
    let p = ui.painter();
    p.rect(
        rect,
        10,
        lerp(pal.card_hover, pal.accent, t),
        Stroke::new(1.0, lerp(pal.control_border, pal.accent, t)),
        StrokeKind::Inside,
    );
    p.circle_filled(
        pos2(rect.left() + 10.0 + 14.0 * t, rect.center().y),
        6.0,
        lerp(pal.text_dim, pal.on_accent, t),
    );
    if resp.has_focus() {
        focus_ring(ui, rect, 10, pal);
    }
    resp
}

/// A switch with its text beside it, for places that used a plain checkbox.
pub fn check(ui: &mut Ui, on: &mut bool, text: impl ToString) -> Response {
    let text = text.to_string();
    ui.horizontal(|ui| {
        let mut resp = switch(ui, on, &plain(&text));
        ui.spacing_mut().item_spacing.x = 8.0;
        let label = ui.add(
            egui::Label::new(text.as_str())
                .wrap_mode(egui::TextWrapMode::Wrap)
                .sense(Sense::click()),
        );
        if label.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        resp
    })
    .inner
}

/// Title, supporting line and a switch at the far edge.
pub fn switch_row(ui: &mut Ui, title: &str, note: &str, on: &mut bool) -> Response {
    let pal = current(ui.ctx());
    let width = ui.available_width();
    let text_width = (width - 50.0).max(80.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        ui.allocate_ui_with_layout(vec2(text_width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(text_width);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(6.0);
            ui.add(egui::Label::new(title).wrap());
            if !note.is_empty() {
                help(ui, note, pal);
            }
            ui.add_space(6.0);
        });
        ui.allocate_ui_with_layout(vec2(34.0, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.add_space(if note.is_empty() { 6.0 } else { 12.0 });
            switch(ui, on, title)
        })
        .inner
    })
    .inner
}

/// A [`switch_row`] over a setting that can inherit; shows where the value comes from.
pub fn inherit_row(
    ui: &mut Ui,
    title: &str,
    note: &str,
    value: &mut Option<bool>,
    inherited: bool,
    source: &str,
) {
    let mut on = value.unwrap_or(inherited);
    let detail = if value.is_none() {
        if note.is_empty() {
            format!("Inherited from {source}")
        } else {
            format!("{note} Inherited from {source}.")
        }
    } else {
        note.to_string()
    };
    if switch_row(ui, title, &detail, &mut on).changed() {
        *value = Some(on);
    }
    if value.is_some()
        && ui
            .add(Btn::ghost(format!("Use {source}")).small())
            .clicked()
    {
        *value = None;
    }
}

/// A closed select. `items` draws the popup; use [`option`] for each row.
pub fn select(
    ui: &mut Ui,
    salt: &str,
    width: f32,
    shown: &str,
    mono: bool,
    items: impl FnOnce(&mut Ui),
) -> Response {
    let pal = current(ui.ctx());
    let width = width.min(ui.available_width());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, tokens::CONTROL), Sense::click());
    let id = ui.scope_id().with(("forge-select", salt));
    let popup_id = egui::Popup::default_response_id(&resp);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    paint_chrome(
        ui,
        id,
        rect,
        Chrome {
            focused: open || resp.has_focus(),
            hovered: resp.hovered(),
            error: false,
            inherited: false,
            enabled: ui.is_enabled(),
        },
        pal,
    );
    resp.widget_info(|| WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), shown));
    let font = if mono {
        FontId::monospace(13.0)
    } else {
        FontId::proportional(14.0)
    };
    let clip = rect.shrink2(vec2(12.0, 0.0));
    let clip = Rect::from_min_max(clip.min, pos2(clip.right() - 20.0, clip.bottom()));
    ui.painter().with_clip_rect(clip).text(
        clip.left_center(),
        Align2::LEFT_CENTER,
        shown,
        font,
        pal.text,
    );
    let turn = motion::ease(ui.ctx(), id.with("turn"), open, motion::CONTROL);
    chevron(
        ui,
        pos2(rect.right() - 16.0, rect.center().y),
        std::f32::consts::FRAC_PI_2 * (1.0 + 2.0 * turn),
        pal.text_dim,
    );
    egui::Popup::menu(&resp).width(width).gap(4.0).show(|ui| {
        ui.set_min_width(width - 12.0);
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                items(ui);
            });
    });
    resp
}

/// One row in a [`select`] popup.
pub fn option(ui: &mut Ui, selected: bool, text: &str) -> Response {
    let pal = current(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    resp.widget_info(|| WidgetInfo::selected(egui::Role::ComboBox, true, selected, text));
    let hover = motion::ease(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::FAST);
    ui.painter()
        .rect_filled(rect, 6, pal.card_hover.gamma_multiply(hover));
    let clip = rect.shrink2(vec2(10.0, 0.0));
    ui.painter().with_clip_rect(clip).text(
        clip.left_center(),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        pal.text,
    );
    if selected {
        ui.painter().text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            crate::icons::CHECK,
            FontId::proportional(14.0),
            pal.accent,
        );
    }
    resp
}

pub fn select_value<T: PartialEq + Clone>(ui: &mut Ui, value: &mut T, option_value: T, text: &str) {
    if option(ui, *value == option_value, text).clicked() {
        *value = option_value;
        ui.close();
    }
}

/// A labeled [`select`] for the common "one of these values" case.
pub fn select_field<T: PartialEq + Clone>(
    ui: &mut Ui,
    title: &str,
    value: &mut T,
    options: &[(T, String)],
) {
    let pal = current(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        label(ui, title, pal);
        let shown = options
            .iter()
            .find(|(option_value, _)| option_value == value)
            .map(|(_, text)| text.clone())
            .unwrap_or_default();
        let width = ui.available_width();
        select(ui, title, width, &shown, false, |ui| {
            for (option_value, text) in options {
                select_value(ui, value, option_value.clone(), text);
            }
        });
    });
}

/// A small chevron pointing along `angle`; rotating it says open or closed.
pub fn chevron(ui: &Ui, center: egui::Pos2, angle: f32, color: Color32) {
    let (s, c) = angle.sin_cos();
    let at = |x: f32, y: f32| center + vec2(x * c - y * s, x * s + y * c);
    ui.painter().add(Shape::line(
        vec![at(-2.5, -4.5), at(2.5, 0.0), at(-2.5, 4.5)],
        Stroke::new(1.75, color),
    ));
}

// ---------- Layout ----------

/// Folds content open and closed. Height, opacity and clip share one clock, so what sits
/// below moves out of the way instead of jumping.
pub fn reveal<R>(ui: &mut Ui, id: Id, open: bool, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let t = motion::ease_panel(ui.ctx(), id.with("reveal"), open);
    if t <= 0.0 {
        return None;
    }
    let width = ui.available_width();
    let top = ui.cursor().min;
    let known = ui.data(|d| d.get_temp::<f32>(id.with("h")).unwrap_or(0.0));
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(id.with("body"))
            .max_rect(Rect::from_min_size(top, vec2(width, 100_000.0)))
            .layout(Layout::top_down(Align::Min)),
    );
    if t < 1.0 {
        let visible = Rect::from_min_size(top - vec2(4.0, 0.0), vec2(width + 8.0, known * t));
        child.set_clip_rect(child.clip_rect().intersect(visible));
        child.set_opacity(t);
    }
    let out = add(&mut child);
    let full = child.min_rect().height();
    ui.data_mut(|d| d.insert_temp(id.with("h"), full));
    ui.allocate_exact_size(vec2(width, full * t), Sense::hover());
    Some(out)
}

/// A card holding a fixed group of fields.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let pal = current(ui.ctx());
    egui::Frame::new()
        .fill(pal.card)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(tokens::RADIUS_LARGE)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 12.0;
            add(ui)
        })
        .inner
}

/// A card that folds. While closed it keeps a one-line summary in its header.
pub fn section(
    ui: &mut Ui,
    salt: &str,
    title: &str,
    summary: &str,
    default_open: bool,
    add: impl FnOnce(&mut Ui),
) {
    let pal = current(ui.ctx());
    let id = ui.scope_id().with(("forge-section", salt));
    let key = id.with("open");
    let mut open = ui.data(|d| d.get_temp::<bool>(key).unwrap_or(default_open));
    let width = ui.available_width();
    let layer = ui.painter().add(Shape::Noop);
    let top = ui.cursor().min;
    let mut body_gap = 0.0;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 48.0), Sense::click());
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, title));
        if resp.clicked() {
            open = !open;
            ui.data_mut(|d| d.insert_temp(key, open));
        }
        let t = motion::ease_panel(ui.ctx(), id.with("turn"), open);
        let hover = motion::ease(ui.ctx(), id.with("h"), resp.hovered(), motion::FAST);
        ui.painter().rect_filled(
            rect.shrink(1.0),
            tokens::RADIUS_LARGE,
            pal.card_hover.gamma_multiply(hover),
        );
        chevron(
            ui,
            pos2(rect.left() + 22.0, rect.center().y),
            std::f32::consts::FRAC_PI_2 * t,
            pal.text_dim,
        );
        let title_galley = ui.painter().layout_no_wrap(
            title.to_string(),
            FontId::new(14.0, crate::fonts::semibold()),
            pal.text,
        );
        let title_width = title_galley.size().x;
        ui.painter().galley(
            pos2(
                rect.left() + 40.0,
                rect.center().y - title_galley.size().y / 2.0,
            ),
            title_galley,
            pal.text,
        );
        if t < 1.0 && !summary.is_empty() {
            let area = Rect::from_min_max(
                pos2(rect.left() + 56.0 + title_width, rect.top()),
                pos2(rect.right() - 16.0, rect.bottom()),
            );
            ui.painter().with_clip_rect(area).text(
                area.right_center(),
                Align2::RIGHT_CENTER,
                summary,
                FontId::proportional(tokens::CAPTION),
                pal.text_dim.gamma_multiply(1.0 - t),
            );
        }
        if resp.has_focus() {
            focus_ring(ui, rect, tokens::RADIUS_LARGE, pal);
        }
        reveal(ui, id.with("body"), open, |ui| {
            egui::Frame::NONE
                .inner_margin(Margin {
                    left: 16,
                    right: 16,
                    top: 4,
                    bottom: 16,
                })
                .show(ui, |ui| {
                    ui.set_width(width - 32.0);
                    ui.spacing_mut().item_spacing.y = 12.0;
                    add(ui);
                });
        });
        body_gap = ui.cursor().min.y;
    });
    let rect = Rect::from_min_size(top, vec2(width, (body_gap - top.y).max(48.0)));
    ui.painter().set(
        layer,
        Shape::Rect(egui::epaint::RectShape::new(
            rect,
            tokens::RADIUS_LARGE,
            pal.card,
            Stroke::new(1.0, pal.border),
            StrokeKind::Inside,
        )),
    );
}

/// Lays fields out on a share of the width, wrapping to one column when narrow.
pub fn columns(ui: &mut Ui, spans: &[f32], mut add: impl FnMut(usize, &mut Ui)) {
    const GAP: f32 = 12.0;
    let width = ui.available_width();
    if width < 420.0 {
        for i in 0..spans.len() {
            ui.vertical(|ui| add(i, ui));
        }
        return;
    }
    let total: f32 = spans.iter().sum();
    let usable = width - GAP * (spans.len().saturating_sub(1)) as f32;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        for (i, span) in spans.iter().enumerate() {
            let cell = usable * span / total;
            ui.allocate_ui_with_layout(vec2(cell, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(cell);
                add(i, ui);
            });
        }
    });
}

/// Tags as chips: Enter or a comma adds one, Backspace on empty removes the last.
pub fn tag_input(ui: &mut Ui, title: &str, tags: &mut Vec<String>) -> bool {
    let pal = current(ui.ctx());
    let id = ui.scope_id().with(("forge-tags", title));
    let edit_id = id.with("edit");
    let mut changed = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        label(ui, title, pal);
        let width = ui.available_width();
        let layer = ui.painter().add(Shape::Noop);
        let frame = egui::Frame::NONE
            .inner_margin(Margin::symmetric(8, 7))
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    ui.spacing_mut().interact_size.y = 22.0;
                    let mut remove = None;
                    for (i, tag) in tags.iter().enumerate() {
                        if tag_chip(ui, tag, pal) {
                            remove = Some(i);
                        }
                    }
                    if let Some(i) = remove {
                        tags.remove(i);
                        changed = true;
                    }
                    let mut draft = ui.data(|d| d.get_temp::<String>(id).unwrap_or_default());
                    let hint = if tags.is_empty() {
                        "Type a tag and press Enter"
                    } else {
                        "Add a tag"
                    };
                    let resp = ui.add(
                        TextEdit::singleline(&mut draft)
                            .id(edit_id)
                            .frame(egui::Frame::NONE)
                            .desired_width(180.0)
                            .align(Align2::LEFT_CENTER)
                            .min_size(vec2(0.0, 22.0))
                            .hint_text(RichText::new(hint).color(pal.faint)),
                    );
                    resp.widget_info(|| WidgetInfo::labeled(egui::Role::TextInput, true, title));
                    let commit = (resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        || draft.contains(',');
                    if commit {
                        for piece in draft.split(',') {
                            let piece = piece.trim();
                            if !piece.is_empty() && !tags.iter().any(|t| t == piece) {
                                tags.push(piece.to_string());
                                changed = true;
                            }
                        }
                        draft.clear();
                        resp.request_focus();
                    } else if draft.is_empty()
                        && resp.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Backspace))
                        && tags.pop().is_some()
                    {
                        changed = true;
                    }
                    ui.data_mut(|d| d.insert_temp(id, draft));
                    resp.has_focus()
                })
                .inner
            });
        let rect = frame.response.rect;
        let heat = motion::ease(ui.ctx(), id.with("heat"), frame.inner, motion::HEAT);
        ui.painter().set(
            layer,
            Shape::Rect(egui::epaint::RectShape::new(
                rect,
                tokens::RADIUS,
                pal.card,
                Stroke::new(1.0, lerp(pal.control_border, pal.accent, heat)),
                StrokeKind::Inside,
            )),
        );
        if heat > 0.0 {
            ui.painter().rect_stroke(
                rect,
                tokens::RADIUS,
                Stroke::new(3.0, pal.ring.gamma_multiply(heat)),
                StrokeKind::Outside,
            );
            let half = (rect.width() / 2.0 - 8.0) * heat;
            ui.painter().hline(
                (rect.center().x - half)..=(rect.center().x + half),
                rect.bottom() - 0.5,
                Stroke::new(2.0, pal.accent),
            );
        }
    });
    changed
}

fn tag_chip(ui: &mut Ui, tag: &str, pal: Palette) -> bool {
    let galley = ui.painter().layout_no_wrap(
        tag.to_string(),
        FontId::proportional(tokens::CAPTION),
        pal.text_dim,
    );
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 28.0, 22.0), Sense::hover());
    ui.painter().rect_filled(rect, 4, pal.card_hover);
    ui.painter().galley(
        pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        pal.text_dim,
    );
    let x_rect = Rect::from_center_size(rect.right_center() - vec2(10.0, 0.0), Vec2::splat(16.0));
    let x = ui.interact(x_rect, ui.scope_id().with(("tag-x", tag)), Sense::click());
    x.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, format!("Remove {tag}")));
    ui.painter().text(
        x_rect.center(),
        Align2::CENTER_CENTER,
        crate::icons::X,
        FontId::proportional(11.0),
        if x.hovered() { pal.text } else { pal.text_dim },
    );
    x.clicked()
}

/// A dark well showing a command, with a copy button. Returns true on the frame it was copied.
pub fn command_line(ui: &mut Ui, command: &str) -> bool {
    let pal = current(ui.ctx());
    let id = ui.scope_id().with("forge-command");
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 44.0), Sense::hover());
    ui.painter().rect(
        rect,
        tokens::RADIUS,
        pal.well,
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.left_center() + vec2(14.0, 0.0),
        Align2::LEFT_CENTER,
        "$",
        FontId::monospace(13.0),
        pal.faint,
    );
    let area = Rect::from_min_max(rect.min + vec2(30.0, 0.0), rect.max - vec2(44.0, 0.0));
    ui.painter().with_clip_rect(area).text(
        area.left_center(),
        Align2::LEFT_CENTER,
        command,
        FontId::monospace(13.0),
        pal.text,
    );
    let info = ui.interact(area, id.with("text"), Sense::hover());
    info.widget_info(|| WidgetInfo::labeled(egui::Role::Label, true, command));
    let copied_key = id.with("copied");
    let now = ui.input(|i| i.time);
    let copied_at = ui.data(|d| d.get_temp::<f64>(copied_key));
    let recently = copied_at.is_some_and(|at| now - at < 1.4);
    if recently {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    let button = Rect::from_center_size(rect.right_center() - vec2(22.0, 0.0), Vec2::splat(32.0));
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(id.with("copy"))
            .max_rect(button)
            .layout(Layout::centered_and_justified(Direction::LeftToRight)),
    );
    let tip = if recently { "Copied" } else { "Copy command" };
    let clicked = child
        .add(
            Btn::icon(
                if recently {
                    crate::icons::CHECK
                } else {
                    crate::icons::COPY
                },
                tip,
            )
            .color(if recently { pal.success } else { pal.text_dim }),
        )
        .on_hover_text(tip)
        .clicked();
    if clicked {
        ui.ctx().copy_text(command.to_string());
        ui.data_mut(|d| d.insert_temp(copied_key, now));
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame at `time` and returns how tall the revealed content's slot was.
    fn slot_height(ctx: &egui::Context, time: f64, open: bool) -> f32 {
        let mut height = 0.0;
        let input = egui::RawInput {
            time: Some(time),
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let top = ui.cursor().top();
            reveal(ui, Id::unique("section"), open, |ui| {
                ui.allocate_exact_size(vec2(100.0, 200.0), Sense::hover());
            });
            height = ui.cursor().top() - top;
        });
        output.textures_delta.clear();
        height
    }

    #[test]
    fn reveal_moves_through_intermediate_heights_then_settles() {
        let ctx = egui::Context::default();
        let dt = 1.0 / 60.0;
        assert_eq!(slot_height(&ctx, 0.0, false), 0.0);
        let mut heights = vec![];
        for frame in 1..40 {
            heights.push(slot_height(&ctx, frame as f64 * dt, true));
        }
        assert!(
            heights.iter().any(|h| *h > 20.0 && *h < 180.0),
            "opening should pass through in-between heights: {heights:?}"
        );
        assert!(heights.windows(2).all(|w| w[1] >= w[0]), "{heights:?}");
        assert!(
            (heights.last().unwrap() - 200.0).abs() < 8.5,
            "should settle at full height: {heights:?}"
        );
    }

    #[test]
    fn reveal_closed_from_the_start_never_runs_its_content() {
        let ctx = egui::Context::default();
        let mut ran = false;
        let input = egui::RawInput {
            time: Some(0.0),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            reveal(ui, Id::unique("hidden"), false, |_| ran = true);
        });
        output.textures_delta.clear();
        assert!(!ran);
    }

    #[test]
    fn plain_removes_icon_glyphs_from_accessible_names() {
        assert_eq!(plain("\u{E3D4}  New host"), "New host");
    }
}
