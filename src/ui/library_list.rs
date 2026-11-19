//! Row and group styling shared by the Host Library and Container Manager.
use egui::{
    Align2, Color32, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use super::{forms, glass, motion};
use crate::themes::{Palette, tokens};

pub(super) fn row_surface(ui: &Ui, row: &Response, selected: bool, pal: Palette) -> f32 {
    let rect = row.rect;
    let id = row.id;
    let hover = motion::ease(ui.ctx(), id.with("h"), row.hovered(), motion::FAST);
    let warm = motion::ease(ui.ctx(), id.with("warm"), selected, motion::FAST);
    let marker = motion::ease(ui.ctx(), id.with("mark"), selected, motion::CONTROL);
    let painter = ui.painter();
    painter.rect_filled(rect, 10, glass::hover_wash(ui).gamma_multiply(hover));
    painter.rect_filled(rect, 10, glass::selection_fill(ui).gamma_multiply(warm));
    if marker > 0.0 {
        let half = (rect.height() / 2.0 - 10.0) * marker;
        painter.vline(
            rect.left() + 1.0,
            (rect.center().y - half)..=(rect.center().y + half),
            Stroke::new(2.0, pal.accent),
        );
    }
    focus(ui, row, pal, 8);
    warm
}

pub(super) fn icon_tile(ui: &Ui, row: Rect, glyph: &str, ink: Color32) -> Rect {
    let size = if row.height() < (tokens::ROW + tokens::COMPACT_ROW) / 2.0 {
        28.0
    } else {
        32.0
    };
    let rect = Rect::from_center_size(
        pos2(row.left() + 12.0 + size / 2.0, row.center().y),
        vec2(size, size),
    );
    ui.painter().rect_filled(rect, 8, glass::hover_wash(ui));
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(16.0),
        ink,
    );
    rect
}

pub(super) fn group_header(
    ui: &Ui,
    rect: Rect,
    id: Id,
    name: &str,
    count: usize,
    closed: bool,
    pal: Palette,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        let action = if closed { "Expand" } else { "Collapse" };
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            ui.is_enabled(),
            format!("{action} {name}, {count}"),
        )
    });
    let hover = motion::ease(ui.ctx(), id.with("h"), response.hovered(), motion::FAST);
    let open = motion::ease_panel(ui.ctx(), id.with("turn"), !closed);
    let p = ui.painter();
    p.rect_filled(rect, 8, glass::hover_wash(ui).gamma_multiply(hover));
    let ink = pal.text_dim.lerp_to_gamma(pal.text, hover);
    forms::chevron(
        ui,
        rect.left_center() + vec2(14.0, 0.0),
        std::f32::consts::FRAC_PI_2 * open,
        ink,
    );
    let count_width = count.checked_ilog10().unwrap_or(0) as f32 * 8.0 + 8.0;
    let name_rect = Rect::from_min_max(
        pos2(rect.left() + 26.0, rect.top()),
        pos2(
            (rect.right() - count_width - 20.0).max(rect.left() + 26.0),
            rect.bottom(),
        ),
    );
    let name_pos = text(
        ui,
        name_rect,
        name,
        FontId::new(12.0, crate::fonts::medium()),
        ink,
        false,
    );
    p.text(
        pos2(
            (name_pos.right() + 8.0).min(rect.right() - count_width - 12.0),
            rect.center().y,
        ),
        Align2::LEFT_CENTER,
        count.to_string(),
        FontId::monospace(12.0),
        pal.text_dim,
    );
    focus(ui, &response, pal, 6);
    response
}

fn focus(ui: &Ui, response: &Response, pal: Palette, radius: u8) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(1.0),
            radius,
            Stroke::new(2.0, pal.accent),
            StrokeKind::Inside,
        );
    }
}

pub(super) fn resource_row(
    ui: &Ui,
    rect: Rect,
    id: Id,
    icon: &str,
    identity: (&str, &str),
    selected: bool,
    pal: Palette,
) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    let (name, subtitle) = identity;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::Button,
            ui.is_enabled(),
            selected,
            format!("{name}, {subtitle}"),
        )
    });
    if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Tab)) {
        response.scroll_to_me(None);
    }
    let warm = row_surface(ui, &response, selected, pal);
    icon_tile(ui, rect, icon, pal.text_dim.lerp_to_gamma(pal.accent, warm));
    response.on_hover_text(format!("{name}\n{subtitle}"))
}

pub(super) fn identity(ui: &Ui, cell: Rect, name: &str, subtitle: &str, pal: Palette) {
    let padding = if cell.height() < (tokens::ROW + tokens::COMPACT_ROW) / 2.0 {
        52.0
    } else {
        56.0
    };
    let cell = Rect::from_min_max(
        pos2(cell.left() + padding, cell.top()),
        pos2(cell.right().max(cell.left() + padding), cell.bottom()),
    );
    if subtitle.is_empty() {
        text(
            ui,
            cell,
            name,
            FontId::new(14.0, crate::fonts::medium()),
            pal.text,
            false,
        );
    } else {
        let (top, bottom) = lines(cell);
        text(
            ui,
            top,
            name,
            FontId::new(14.0, crate::fonts::medium()),
            pal.text,
            false,
        );
        text(
            ui,
            bottom,
            subtitle,
            FontId::monospace(12.0),
            pal.text_dim,
            false,
        );
    }
}

pub(super) fn lines(cell: Rect) -> (Rect, Rect) {
    let center = cell.center().y;
    (
        Rect::from_min_max(pos2(cell.left(), center - 18.0), pos2(cell.right(), center)),
        Rect::from_min_max(pos2(cell.left(), center), pos2(cell.right(), center + 18.0)),
    )
}

pub(super) fn text(
    ui: &Ui,
    cell: Rect,
    value: &str,
    font: FontId,
    ink: Color32,
    right: bool,
) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(value.into(), font, ink);
    job.wrap.max_width = cell.width().max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let painter = ui.painter().with_clip_rect(cell);
    let galley = painter.layout_job(job);
    let pos = pos2(
        if right {
            cell.right() - galley.size().x
        } else {
            cell.left()
        },
        cell.center().y - galley.size().y / 2.0,
    );
    let rect = Rect::from_min_size(pos, galley.size());
    painter.galley(pos, galley, ink);
    rect
}
