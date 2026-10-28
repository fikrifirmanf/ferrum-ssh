//! UI building blocks plus the chrome (tab bar, sidebar, toasts, palette).

pub mod container_view;
pub mod dialogs;
pub mod editors;
pub mod health_view;
pub mod hosts;
pub mod menus;
pub mod pages;
pub mod settings;
pub mod sftp_view;
pub mod term_view;
pub mod terminal;
pub mod updates;

use egui::{Color32, CornerRadius, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::app::{App, Page, Tab};
use crate::conn::Status;
use crate::themes::Palette;

/// A single bordered input with a trailing visibility button. Visibility is UI
/// state only; passwords remain masked on launch and keep their editing ID.
pub fn password_input(
    ui: &mut egui::Ui,
    value: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    let id = ui.next_auto_id();
    let mut visible = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let width = width.min(ui.available_width());
    let stroke = ui.visuals().widgets.inactive.bg_stroke;
    let fill = ui.visuals().extreme_bg_color;
    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let response = ui.add(
                    egui::TextEdit::singleline(value)
                        .id(id.with("text"))
                        .password(!visible)
                        .hint_text(hint)
                        .frame(egui::Frame::NONE)
                        .desired_width((width - 56.0).max(40.0)),
                );
                let label = if visible {
                    "Hide password"
                } else {
                    "Show password"
                };
                let button = ui
                    .add(ghost_button(if visible {
                        crate::icons::EYE_SLASH
                    } else {
                        crate::icons::EYE
                    }))
                    .on_hover_text(label);
                button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
                if button.clicked() {
                    visible = !visible;
                    ui.data_mut(|d| d.insert_temp(id, visible));
                    response.request_focus();
                }
                response
            })
            .inner
        });
    if frame.inner.has_focus() {
        ui.painter().rect_stroke(
            frame.response.rect,
            10,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    frame.inner
}

/// Workbench chrome uses opaque surfaces on every platform.
pub fn chrome_fill(color: Color32) -> Color32 {
    color
}

pub fn command_shortcut(key: &str) -> String {
    format!(
        "{}{key}",
        if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl+"
        }
    )
}

pub fn primary_button(text: &str, pal: Palette) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_string())
            .color(pal.on_accent)
            .strong(),
    )
    .fill(pal.accent)
    .corner_radius(CornerRadius::same(crate::themes::tokens::RADIUS))
    .min_size(Vec2::new(88.0, 36.0))
}

pub fn ghost_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(text.to_string())
        .frame(false)
        .min_size(Vec2::new(28.0, 28.0))
}

pub fn danger_button(text: &str, pal: Palette) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(pal.danger))
        .stroke(Stroke::new(1.0, pal.danger.gamma_multiply(0.5)))
}

/// Rounded square avatar with initials, colored by a stable hash or OS color.
pub fn avatar(ui: &mut egui::Ui, text: &str, color: Color32, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same((size * 0.28) as u8),
        color.gamma_multiply(0.22),
    );
    p.rect_stroke(
        rect,
        CornerRadius::same((size * 0.28) as u8),
        Stroke::new(1.0, color.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    let initials: String = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase();
    let initials = if initials.is_empty() {
        "•".into()
    } else {
        initials
    };
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials,
        egui::FontId::proportional(size * 0.38),
        avatar_text_color(ui.visuals().dark_mode, color),
    );
    resp
}

/// The tint is a low-alpha fill of `color`, so its own hue is too dim to read as text.
fn avatar_text_color(dark_mode: bool, color: Color32) -> Color32 {
    if dark_mode {
        color.lerp_to_gamma(Color32::WHITE, 0.5)
    } else {
        color.lerp_to_gamma(Color32::BLACK, 0.55)
    }
}

pub fn os_avatar(ui: &mut egui::Ui, symbol: &str, color: Color32, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let pal = ui.visuals();
    let ink = avatar_text_color(pal.dark_mode, color);
    ui.painter()
        .rect_filled(rect, CornerRadius::same(12), ink.gamma_multiply(0.12));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        symbol,
        egui::FontId::proportional(size * 0.62),
        ink,
    );
    response
}

/// Ferrum's hexagon mark, painted so it doesn't depend on font coverage.
pub fn logo_mark(ui: &mut egui::Ui, size: f32, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let c = rect.center();
    let r = size * 0.48;
    let pts: Vec<Pos2> = (0..6)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_3 * i as f32 - std::f32::consts::FRAC_PI_2;
            Pos2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    ui.painter().text(
        c,
        egui::Align2::CENTER_CENTER,
        "Fe",
        egui::FontId::proportional(size * 0.42),
        crate::themes::readable_on(color),
    );
    resp
}

pub fn hash_color(s: &str) -> Color32 {
    let palette = [
        Color32::from_rgb(99, 132, 255),
        Color32::from_rgb(76, 211, 138),
        Color32::from_rgb(245, 166, 35),
        Color32::from_rgb(232, 93, 117),
        Color32::from_rgb(160, 120, 255),
        Color32::from_rgb(60, 196, 214),
        Color32::from_rgb(240, 120, 70),
    ];
    let h = s
        .bytes()
        .fold(5381u32, |a, b| a.wrapping_mul(33) ^ b as u32);
    palette[(h as usize) % palette.len()]
}

pub fn pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_string(), egui::FontId::proportional(12.0), color);
    let size = galley.size() + Vec2::new(14.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), color.gamma_multiply(0.15));
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

pub fn section_title(ui: &mut egui::Ui, text: &str, pal: Palette) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(text.to_uppercase())
            .small()
            .strong()
            .color(pal.text_dim),
    );
    ui.add_space(2.0);
}

pub fn card_frame(pal: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.card)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(crate::themes::tokens::RADIUS))
        .inner_margin(egui::Margin::same(16))
}

pub fn status_color(s: &Status, pal: Palette) -> Color32 {
    match s {
        Status::Connected => pal.success,
        Status::Connecting(_) => pal.warning,
        Status::Closed(_) => pal.text_dim,
        Status::Failed(_) => pal.danger,
    }
}

/// Simple fuzzy subsequence score (higher is better), `None` when not matching.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let t = text.to_lowercase();
    let q = query.to_lowercase();
    if let Some(pos) = t.find(&q) {
        return Some(1000 - pos as i32);
    }
    let mut score = 0;
    let mut it = t.chars();
    for qc in q.chars() {
        let mut found = false;
        for tc in it.by_ref() {
            score += 1;
            if tc == qc {
                found = true;
                break;
            }
        }
        if !found {
            return None;
        }
    }
    Some(500 - score)
}

impl App {
    pub fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        egui::Panel::top("titlebar")
            .exact_size(48.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.sidebar)
                    .inner_margin(egui::Margin {
                        left: if cfg!(target_os = "macos") { 96 } else { 16 },
                        right: 16,
                        top: 6,
                        bottom: 6,
                    }),
            )
            .show(ui, |ui| {
                let response = ui.interact(
                    ui.max_rect(),
                    ui.scope_id().with("title-drag"),
                    Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("Ferrum").strong().size(12.0));
                    ui.label(
                        RichText::new("/ Personal vault")
                            .small()
                            .color(pal.text_dim),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let shortcut = if cfg!(target_os = "macos") {
                            "⌘K"
                        } else {
                            "Ctrl+K"
                        };
                        if ui
                            .add(ghost_button(&format!(
                                "{}  Find a command     {shortcut}",
                                crate::icons::MAGNIFYING_GLASS
                            )))
                            .clicked()
                        {
                            self.palette.open = true;
                            self.palette.query.clear();
                        }
                    });
                });
            });
        egui::Panel::top("tabbar")
            .exact_size(40.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.sidebar)
                    .inner_margin(egui::Margin::symmetric(8, 0)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("session-tabs")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            // Home (vault) tab
                            let home_active = self.active_tab.is_none();
                            if tab_chip(ui, "\u{E32E}  Library", None, home_active, pal, false)
                                .0
                                .clicked()
                            {
                                self.active_tab = None;
                            }
                            let mut close = None;
                            let mut activate = None;
                            let mut rename = None;
                            let mut duplicate = None;
                            let mut open_ws: Option<uuid::Uuid> = None;
                            for t in &self.tabs {
                                let (title, status) = match t {
                                    Tab::Terminal(tt) => {
                                        let p = tt.panes.get(tt.active).or(tt.panes.first());
                                        let title = tt
                                            .custom_title
                                            .clone()
                                            .or_else(|| p.map(|p| p.session.title.clone()))
                                            .unwrap_or_default();
                                        let st = p.map(|p| p.session.status.lock().clone());
                                        let title = if tt.panes.len() > 1 {
                                            format!("{title}  ·{}", tt.panes.len())
                                        } else {
                                            title
                                        };
                                        (title, st)
                                    }
                                    Tab::Sftp(s) => (format!("\u{E098}  {}", s.title), None),
                                };
                                let active = self.active_tab == Some(t.id());
                                let short: String = if title.chars().count() > 26 {
                                    title.chars().take(25).collect::<String>() + "…"
                                } else {
                                    title
                                };
                                let (resp, close_clicked) = tab_chip(
                                    ui,
                                    &short,
                                    status.as_ref().map(|s| status_color(s, pal)),
                                    active,
                                    pal,
                                    true,
                                );
                                if resp.clicked() {
                                    activate = Some(t.id());
                                }
                                if resp.middle_clicked() || close_clicked {
                                    close = Some(t.id());
                                }
                                if resp.double_clicked() {
                                    rename = Some((t.id(), short.clone()));
                                }
                                resp.context_menu(|ui| {
                                    if ui.button("Rename tab").clicked() {
                                        rename = Some((t.id(), short.clone()));
                                        ui.close();
                                    }
                                    if ui.button("Duplicate tab").clicked() {
                                        duplicate = Some(t.id());
                                        ui.close();
                                    }
                                    if ui.button("Close tab").clicked() {
                                        close = Some(t.id());
                                        ui.close();
                                    }
                                });
                            }
                            if let Some(id) = activate {
                                self.active_tab = Some(id);
                            }
                            if let Some(id) = duplicate {
                                self.duplicate_tab(id);
                            }
                            if let Some(id) = close {
                                self.close_tab(id);
                            }
                            if let Some((tab, title)) = rename {
                                self.dialog = Some(crate::app::Dialog::RenameTab { tab, title });
                            }
                            ui.menu_button(RichText::new("+").size(16.0), |ui| {
                                ui.set_min_width(200.0);
                                if ui
                                    .button(format!(
                                        "{}  Local terminal     {}",
                                        crate::icons::KEYBOARD,
                                        command_shortcut("T")
                                    ))
                                    .clicked()
                                {
                                    self.open_local();
                                    ui.close();
                                }
                                if ui.button("\u{E098}  SFTP browser").clicked() {
                                    self.open_sftp(None);
                                    ui.close();
                                }
                                if ui.button("\u{EB56}  Serial connection").clicked() {
                                    self.dialog = Some(crate::app::Dialog::Serial {
                                        cfg: crate::model::SerialConfig::default(),
                                        ports: crate::conn::serial::list_ports(),
                                    });
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .add_enabled(
                                        !self.tabs.is_empty(),
                                        egui::Button::new("\u{E464}  Save open tabs as workspace…"),
                                    )
                                    .clicked()
                                {
                                    self.dialog = Some(crate::app::Dialog::SaveWorkspace {
                                        name: String::new(),
                                    });
                                    ui.close();
                                }
                                let ws: Vec<(uuid::Uuid, String)> = self
                                    .vault
                                    .workspaces
                                    .iter()
                                    .map(|w| (w.id, w.name.clone()))
                                    .collect();
                                if !ws.is_empty() {
                                    ui.menu_button("\u{E464}  Open workspace", |ui| {
                                        for (id, name) in ws {
                                            if ui.button(name).clicked() {
                                                open_ws = Some(id);
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                                if ui
                                    .button(format!(
                                        "{}  Command palette   {}",
                                        crate::icons::MAGNIFYING_GLASS,
                                        command_shortcut("K")
                                    ))
                                    .clicked()
                                {
                                    self.palette.open = true;
                                    ui.close();
                                }
                            });
                            if let Some(id) = open_ws {
                                self.open_workspace(id);
                            }
                        });
                    });
            });
    }

    pub fn app_navigation(&mut self, ui: &mut egui::Ui) {
        use crate::themes::tokens;
        let pal = self.pal();
        let narrow = ui.ctx().content_rect().width() < 1080.0;
        let expand_key = egui::Id::unique("narrow-navigation-expanded");
        let expanded = ui.data(|data| data.get_temp::<bool>(expand_key).unwrap_or(false));
        let collapsed = if narrow {
            !expanded
        } else {
            self.vault.settings.sidebar_collapsed
        };
        egui::Panel::left("nav")
            .exact_size(if collapsed {
                tokens::RAIL
            } else {
                tokens::SIDEBAR
            })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(pal.sidebar)
                    .inner_margin(egui::Margin::symmetric(12, 24)),
            )
            .show(ui, |ui| {
                let short_nav = ui.available_height() < 320.0;
                if !short_nav {
                    ui.horizontal(|ui| {
                        logo_mark(ui, 24.0, pal.accent);
                        if !collapsed {
                            ui.label(RichText::new("Ferrum").size(16.0).strong());
                        }
                    });
                    ui.add_space(24.0);
                }
                egui::ScrollArea::vertical()
                    .id_salt("navigation-items")
                    .max_height(
                        (ui.available_height() - if short_nav { 96.0 } else { 120.0 }).max(0.0),
                    )
                    .min_scrolled_height(0.0)
                    .show(ui, |ui| {
                        for (section, items) in [
                            (
                                "CONNECT",
                                vec![
                                    (Page::Hosts, crate::icons::MONITOR, "Hosts"),
                                    (Page::Workspaces, crate::icons::SQUARES_FOUR, "Workspaces"),
                                ],
                            ),
                            (
                                "TOOLS",
                                vec![
                                    (Page::Keychain, crate::icons::KEY, "Keychain"),
                                    (
                                        Page::Forwarding,
                                        crate::icons::ARROWS_LEFT_RIGHT,
                                        "Port forwarding",
                                    ),
                                    (Page::Snippets, crate::icons::SCISSORS, "Snippets"),
                                ],
                            ),
                            (
                                "RECORDS",
                                vec![
                                    (Page::KnownHosts, crate::icons::SHIELD_CHECK, "Known hosts"),
                                    (
                                        Page::History,
                                        crate::icons::CLOCK_COUNTER_CLOCKWISE,
                                        "History and logs",
                                    ),
                                ],
                            ),
                        ] {
                            if !collapsed {
                                ui.label(RichText::new(section).size(12.0).color(pal.text_dim));
                            }
                            for (page, icon, label) in items {
                                if nav_item(
                                    ui,
                                    icon,
                                    label,
                                    self.active_tab.is_none() && self.page == page,
                                    collapsed,
                                    pal,
                                )
                                .clicked()
                                {
                                    self.active_tab = None;
                                    self.page = page;
                                    self.editor = None;
                                    self.host_inspector_open = false;
                                }
                            }
                            ui.add_space(16.0);
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if !short_nav
                        && nav_item(
                            ui,
                            if collapsed { "\u{E13A}" } else { "\u{E058}" },
                            if collapsed {
                                "Expand navigation"
                            } else {
                                "Collapse navigation"
                            },
                            false,
                            collapsed,
                            pal,
                        )
                        .clicked()
                    {
                        if narrow {
                            ui.data_mut(|data| data.insert_temp(expand_key, !expanded));
                        } else {
                            self.vault.settings.sidebar_collapsed =
                                !self.vault.settings.sidebar_collapsed;
                            crate::store::touch_settings(&mut self.vault);
                            self.mark_dirty();
                        }
                    }
                    if self.store.master_password.is_some()
                        && nav_item(
                            ui,
                            crate::icons::LOCK_SIMPLE,
                            "Lock vault",
                            false,
                            collapsed,
                            pal,
                        )
                        .clicked()
                    {
                        self.locked = true;
                    }
                    if nav_item(
                        ui,
                        crate::icons::GEAR,
                        "Settings",
                        self.active_tab.is_none() && self.page == Page::Settings,
                        collapsed,
                        pal,
                    )
                    .clicked()
                    {
                        self.active_tab = None;
                        self.page = Page::Settings;
                        self.editor = None;
                    }
                });
            });
    }

    pub fn vault_home(&mut self, ui: &mut egui::Ui) {
        use crate::themes::tokens;
        let pal = self.pal();
        if self.page == Page::Hosts && self.editor.is_none() {
            let visible = self.filtered_hosts();
            if !visible.iter().any(|h| Some(h.id) == self.inspected_host) {
                self.inspected_host = visible.first().map(|h| h.id);
            }
            if self.inspected_host.is_some() && ui.ctx().content_rect().width() >= 1200.0 {
                egui::Panel::right("host-inspector")
                    .default_size(tokens::INSPECTOR)
                    .size_range(288.0..=360.0)
                    .resizable(true)
                    .frame(
                        egui::Frame::new()
                            .fill(pal.panel)
                            .inner_margin(egui::Margin::same(tokens::INSET))
                            .stroke(Stroke::new(1.0, pal.border)),
                    )
                    .show(ui, |ui| {
                        self.host_inspector(ui);
                    });
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::same(if ui.ctx().content_rect().height()<640.0{tokens::INSET}else{tokens::PAGE})),
            )
            .show(ui, |ui| {
                if self.editor.is_some() {
                    if ui.available_width()>900.0 {
                        ui.horizontal_top(|ui|{
                            let width=(ui.available_width()-272.0).min(720.0);
                            ui.allocate_ui_with_layout(Vec2::new(width,ui.available_height()),egui::Layout::top_down(egui::Align::Min),|ui|self.editor_panel(ui));
                            ui.add_space(24.0);
                            ui.vertical(|ui|{
                                ui.set_max_width(224.0);ui.add_space(56.0);
                                ui.label(RichText::new("Keep the target in view").strong());
                                ui.label(RichText::new("Review the address and group before saving or connecting.").small().color(pal.text_dim));
                                ui.add_space(24.0);ui.separator();ui.add_space(16.0);
                                ui.label(RichText::new("Save or connect").strong());
                                ui.label(RichText::new("Save changes returns to the library. Save and connect opens the host in a session tab.").small().color(pal.text_dim));
                            });
                        });
                    }else{self.editor_panel(ui);}
                    return;
                }
                match self.page {
                    Page::Hosts => self.hosts_page(ui),
                    Page::Keychain => self.keychain_page(ui),
                    Page::Forwarding => self.forwarding_page(ui),
                    Page::Snippets => self.snippets_page(ui),
                    Page::Workspaces => self.workspaces_page(ui),
                    Page::KnownHosts => self.known_hosts_page(ui),
                    Page::History => self.history_page(ui),
                    Page::Settings => self.settings_page(ui),
                }
            });
        if self.page == Page::Hosts
            && self.editor.is_none()
            && self.host_inspector_open
            && self.inspected_host.is_some()
            && ui.ctx().content_rect().width() < 1200.0
        {
            let ctx = ui.ctx().clone();
            let response = egui::Modal::new(egui::Id::unique("host-inspector-overlay"))
                .frame(
                    egui::Frame::new()
                        .fill(pal.panel)
                        .stroke(Stroke::new(1.0, pal.border))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::same(16)),
                )
                .show(&ctx, |ui| {
                    ui.set_width(tokens::INSPECTOR);
                    ui.set_height((ctx.content_rect().height() - 160.0).clamp(320.0, 640.0));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Selected host").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(ghost_button("×"))
                                .on_hover_text("Close host details")
                                .clicked()
                            {
                                self.host_inspector_open = false;
                            }
                        });
                    });
                    self.host_inspector(ui);
                });
            if response.should_close() {
                self.host_inspector_open = false;
            }
        }
    }

    pub fn draw_toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| now - t.at < 5.0);
        if self.toasts.is_empty() {
            return;
        }
        let pal = self.pal();
        let screen = ctx.content_rect();
        let mut y = screen.bottom() - 40.0;
        let mut dismiss = None;
        for (i, t) in self.toasts.iter().enumerate().rev().take(3) {
            let id = egui::Id::unique(("toast", i, (t.at * 1000.0) as i64));
            let area = egui::Area::new(id)
                .order(egui::Order::Tooltip)
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .fixed_pos(Pos2::new(screen.right() - 16.0, y))
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(pal.panel)
                        .stroke(Stroke::new(
                            1.0,
                            if t.error { pal.danger } else { pal.border },
                        ))
                        .corner_radius(CornerRadius::same(crate::themes::tokens::RADIUS))
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 4],
                            blur: 16,
                            spread: 0,
                            color: Color32::from_black_alpha(90),
                        })
                        .show(ui, |ui| {
                            let width = (screen.width() - 64.0).clamp(200.0, 420.0);
                            ui.set_width(width);
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    RichText::new(if t.error { "\u{E4E0}" } else { "\u{E182}" })
                                        .color(if t.error { pal.danger } else { pal.success }),
                                );
                                ui.allocate_ui_with_layout(
                                    Vec2::new((width - 80.0).max(100.0), 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.add(egui::Label::new(&t.text).wrap());
                                    },
                                );
                                if ui
                                    .add(ghost_button("×"))
                                    .on_hover_text("Dismiss notification")
                                    .clicked()
                                {
                                    dismiss = Some(i);
                                }
                            });
                        });
                });
            y -= area.response.rect.height() + 8.0;
        }
        if let Some(i) = dismiss {
            self.toasts.remove(i);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }

    pub fn draw_palette(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        let pal = self.pal();
        #[derive(Clone)]
        enum Action {
            Host(uuid::Uuid),
            Sftp(uuid::Uuid),
            Snippet(uuid::Uuid),
            Workspace(uuid::Uuid),
            Local,
            NewHost,
            QuickConnect,
            Page(Page),
            Quick(String),
            Tab(u64),
            SplitH,
            SplitV,
            Lock,
        }
        let q = self.palette.query.clone();
        let mut items: Vec<(i32, String, String, Action)> = vec![];
        let mut push = |label: String, detail: String, a: Action| {
            if let Some(score) = fuzzy(&q, &format!("{label} {detail}")) {
                items.push((score, label, detail, a));
            }
        };
        for h in &self.vault.hosts {
            push(
                format!("Connect  {}", h.display_name()),
                h.address.clone(),
                Action::Host(h.id),
            );
            if h.protocol == crate::model::Protocol::Ssh {
                push(
                    format!("SFTP  {}", h.display_name()),
                    h.address.clone(),
                    Action::Sftp(h.id),
                );
            }
        }
        for s in &self.vault.snippets {
            push(
                format!("Run snippet  {}", s.name),
                s.script.lines().next().unwrap_or("").to_string(),
                Action::Snippet(s.id),
            );
        }
        for w in &self.vault.workspaces {
            push(
                format!("Open workspace  {}", w.name),
                format!("{} tabs", w.tabs.len()),
                Action::Workspace(w.id),
            );
        }
        for t in &self.tabs {
            if let Tab::Terminal(tt) = t {
                if let Some(p) = tt.panes.first() {
                    push(
                        format!("Go to tab  {}", p.session.title),
                        String::new(),
                        Action::Tab(tt.id),
                    );
                }
            }
        }
        push(
            "New local terminal".into(),
            command_shortcut("T"),
            Action::Local,
        );
        push("New host".into(), String::new(), Action::NewHost);
        push(
            "Quick connect".into(),
            "user@host:port".into(),
            Action::QuickConnect,
        );
        push(
            "Split pane right".into(),
            command_shortcut("D"),
            Action::SplitH,
        );
        push(
            "Split pane down".into(),
            format!("Shift+{}", command_shortcut("D")),
            Action::SplitV,
        );
        for (p, n) in [
            (Page::Hosts, "Hosts"),
            (Page::Keychain, "Keychain"),
            (Page::Forwarding, "Port forwarding"),
            (Page::Snippets, "Snippets"),
            (Page::Workspaces, "Workspaces"),
            (Page::KnownHosts, "Known hosts"),
            (Page::History, "History & logs"),
            (Page::Settings, "Settings"),
        ] {
            push(format!("Open {n}"), String::new(), Action::Page(p));
        }
        if self.store.master_password.is_some() {
            push("Lock vault".into(), String::new(), Action::Lock);
        }
        if crate::model::parse_quick_connect(&q).is_some()
            && q.contains(|c: char| c == '@' || c == '.')
        {
            items.insert(
                0,
                (
                    i32::MAX,
                    format!("Quick connect  {q}"),
                    "SSH".into(),
                    Action::Quick(q.clone()),
                ),
            );
        }
        items.sort_by(|a, b| b.0.cmp(&a.0));
        items.truncate(12);
        let mut chosen: Option<Action> = None;
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("palette"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(screen.center().x, screen.top() + 90.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.panel)
                    .stroke(Stroke::new(1.0, pal.border))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 12],
                        blur: 40,
                        spread: 0,
                        color: Color32::from_black_alpha(120),
                    })
                    .show(ui, |ui| {
                        ui.set_width((screen.width() - 64.0).clamp(280.0, 560.0));
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.palette.query)
                                .hint_text("Search hosts, snippets, actions — or type user@host")
                                .font(egui::FontId::proportional(16.0))
                                .desired_width(f32::INFINITY)
                                .frame(egui::Frame::NONE),
                        );
                        te.request_focus();
                        if te.changed() {
                            self.palette.selected = 0;
                        }
                        ui.separator();
                        let n = items.len();
                        if ui.input(|i| i.key_pressed(Key::ArrowDown)) && n > 0 {
                            self.palette.selected = (self.palette.selected + 1) % n;
                        }
                        if ui.input(|i| i.key_pressed(Key::ArrowUp)) && n > 0 {
                            self.palette.selected = (self.palette.selected + n - 1) % n;
                        }
                        for (i, (_, label, detail, action)) in items.iter().enumerate() {
                            let sel = i == self.palette.selected;
                            let (rect, resp) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 34.0),
                                Sense::click(),
                            );
                            if sel || resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    CornerRadius::same(10),
                                    pal.accent_soft,
                                );
                            }
                            ui.painter().text(
                                rect.left_center() + Vec2::new(10.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                label,
                                egui::FontId::proportional(14.0),
                                pal.text,
                            );
                            ui.painter().text(
                                rect.right_center() - Vec2::new(10.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                detail,
                                egui::FontId::proportional(12.0),
                                pal.text_dim,
                            );
                            if resp.clicked() {
                                chosen = Some(action.clone());
                            }
                        }
                        if items.is_empty() {
                            ui.label(RichText::new("No results").color(pal.text_dim));
                        }
                        if ui.input(|i| i.key_pressed(Key::Enter)) {
                            chosen = items.get(self.palette.selected).map(|x| x.3.clone());
                        }
                    });
            });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.palette.open = false;
        }
        if let Some(a) = chosen {
            self.palette.open = false;
            match a {
                Action::Host(id) => self.open_host(id),
                Action::Sftp(id) => self.open_sftp(Some(id)),
                Action::Snippet(id) => {
                    if let Some(s) = self.vault.snippet(id).cloned() {
                        self.begin_run_snippet(s);
                    }
                }
                Action::Workspace(id) => self.open_workspace(id),
                Action::Local => self.open_local(),
                Action::QuickConnect => {
                    self.dialog = Some(crate::app::Dialog::QuickConnect {
                        input: String::new(),
                        error: None,
                    })
                }
                Action::NewHost => {
                    self.active_tab = None;
                    self.page = Page::Hosts;
                    self.editor = Some(crate::app::Editor::Host(Box::default(), true));
                }
                Action::Page(p) => {
                    self.active_tab = None;
                    self.page = p;
                }
                Action::Quick(q) => self.open_quick_connect(&q),
                Action::Tab(id) => self.active_tab = Some(id),
                Action::SplitH => self.split_active(crate::app::SplitLayout::Horizontal),
                Action::SplitV => self.split_active(crate::app::SplitLayout::Vertical),
                Action::Lock => self.locked = true,
            }
        }
    }
}

fn tab_chip(
    ui: &mut egui::Ui,
    title: &str,
    dot: Option<Color32>,
    active: bool,
    pal: Palette,
    closable: bool,
) -> (egui::Response, bool) {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(
        title.to_string(),
        font,
        if active { pal.text } else { pal.text_dim },
    );
    let extra = if dot.is_some() { 16.0 } else { 0.0 } + if closable { 24.0 } else { 0.0 };
    let size = Vec2::new((galley.size().x + 24.0 + extra).max(88.0), 40.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let p = ui.painter();
    if active || resp.hovered() {
        p.rect_filled(rect, 0, if active { pal.bg } else { pal.card_hover });
    }
    p.vline(rect.right(), rect.y_range(), Stroke::new(1.0, pal.border));
    if active {
        p.hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            Stroke::new(2.0, pal.accent),
        );
    }
    if resp.has_focus() {
        p.rect_stroke(
            rect.shrink(2.0),
            4,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, title));
    let mut x = rect.left() + 13.0;
    if let Some(c) = dot {
        p.circle_filled(Pos2::new(x + 3.0, rect.center().y), 4.0, c);
        x += 16.0;
    }
    p.galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        pal.text,
    );
    let mut close = false;
    if closable && (resp.hovered() || active) {
        let cr = Rect::from_center_size(
            Pos2::new(rect.right() - 14.0, rect.center().y),
            Vec2::splat(18.0),
        );
        let cresp = ui.interact(cr, resp.id.with("close"), Sense::click());
        if cresp.hovered() {
            ui.painter()
                .rect_filled(cr, CornerRadius::same(5), pal.card_hover);
        }
        ui.painter().text(
            cr.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(12.0),
            pal.text_dim,
        );
        close = cresp.clicked();
    }
    (resp, close)
}

fn nav_item(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    active: bool,
    collapsed: bool,
    pal: Palette,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, active, label));
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(9), pal.accent_soft);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(9), pal.card_hover);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            8,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    let color = if active { pal.accent } else { pal.text_dim };
    p.text(
        rect.left_center() + Vec2::new(12.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        color,
    );
    if !collapsed {
        p.text(
            rect.left_center() + Vec2::new(40.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.0),
            if active { pal.text } else { pal.text_dim },
        );
    }
    if collapsed {
        resp.on_hover_text(label)
    } else {
        resp
    }
}
