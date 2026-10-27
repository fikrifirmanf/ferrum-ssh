//! Terminal widget: renders an alacritty grid with egui and translates
//! keyboard / mouse input into terminal byte sequences.

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::vte::ansi::{Color, CursorShape as VtCursor, NamedColor};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Vec2};

use crate::conn::Session;
use crate::model::{BackspaceMode, CursorShape};
use crate::term::Input;
use crate::themes::{TermScheme, indexed_color};

pub struct ViewConfig<'a> {
    pub scheme: &'a TermScheme,
    pub font: FontId,
    pub line_height: f32,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub keywords: &'a [(regex::Regex, Color32)],
    pub copy_on_select: bool,
    pub right_click_paste: bool,
    pub option_is_meta: bool,
    pub brighten_bold: bool,
    pub suggestion: Option<String>,
    pub focused: bool,
}

#[derive(Default)]
pub struct ViewState {
    pub search_open: bool,
    pub search_query: String,
    pub search_regex: Option<RegexSearch>,
    pub search_origin: Option<Point>,
    pub search_status: String,
    scroll_accum: f32,
    mouse_down_btn: Option<u8>,
    selecting: bool,
}

#[derive(Default)]
pub struct ViewOutput {
    /// Bytes produced by the user (to be sent to the session, and broadcast).
    pub input: Vec<u8>,
    pub entered_line: Option<String>,
    pub copied: Option<String>,
    pub clicked: bool,
    pub open_url: Option<String>,
    pub accepted_suggestion: bool,
    pub context: Option<ContextAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextAction {
    SplitRight,
    SplitDown,
    ClosePane,
    OpenSftp,
    Clear,
    Find,
    SelectAll,
}

fn resolve_color(
    c: Color,
    scheme: &TermScheme,
    colors: &alacritty_terminal::term::color::Colors,
) -> Color32 {
    match c {
        Color::Spec(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => {
            if let Some(rgb) = colors[i as usize] {
                Color32::from_rgb(rgb.r, rgb.g, rgb.b)
            } else {
                indexed_color(scheme, i)
            }
        }
        Color::Named(n) => {
            let idx = n as usize;
            if idx < alacritty_terminal::term::color::COUNT {
                if let Some(rgb) = colors[idx] {
                    return Color32::from_rgb(rgb.r, rgb.g, rgb.b);
                }
            }
            match idx {
                0..=15 => scheme.ansi[idx],
                256 => scheme.fg,
                257 => scheme.bg,
                258 => scheme.cursor,
                259..=266 => scheme.ansi[idx - 259].gamma_multiply(0.66),
                267 => scheme.fg,
                268 => scheme.fg.gamma_multiply(0.66),
                _ => scheme.fg,
            }
        }
    }
}

fn is_default_fg(c: &Color) -> bool {
    matches!(c, Color::Named(NamedColor::Foreground))
}

fn brighten(c: Color) -> Color {
    match c {
        Color::Named(n) if (n as usize) < 8 => Color::Indexed(n as u8 + 8),
        Color::Indexed(i) if i < 8 => Color::Indexed(i + 8),
        other => other,
    }
}

struct RowCell {
    c: char,
    fg: Color32,
    bg: Color32,
    flags: Flags,
    default_fg: bool,
    selected: bool,
    zw: Option<String>,
}

pub fn cell_metrics(ui: &egui::Ui, font: &FontId, line_height: f32) -> (f32, f32) {
    let (w, h) = ui
        .ctx()
        .fonts_mut(|f| (f.glyph_width(font, 'M'), f.row_height(font)));
    (w.max(1.0), (h * line_height).round().max(1.0))
}

fn modifier_param(m: &egui::Modifiers) -> u8 {
    1 + (m.shift as u8) + (m.alt as u8) * 2 + (m.ctrl as u8) * 4
}

fn key_bytes(
    key: Key,
    m: &egui::Modifiers,
    mode: TermMode,
    backspace: BackspaceMode,
    option_meta: bool,
) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let has_mods = m.shift || m.alt || m.ctrl;
    let csi_mod = |final_: char| -> Vec<u8> {
        format!("\x1b[1;{}{}", modifier_param(m), final_).into_bytes()
    };
    let arrow = |c: char| -> Vec<u8> {
        if has_mods {
            csi_mod(c)
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if has_mods {
            format!("\x1b[{n};{}~", modifier_param(m)).into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    Some(match key {
        Key::Enter => {
            if m.alt && option_meta {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        Key::Backspace => {
            let base = match backspace {
                BackspaceMode::Del => 0x7f,
                BackspaceMode::CtrlH => 0x08,
            };
            if m.alt && option_meta {
                vec![0x1b, base]
            } else if m.ctrl {
                vec![0x08]
            } else {
                vec![base]
            }
        }
        Key::Tab => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        Key::Escape => b"\x1b".to_vec(),
        Key::ArrowUp => arrow('A'),
        Key::ArrowDown => arrow('B'),
        Key::ArrowRight => arrow('C'),
        Key::ArrowLeft => arrow('D'),
        Key::Home => arrow('H'),
        Key::End => arrow('F'),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::F1 => {
            if has_mods {
                csi_mod('P')
            } else {
                b"\x1bOP".to_vec()
            }
        }
        Key::F2 => {
            if has_mods {
                csi_mod('Q')
            } else {
                b"\x1bOQ".to_vec()
            }
        }
        Key::F3 => {
            if has_mods {
                csi_mod('R')
            } else {
                b"\x1bOR".to_vec()
            }
        }
        Key::F4 => {
            if has_mods {
                csi_mod('S')
            } else {
                b"\x1bOS".to_vec()
            }
        }
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        _ => return None,
    })
}

fn ctrl_byte(key: Key) -> Option<u8> {
    let name = key.name();
    if name.len() == 1 {
        let c = name.chars().next()?.to_ascii_lowercase();
        if c.is_ascii_lowercase() {
            return Some(c as u8 - b'a' + 1);
        }
        return match c {
            '2' | '@' => Some(0),
            '3' | '[' => Some(0x1b),
            '4' | '\\' => Some(0x1c),
            '5' | ']' => Some(0x1d),
            '6' | '^' => Some(0x1e),
            '7' | '-' | '/' => Some(0x1f),
            '8' => Some(0x7f),
            _ => None,
        };
    }
    match key {
        Key::Space => Some(0),
        Key::OpenBracket => Some(0x1b),
        Key::Backslash => Some(0x1c),
        Key::CloseBracket => Some(0x1d),
        Key::Minus | Key::Slash => Some(0x1f),
        _ => None,
    }
}

fn mouse_report(
    mode: TermMode,
    button: u8,
    col: usize,
    row: usize,
    pressed: bool,
    motion: bool,
) -> Vec<u8> {
    let b = button + if motion { 32 } else { 0 };
    if mode.contains(TermMode::SGR_MOUSE) {
        format!(
            "\x1b[<{};{};{}{}",
            b,
            col + 1,
            row + 1,
            if pressed { 'M' } else { 'm' }
        )
        .into_bytes()
    } else {
        let b = if pressed { b } else { 3 };
        let cx = (col + 33).min(255) as u8;
        let cy = (row + 33).min(255) as u8;
        vec![0x1b, b'[', b'M', 32 + b, cx, cy]
    }
}

pub fn show(
    ui: &mut egui::Ui,
    sess: &mut Session,
    st: &mut ViewState,
    cfg: &ViewConfig,
    backspace: BackspaceMode,
) -> ViewOutput {
    let mut out = ViewOutput::default();
    let scheme = cfg.scheme;
    let (cell_w, cell_h) = cell_metrics(ui, &cfg.font, cfg.line_height);
    let full = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(full, Sense::click_and_drag());
    let id = resp.id;
    let pad = 8.0;
    let inner = full.shrink(pad);
    let cols = ((inner.width() / cell_w).floor() as usize).max(2);
    let rows = ((inner.height() / cell_h).floor() as usize).max(1);
    if (cols as u16, rows as u16) != sess.last_size {
        sess.last_size = (cols as u16, rows as u16);
        sess.term
            .lock()
            .resize(cols, rows, cell_w as u16, cell_h as u16);
        sess.send_input(Input::Resize {
            cols: cols as u16,
            rows: rows as u16,
            px_w: (cols as f32 * cell_w) as u16,
            px_h: (rows as f32 * cell_h) as u16,
        });
    }

    if resp.clicked() || resp.drag_started() || cfg.focused && !ui.memory(|m| m.focused().is_some())
    {
        resp.request_focus();
    }
    if resp.clicked() {
        out.clicked = true;
    }
    let has_focus = resp.has_focus();
    if has_focus {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }

    let painter = ui.painter_at(full);
    painter.rect_filled(full, 0.0, scheme.bg);

    let origin = inner.min;
    let to_cell = |pos: Pos2, display_offset: usize| -> (Point, Side, usize, usize) {
        let x = ((pos.x - origin.x) / cell_w).max(0.0);
        let col = (x.floor() as usize).min(cols - 1);
        let row = (((pos.y - origin.y) / cell_h).max(0.0).floor() as usize).min(rows - 1);
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        (
            Point::new(Line(row as i32 - display_offset as i32), Column(col)),
            side,
            col,
            row,
        )
    };

    let mut term = sess.term.lock();
    let mode = *term.term.mode();
    let mouse_mode = mode.intersects(TermMode::MOUSE_MODE);
    let display_offset = term.term.grid().display_offset();
    let modifiers = ui.input(|i| i.modifiers);

    // ---------------- Mouse ----------------
    if let Some(pos) = resp.interact_pointer_pos().or(resp.hover_pos()) {
        let (point, side, col, row) = to_cell(pos, display_offset);
        if mouse_mode && !modifiers.shift {
            let pressed_btn = ui.input(|i| {
                if i.pointer.button_pressed(egui::PointerButton::Primary) {
                    Some(0)
                } else if i.pointer.button_pressed(egui::PointerButton::Middle) {
                    Some(1)
                } else if i.pointer.button_pressed(egui::PointerButton::Secondary) {
                    Some(2)
                } else {
                    None
                }
            });
            if resp.hovered() || st.mouse_down_btn.is_some() {
                if let Some(b) = pressed_btn {
                    st.mouse_down_btn = Some(b);
                    out.input
                        .extend(mouse_report(mode, b, col, row, true, false));
                }
                if let Some(b) = st.mouse_down_btn {
                    if ui.input(|i| i.pointer.any_released()) {
                        out.input
                            .extend(mouse_report(mode, b, col, row, false, false));
                        st.mouse_down_btn = None;
                    } else if resp.dragged()
                        && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                    {
                        out.input
                            .extend(mouse_report(mode, b, col, row, true, true));
                    }
                }
            }
        } else {
            if resp.drag_started_by(egui::PointerButton::Primary) {
                let ty = if modifiers.alt {
                    SelectionType::Block
                } else {
                    SelectionType::Simple
                };
                term.term.selection = Some(Selection::new(ty, point, side));
                st.selecting = true;
            } else if resp.dragged_by(egui::PointerButton::Primary) && st.selecting {
                if let Some(sel) = term.term.selection.as_mut() {
                    sel.update(point, side);
                }
                // Auto-scroll while dragging beyond edges.
                if pos.y < inner.top() {
                    term.term.scroll_display(Scroll::Delta(1));
                } else if pos.y > inner.bottom() {
                    term.term.scroll_display(Scroll::Delta(-1));
                }
            }
            if resp.drag_stopped() && st.selecting {
                st.selecting = false;
                if cfg.copy_on_select {
                    if let Some(t) = term.term.selection_to_string().filter(|t| !t.is_empty()) {
                        out.copied = Some(t);
                    }
                }
            }
            if resp.double_clicked() {
                term.term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
                if cfg.copy_on_select {
                    out.copied = term.term.selection_to_string();
                }
            } else if resp.triple_clicked() {
                term.term.selection = Some(Selection::new(SelectionType::Lines, point, side));
                if cfg.copy_on_select {
                    out.copied = term.term.selection_to_string();
                }
            } else if resp.clicked() {
                term.term.selection = None;
                if modifiers.command || modifiers.ctrl {
                    // Open URL under cursor.
                    let line = row_text(&term.term, Line(row as i32 - display_offset as i32), cols);
                    if let Some(url) = url_at(&line.0, &line.1, col) {
                        out.open_url = Some(url);
                    }
                }
            }
        }
        if resp.secondary_clicked() && cfg.right_click_paste && !mouse_mode {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                if let Ok(text) = cb.get_text() {
                    out.input.extend(paste_bytes(&text, mode));
                    lose_track(&mut sess.line_buf, &mut sess.line_untracked);
                }
            }
        }
    }

    // Scroll wheel
    if resp.hovered() {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            st.scroll_accum += dy;
            let lines = (st.scroll_accum / cell_h).trunc() as i32;
            if lines != 0 {
                st.scroll_accum -= lines as f32 * cell_h;
                if mouse_mode && !modifiers.shift {
                    if let Some(pos) = resp.hover_pos() {
                        let (_, _, col, row) = to_cell(pos, display_offset);
                        let b = if lines > 0 { 64 } else { 65 };
                        for _ in 0..lines.abs() {
                            out.input
                                .extend(mouse_report(mode, b, col, row, true, false));
                        }
                    }
                } else if mode.contains(TermMode::ALT_SCREEN)
                    && mode.contains(TermMode::ALTERNATE_SCROLL)
                {
                    let seq: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                    for _ in 0..lines.abs() {
                        out.input.extend_from_slice(seq);
                    }
                } else {
                    term.term.scroll_display(Scroll::Delta(lines));
                }
            }
        }
    }

    // Only suggest when the screen agrees with what was typed: the cursor sits right after it
    // and nothing (such as the shell's own suggestion) follows. This also hides suggestions
    // for text that is not echoed, like a password.
    let suggestion = cfg.suggestion.clone().filter(|_| {
        has_focus && !st.search_open && display_offset == 0 && !sess.line_untracked && {
            let (before, cursor_free) = line_before_cursor(&term.term);
            cursor_free
                && before.ends_with(&sess.line_buf)
                && !crate::suggest::is_secret_prompt(&before)
        }
    });

    // ---------------- Keyboard ----------------
    let mut typed_any = false;
    if has_focus && !st.search_open {
        let events = ui.input(|i| i.events.clone());
        let mut skip_text = false;
        for ev in events {
            match ev {
                egui::Event::Text(t) => {
                    if skip_text {
                        skip_text = false;
                        continue;
                    }
                    if !sess.line_untracked {
                        sess.line_buf.push_str(&t);
                    }
                    out.input
                        .extend(crate::term::encode_input(&sess.target.charset, &t));
                    typed_any = true;
                }
                egui::Event::Paste(t) => {
                    out.input.extend(paste_bytes(&t, mode));
                    if t.contains(['\n', '\r']) {
                        lose_track(&mut sess.line_buf, &mut sess.line_untracked);
                    } else if !sess.line_untracked {
                        sess.line_buf.push_str(&t);
                    }
                    typed_any = true;
                }
                egui::Event::Copy => {
                    if let Some(t) = term.term.selection_to_string().filter(|t| !t.is_empty()) {
                        out.copied = Some(t);
                    } else if !cfg!(target_os = "macos") {
                        out.input.push(0x03);
                        sess.line_buf.clear();
                        sess.line_untracked = false;
                    }
                }
                egui::Event::Cut => {
                    if !cfg!(target_os = "macos") {
                        out.input.push(0x18);
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers: m,
                    ..
                } => {
                    // App-level shortcuts are handled elsewhere.
                    if m.mac_cmd || (!cfg!(target_os = "macos") && m.ctrl && m.shift) {
                        if !cfg!(target_os = "macos") && m.ctrl && m.shift {
                            match key {
                                Key::C => {
                                    if let Some(t) = term.term.selection_to_string() {
                                        out.copied = Some(t);
                                    }
                                }
                                Key::V => {
                                    if let Ok(mut cb) = arboard::Clipboard::new() {
                                        if let Ok(text) = cb.get_text() {
                                            out.input.extend(paste_bytes(&text, mode));
                                            lose_track(
                                                &mut sess.line_buf,
                                                &mut sess.line_untracked,
                                            );
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        continue;
                    }
                    if m.shift
                        && matches!(key, Key::PageUp | Key::PageDown)
                        && !mode.contains(TermMode::ALT_SCREEN)
                    {
                        term.term.scroll_display(if key == Key::PageUp {
                            Scroll::PageUp
                        } else {
                            Scroll::PageDown
                        });
                        continue;
                    }
                    // A suggestion computed before this frame's typing would be stale.
                    if key == Key::ArrowRight && !m.any() && !typed_any {
                        if let Some(sug) = &suggestion {
                            out.input
                                .extend(crate::term::encode_input(&sess.target.charset, sug));
                            sess.line_buf.push_str(sug);
                            out.accepted_suggestion = true;
                            continue;
                        }
                    }
                    if m.ctrl && !m.alt {
                        if let Some(b) = ctrl_byte(key) {
                            out.input.push(b);
                            match b {
                                0x03 => {
                                    sess.line_buf.clear();
                                    sess.line_untracked = false;
                                }
                                0x15 => sess.line_buf.clear(),
                                // Ctrl+L only redraws the line.
                                0x0c => {}
                                _ => lose_track(&mut sess.line_buf, &mut sess.line_untracked),
                            }
                            skip_text = true;
                            continue;
                        }
                    }
                    if m.alt && cfg.option_is_meta && !m.ctrl {
                        let name = key.name();
                        if name.chars().count() == 1 {
                            let c = name.chars().next().unwrap();
                            let c = if m.shift {
                                c.to_ascii_uppercase()
                            } else {
                                c.to_ascii_lowercase()
                            };
                            out.input.push(0x1b);
                            out.input.extend(c.to_string().as_bytes());
                            lose_track(&mut sess.line_buf, &mut sess.line_untracked);
                            skip_text = true;
                            continue;
                        }
                    }
                    if let Some(bytes) = key_bytes(key, &m, mode, backspace, cfg.option_is_meta) {
                        match key {
                            Key::Enter => {
                                let line = std::mem::take(&mut sess.line_buf);
                                let tracked = !std::mem::take(&mut sess.line_untracked);
                                // Never keep what was typed at a password prompt.
                                if tracked
                                    && !mode.contains(TermMode::ALT_SCREEN)
                                    && !line.trim().is_empty()
                                    && !crate::suggest::is_secret_prompt(
                                        &line_before_cursor(&term.term).0,
                                    )
                                {
                                    out.entered_line = Some(line);
                                }
                            }
                            Key::Backspace if !m.alt && !m.ctrl => {
                                sess.line_buf.pop();
                            }
                            Key::Backspace
                            | Key::ArrowUp
                            | Key::ArrowDown
                            | Key::ArrowLeft
                            | Key::ArrowRight
                            | Key::Home
                            | Key::End
                            | Key::Delete
                            | Key::PageUp
                            | Key::PageDown
                            | Key::Tab
                            | Key::Escape => {
                                lose_track(&mut sess.line_buf, &mut sess.line_untracked)
                            }
                            _ => {}
                        }
                        out.input.extend(bytes);
                        typed_any = true;
                    }
                }
                _ => {}
            }
        }
    }
    if typed_any && display_offset > 0 {
        term.term.scroll_display(Scroll::Bottom);
    }

    // ---------------- Render ----------------
    let display_offset = term.term.grid().display_offset();
    let alt_screen = mode.contains(TermMode::ALT_SCREEN);
    let content = term.term.renderable_content();
    let selection = content.selection;
    let colors = content.colors;
    let cursor = content.cursor;
    let mut grid_rows: Vec<Vec<RowCell>> = (0..rows).map(|_| Vec::with_capacity(cols)).collect();
    for ind in content.display_iter {
        let r = ind.point.line.0 + display_offset as i32;
        if r < 0 || r as usize >= rows {
            continue;
        }
        let cell = &ind.cell;
        let mut fg_c = cell.fg;
        if cfg.brighten_bold && cell.flags.contains(Flags::BOLD) {
            fg_c = brighten(fg_c);
        }
        let mut fg = resolve_color(fg_c, scheme, colors);
        let mut bg = resolve_color(cell.bg, scheme, colors);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::DIM) {
            fg = fg.gamma_multiply(0.66);
        }
        if cell.flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let selected = selection.map(|s| s.contains(ind.point)).unwrap_or(false);
        let zw = cell.zerowidth().map(|z| z.iter().collect::<String>());
        grid_rows[r as usize].push(RowCell {
            c: cell.c,
            fg,
            bg,
            flags: cell.flags,
            default_fg: is_default_fg(&cell.fg) && !cell.flags.contains(Flags::INVERSE),
            selected,
            zw,
        });
    }

    let selection_bg = scheme.selection;
    for (r, row) in grid_rows.iter_mut().enumerate() {
        let y = origin.y + r as f32 * cell_h;
        // Keyword highlighting (skip full-screen apps).
        if !alt_screen && !cfg.keywords.is_empty() {
            let mut text = String::with_capacity(row.len());
            let mut char_cols = Vec::with_capacity(row.len());
            for (ci, c) in row.iter().enumerate() {
                if c.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                text.push(c.c);
                char_cols.push(ci);
            }
            if !text.trim().is_empty() {
                for (re, color) in cfg.keywords {
                    for m in re.find_iter(&text) {
                        let start = text[..m.start()].chars().count();
                        let len = m.as_str().chars().count();
                        for k in start..start + len {
                            if let Some(&ci) = char_cols.get(k) {
                                if row[ci].default_fg {
                                    row[ci].fg = *color;
                                }
                            }
                        }
                    }
                }
            }
        }
        // Backgrounds.
        let mut c = 0;
        while c < row.len() {
            let bg = if row[c].selected {
                selection_bg
            } else {
                row[c].bg
            };
            let start = c;
            while c < row.len()
                && (if row[c].selected {
                    selection_bg
                } else {
                    row[c].bg
                }) == bg
            {
                c += 1;
            }
            if bg != scheme.bg {
                let rect = Rect::from_min_size(
                    Pos2::new(origin.x + start as f32 * cell_w, y),
                    Vec2::new((c - start) as f32 * cell_w + 0.5, cell_h + 0.5),
                );
                painter.rect_filled(rect, 0.0, bg);
            }
        }
        // Text runs.
        let mut c = 0;
        while c < row.len() {
            let cell = &row[c];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                || (cell.c == ' '
                    && !cell
                        .flags
                        .intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT))
            {
                c += 1;
                continue;
            }
            let start = c;
            let style = (
                cell.fg,
                cell.flags
                    & (Flags::BOLD | Flags::ITALIC | Flags::ALL_UNDERLINES | Flags::STRIKEOUT),
            );
            let mut text = String::new();
            if cell.flags.contains(Flags::WIDE_CHAR) {
                text.push(cell.c);
                if let Some(z) = &cell.zw {
                    text.push_str(z);
                }
                c += 1;
            } else {
                while c < row.len() {
                    let cc = &row[c];
                    let s2 = (
                        cc.fg,
                        cc.flags
                            & (Flags::BOLD
                                | Flags::ITALIC
                                | Flags::ALL_UNDERLINES
                                | Flags::STRIKEOUT),
                    );
                    if s2 != style
                        || cc
                            .flags
                            .intersects(Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER)
                        || cc.zw.is_some() && c != start
                    {
                        break;
                    }
                    text.push(cc.c);
                    if let Some(z) = &cc.zw {
                        text.push_str(z);
                        c += 1;
                        break;
                    }
                    c += 1;
                }
            }
            let mut job = LayoutJob::default();
            let underline = if style.1.intersects(Flags::ALL_UNDERLINES) {
                Stroke::new(1.0, style.0)
            } else {
                Stroke::NONE
            };
            let strike = if style.1.contains(Flags::STRIKEOUT) {
                Stroke::new(1.0, style.0)
            } else {
                Stroke::NONE
            };
            job.append(
                &text,
                0.0,
                TextFormat {
                    font_id: cfg.font.clone(),
                    color: style.0,
                    italics: style.1.contains(Flags::ITALIC),
                    underline,
                    strikethrough: strike,
                    ..Default::default()
                },
            );
            let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
            let ty = y + (cell_h - galley.size().y) / 2.0;
            let pos = Pos2::new(origin.x + start as f32 * cell_w, ty);
            painter.galley(pos, galley.clone(), style.0);
            if style.1.contains(Flags::BOLD) {
                // Faux bold: overdraw with a sub-pixel offset.
                painter.galley(pos + Vec2::new(0.6, 0.0), galley, style.0);
            }
        }
    }

    // Cursor
    let cursor_row = cursor.point.line.0 + display_offset as i32;
    if cursor.shape != VtCursor::Hidden
        && cursor_row >= 0
        && (cursor_row as usize) < rows
        && mode.contains(TermMode::SHOW_CURSOR)
    {
        let blink_on =
            !cfg.cursor_blink || !has_focus || (ui.input(|i| i.time) * 1.6) as i64 % 2 == 0;
        if cfg.cursor_blink && has_focus {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(300));
        }
        let x = origin.x + cursor.point.column.0 as f32 * cell_w;
        let y = origin.y + cursor_row as f32 * cell_h;
        let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w, cell_h));
        let shape = match cursor.shape {
            VtCursor::Beam => CursorShape::Beam,
            VtCursor::Underline => CursorShape::Underline,
            _ => cfg.cursor_shape,
        };
        if !has_focus {
            painter.rect_stroke(
                rect.shrink(0.5),
                0.0,
                Stroke::new(1.0, scheme.cursor),
                egui::StrokeKind::Inside,
            );
        } else if blink_on {
            match shape {
                CursorShape::Block => {
                    painter.rect_filled(rect, 1.0, scheme.cursor);
                    let ch = grid_rows
                        .get(cursor_row as usize)
                        .and_then(|r| r.get(cursor.point.column.0))
                        .map(|c| c.c)
                        .unwrap_or(' ');
                    if ch != ' ' {
                        painter.text(
                            rect.left_center(),
                            egui::Align2::LEFT_CENTER,
                            ch,
                            cfg.font.clone(),
                            scheme.bg,
                        );
                    }
                }
                CursorShape::Beam => {
                    painter.rect_filled(
                        Rect::from_min_size(rect.min, Vec2::new(2.0, cell_h)),
                        0.0,
                        scheme.cursor,
                    );
                }
                CursorShape::Underline => {
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y + cell_h - 2.0), Vec2::new(cell_w, 2.0)),
                        0.0,
                        scheme.cursor,
                    );
                }
            }
        }
        // Suggestion ghost text, starting in the cursor cell and cut at the window edge.
        if let Some(sug) = &suggestion {
            let room = cols.saturating_sub(cursor.point.column.0);
            let mut ghost = sug.chars().take(room);
            let mut start = x;
            // A block cursor covers its cell, so the first character is drawn on it.
            if blink_on && matches!(shape, CursorShape::Block) {
                if let Some(first) = ghost.next() {
                    painter.text(
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        first,
                        cfg.font.clone(),
                        scheme.bg.gamma_multiply(0.7),
                    );
                    start += cell_w;
                }
            }
            painter.text(
                Pos2::new(start, y + cell_h / 2.0),
                egui::Align2::LEFT_CENTER,
                ghost.collect::<String>(),
                cfg.font.clone(),
                scheme.fg.gamma_multiply(0.35),
            );
        }
        // IME candidate window position.
        if has_focus {
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Terminal,
                    rect: full,
                    cursor_rect: rect,
                    should_interrupt_composition: false,
                });
            });
        }
    }

    // Scrollback indicator
    if display_offset > 0 {
        let total = term.term.grid().history_size() + rows;
        let frac_top = (term.term.grid().history_size() - display_offset) as f32 / total as f32;
        let frac_h = rows as f32 / total as f32;
        let bar = Rect::from_min_size(
            Pos2::new(full.right() - 5.0, full.top() + frac_top * full.height()),
            Vec2::new(3.0, (frac_h * full.height()).max(20.0)),
        );
        painter.rect_filled(bar, 2.0, scheme.fg.gamma_multiply(0.35));
    }
    drop(term);

    // ---------------- Search overlay ----------------
    if has_focus && ui.input(|i| i.modifiers.command && i.key_pressed(Key::F)) {
        st.search_open = true;
    }
    if st.search_open {
        search_overlay(ui, sess, st, full);
    }

    // ---------------- Context menu ----------------
    if !cfg.right_click_paste || mouse_mode {
        resp.context_menu(|ui| {
            ui.set_min_width(180.0);
            let has_sel = sess.term.lock().term.selection.is_some();
            if ui.add_enabled(has_sel, egui::Button::new("Copy")).clicked() {
                out.copied = sess.term.lock().term.selection_to_string();
                ui.close();
            }
            if ui.button("Paste").clicked() {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    if let Ok(text) = cb.get_text() {
                        out.input.extend(paste_bytes(&text, mode));
                    }
                }
                ui.close();
            }
            context_items(ui, &mut out);
        });
    } else if resp.secondary_clicked() && modifiers.shift {
        // Shift+right click opens the menu even when right-click pastes.
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if mouse_mode {
            egui::CursorIcon::Default
        } else {
            egui::CursorIcon::Text
        });
    }
    out
}

fn context_items(ui: &mut egui::Ui, out: &mut ViewOutput) {
    ui.separator();
    for (label, action) in [
        ("Select all", ContextAction::SelectAll),
        ("Find…", ContextAction::Find),
        ("Clear screen", ContextAction::Clear),
    ] {
        if ui.button(label).clicked() {
            out.context = Some(action);
            ui.close();
        }
    }
    ui.separator();
    for (label, action) in [
        ("Split right", ContextAction::SplitRight),
        ("Split down", ContextAction::SplitDown),
        ("Open SFTP", ContextAction::OpenSftp),
        ("Close pane", ContextAction::ClosePane),
    ] {
        if ui.button(label).clicked() {
            out.context = Some(action);
            ui.close();
        }
    }
}

pub fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let text = text.replace("\r\n", "\n").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let clean = text.replace("\x1b[201~", "");
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

fn row_text<T>(
    term: &alacritty_terminal::Term<T>,
    line: Line,
    cols: usize,
) -> (String, Vec<usize>) {
    let grid = term.grid();
    let mut s = String::new();
    let mut map = vec![];
    if line.0 < -(grid.history_size() as i32) || line.0 >= grid.screen_lines() as i32 {
        return (s, map);
    }
    let row = &grid[line];
    for c in 0..cols.min(grid.columns()) {
        let cell = &row[Column(c)];
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        s.push(cell.c);
        map.push(c);
    }
    (s, map)
}

/// Stop following the typed line: an edit happened that `line_buf` cannot mirror.
fn lose_track(line_buf: &mut String, untracked: &mut bool) {
    line_buf.clear();
    *untracked = true;
}

/// What the screen shows of the cursor's logical line up to the cursor (wrapped rows joined),
/// and whether the cursor cell is empty.
fn line_before_cursor<T>(term: &alacritty_terminal::Term<T>) -> (String, bool) {
    const MAX_WRAPPED_ROWS: i32 = 12;
    let grid = term.grid();
    let cursor = grid.cursor.point;
    let cols = grid.columns();
    let top = -(grid.history_size() as i32);
    let mut first = cursor.line.0;
    while first > top
        && cursor.line.0 - first < MAX_WRAPPED_ROWS
        && grid[Line(first - 1)][Column(cols - 1)]
            .flags
            .contains(Flags::WRAPLINE)
    {
        first -= 1;
    }
    let mut text = String::new();
    for l in first..=cursor.line.0 {
        let row = &grid[Line(l)];
        let end = if l == cursor.line.0 {
            cursor.column.0.min(cols)
        } else {
            cols
        };
        for c in 0..end {
            let cell = &row[Column(c)];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                text.push(cell.c);
            }
        }
    }
    let free = grid[cursor.line][Column(cursor.column.0.min(cols - 1))].c == ' ';
    (text, free)
}

fn url_at(text: &str, map: &[usize], col: usize) -> Option<String> {
    let re = regex::Regex::new(r#"(?:https?|ftp)://[^\s<>"'`]+"#).ok()?;
    for m in re.find_iter(text) {
        let start = text[..m.start()].chars().count();
        let len = m.as_str().chars().count();
        let (a, b) = (map.get(start)?, map.get(start + len - 1)?);
        if col >= *a && col <= *b {
            return Some(
                m.as_str()
                    .trim_end_matches(['.', ',', ')', ']'])
                    .to_string(),
            );
        }
    }
    None
}

fn search_overlay(ui: &mut egui::Ui, sess: &mut Session, st: &mut ViewState, area: Rect) {
    let pos = Pos2::new(area.right() - 360.0, area.top() + 10.0);
    egui::Area::new(egui::Id::new(("term-search", sess.id)))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut st.search_query)
                            .hint_text("Find in terminal…")
                            .desired_width(180.0),
                    );
                    if !te.has_focus() && !te.lost_focus() && st.search_regex.is_none() {
                        te.request_focus();
                    }
                    if te.changed() {
                        st.search_regex = None;
                        st.search_origin = None;
                    }
                    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let shift = ui.input(|i| i.modifiers.shift);
                    let prev = ui.button("▲").on_hover_text("Previous (Enter)").clicked()
                        || enter && !shift;
                    let next = ui.button("▼").on_hover_text("Next (Shift+Enter)").clicked()
                        || enter && shift;
                    if prev || next {
                        do_search(
                            sess,
                            st,
                            if prev {
                                Direction::Left
                            } else {
                                Direction::Right
                            },
                        );
                        te.request_focus();
                    }
                    ui.label(egui::RichText::new(&st.search_status).small().weak());
                    if ui.button("×").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        st.search_open = false;
                        st.search_regex = None;
                        sess.term.lock().term.selection = None;
                    }
                });
            });
        });
}

fn do_search(sess: &mut Session, st: &mut ViewState, dir: Direction) {
    if st.search_query.is_empty() {
        return;
    }
    if st.search_regex.is_none() {
        match RegexSearch::new(&format!("(?i){}", regex::escape(&st.search_query))) {
            Ok(r) => st.search_regex = Some(r),
            Err(_) => {
                st.search_status = "invalid".into();
                return;
            }
        }
    }
    let mut guard = sess.term.lock();
    let term = &mut guard.term;
    let origin = st.search_origin.unwrap_or_else(|| {
        let grid = term.grid();
        let line = Line(grid.screen_lines() as i32 - 1);
        Point::new(line, Column(grid.columns().saturating_sub(1)))
    });
    let regex = st.search_regex.as_mut().unwrap();
    let side = if dir == Direction::Left {
        Side::Left
    } else {
        Side::Right
    };
    let start = match dir {
        Direction::Left => origin.sub(term, alacritty_terminal::index::Boundary::None, 1),
        Direction::Right => origin.add(term, alacritty_terminal::index::Boundary::None, 1),
    };
    match term.search_next(regex, start, dir, side, None) {
        Some(m) => {
            let (s, e) = (*m.start(), *m.end());
            let mut sel = Selection::new(SelectionType::Simple, s, Side::Left);
            sel.update(e, Side::Right);
            term.selection = Some(sel);
            term.scroll_to_point(s);
            st.search_origin = Some(s);
            st.search_status = format!("line {}", s.line.0);
        }
        None => st.search_status = "no matches".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::TermSize;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;

    fn screen(cols: usize, output: &str) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &TermSize { cols, rows: 6 }, VoidListener);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new()
            .advance(&mut term, output.as_bytes());
        term
    }

    #[test]
    fn reads_typed_text_up_to_the_cursor() {
        let term = screen(40, "user@host:~$ git sta");
        let (before, free) = line_before_cursor(&term);
        assert_eq!(before, "user@host:~$ git sta");
        assert!(free);
    }

    #[test]
    fn joins_a_command_wrapped_over_several_rows() {
        let term = screen(10, "$ echo aaaaaaaaaaaaaaa");
        let (before, _) = line_before_cursor(&term);
        assert_eq!(before, "$ echo aaaaaaaaaaaaaaa");
    }

    #[test]
    fn does_not_reach_back_past_a_finished_line() {
        let term = screen(40, "old output\r\n$ ls");
        assert_eq!(line_before_cursor(&term).0, "$ ls");
    }

    #[test]
    fn text_after_the_cursor_marks_a_shell_suggestion() {
        // The shell printed its own grey suggestion, then moved the cursor back over it.
        let term = screen(40, "$ gi\x1b[90mt status\x1b[0m\x1b[8D");
        let (before, free) = line_before_cursor(&term);
        assert_eq!(before, "$ gi");
        assert!(!free);
    }

    #[test]
    fn unechoed_input_leaves_the_prompt_as_the_last_text() {
        let term = screen(40, "[sudo] password for fikri: ");
        let (before, _) = line_before_cursor(&term);
        assert!(!before.ends_with("hunter2"));
        assert!(crate::suggest::is_secret_prompt(&before));
    }

    #[test]
    fn moving_the_cursor_left_shortens_the_text_before_it() {
        let term = screen(40, "$ git sta\x1b[3D");
        let (before, _) = line_before_cursor(&term);
        assert!(!before.ends_with("git sta"));
    }
}
