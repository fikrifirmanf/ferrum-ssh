//! UI building blocks plus the chrome (tab bar, sidebar, toasts, palette).

pub mod container_view;
pub mod dialogs;
pub mod editors;
pub mod forms;
pub mod health_view;
pub mod hosts;
pub mod menus;
pub mod motion;
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

/// A password box with a visibility button. Visibility is UI state only; passwords stay
/// masked on launch.
pub fn password_input(
    ui: &mut egui::Ui,
    value: &mut String,
    hint: &str,
    width: f32,
) -> egui::Response {
    let salt = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    ui.push_id(salt, |ui| {
        forms::Input::bare("Password")
            .hint(hint)
            .password()
            .width(width.min(ui.available_width()))
            .show(ui, value)
    })
    .inner
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

pub fn primary_button(text: &str, _pal: Palette) -> forms::Btn {
    forms::Btn::primary(text)
}

pub fn ghost_button(text: &str) -> forms::Btn {
    forms::Btn::ghost(text)
}

pub fn danger_button(text: &str, _pal: Palette) -> forms::Btn {
    forms::Btn::danger(text)
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

/// Prompt F, painted so it stays crisp at any scale: the name (F) and the job (`>_`).
/// Below 32 pt it uses the simplified small-size drawing with square cuts.
pub fn logo_mark(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let pal = crate::themes::current(ui.ctx());
    let polygons = logo_polygons(size);
    for (hot, points) in polygons {
        let color = if hot { pal.accent } else { pal.text };
        let at = |(x, y): &(f32, f32)| rect.min + Vec2::new(*x, *y) * (size / 1024.0);
        ui.painter().add(egui::Shape::convex_polygon(
            points.iter().map(at).collect(),
            color,
            Stroke::NONE,
        ));
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Ferrum"));
    resp
}

/// The mark as convex parts on a 1024 grid; `true` marks the ember prompt.
fn logo_polygons(size: f32) -> Vec<(bool, Vec<(f32, f32)>)> {
    if size < 32.0 {
        vec![
            (
                false,
                vec![(60.0, 90.0), (964.0, 90.0), (964.0, 300.0), (60.0, 300.0)],
            ),
            (
                false,
                vec![(60.0, 300.0), (283.0, 300.0), (283.0, 934.0), (60.0, 934.0)],
            ),
            (
                true,
                vec![
                    (374.0, 400.0),
                    (531.0, 400.0),
                    (728.0, 591.0),
                    (571.0, 591.0),
                ],
            ),
            (
                true,
                vec![
                    (571.0, 591.0),
                    (728.0, 591.0),
                    (531.0, 782.0),
                    (374.0, 782.0),
                ],
            ),
            (
                true,
                vec![
                    (636.0, 811.0),
                    (964.0, 811.0),
                    (964.0, 934.0),
                    (636.0, 934.0),
                ],
            ),
        ]
    } else {
        vec![
            (
                false,
                vec![
                    (204.0, 110.0),
                    (900.0, 110.0),
                    (836.0, 293.0),
                    (110.0, 293.0),
                    (110.0, 199.0),
                ],
            ),
            (
                false,
                vec![
                    (110.0, 293.0),
                    (299.0, 293.0),
                    (299.0, 914.0),
                    (110.0, 914.0),
                ],
            ),
            (
                true,
                vec![
                    (390.0, 400.0),
                    (512.0, 400.0),
                    (676.0, 555.0),
                    (554.0, 555.0),
                ],
            ),
            (
                true,
                vec![
                    (554.0, 555.0),
                    (676.0, 555.0),
                    (512.0, 710.0),
                    (390.0, 710.0),
                ],
            ),
            (
                true,
                vec![
                    (628.0, 820.0),
                    (900.0, 820.0),
                    (900.0, 914.0),
                    (628.0, 914.0),
                ],
            ),
        ]
    }
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

pub fn section_title(ui: &mut egui::Ui, text: &str, pal: Palette) {
    ui.add_space(6.0);
    forms::label(ui, text, pal);
    ui.add_space(2.0);
}

pub fn card_frame(pal: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.card)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(crate::themes::tokens::RADIUS_LARGE))
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
        use crate::themes::tokens;
        let pal = self.pal();
        egui::Panel::top("topbar")
            .exact_size(tokens::BAR)
            .frame(
                egui::Frame::new()
                    .fill(pal.sidebar)
                    .inner_margin(egui::Margin {
                        left: if cfg!(target_os = "macos") { 88 } else { 12 },
                        right: 12,
                        top: 0,
                        bottom: 0,
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
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let wide = ui.available_width() > 860.0;
                    logo_mark(ui, 20.0);
                    ui.label(
                        RichText::new("Ferrum")
                            .size(14.0)
                            .family(crate::fonts::semibold()),
                    );
                    if wide {
                        ui.label(RichText::new("Personal vault").size(12.0).color(pal.faint));
                    }
                    ui.add_space(8.0);
                    let cmdk_width = if wide { 184.0 } else { 36.0 };
                    let tabs_width = (ui.available_width() - cmdk_width - 12.0).max(120.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(tabs_width, tokens::BAR),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| self.tab_strip(ui, pal),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if command_button(ui, wide, pal).clicked() {
                            self.palette.open = true;
                            self.palette.query.clear();
                            self.palette.selected = 0;
                        }
                    });
                });
            });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui, pal: Palette) {
        egui::ScrollArea::horizontal()
            .id_salt("session-tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let home_active = self.active_tab.is_none();
                    if tab_chip(ui, "Library", None, home_active, pal, false)
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
                            Tab::Sftp(s) => (
                                format!("{}  {}", crate::icons::ARROWS_DOWN_UP, s.title),
                                None,
                            ),
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
                    ui.add_space(4.0);
                    let new_tab = egui::Button::new(
                        RichText::new(crate::icons::PLUS)
                            .size(16.0)
                            .color(pal.text_dim),
                    )
                    .frame(false)
                    .min_size(Vec2::splat(32.0));
                    egui::containers::menu::MenuButton::from_button(new_tab)
                        .ui(ui, |ui| {
                            ui.set_min_width(240.0);
                            if ui
                                .button(format!(
                                    "{}  Local terminal     {}",
                                    crate::icons::TERMINAL_WINDOW,
                                    command_shortcut("T")
                                ))
                                .clicked()
                            {
                                self.open_local();
                                ui.close();
                            }
                            if ui
                                .button(format!("{}  SFTP browser", crate::icons::ARROWS_DOWN_UP))
                                .clicked()
                            {
                                self.open_sftp(None);
                                ui.close();
                            }
                            if ui
                                .button(format!("{}  Serial connection", crate::icons::PLUGS))
                                .clicked()
                            {
                                self.dialog = Some(crate::app::Dialog::Serial {
                                    cfg: crate::model::SerialConfig::default(),
                                });
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(
                                    !self.tabs.is_empty(),
                                    egui::Button::new(format!(
                                        "{}  Save open tabs as workspace…",
                                        crate::icons::SQUARES_FOUR
                                    )),
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
                                ui.menu_button(
                                    format!("{}  Open workspace", crate::icons::SQUARES_FOUR),
                                    |ui| {
                                        for (id, name) in ws {
                                            if ui.button(name).clicked() {
                                                open_ws = Some(id);
                                                ui.close();
                                            }
                                        }
                                    },
                                );
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
                        })
                        .0
                        .on_hover_text("New tab");
                    if let Some(id) = open_ws {
                        self.open_workspace(id);
                    }
                });
            });
    }

    pub fn app_navigation(&mut self, ui: &mut egui::Ui) {
        use crate::themes::tokens;
        let pal = self.pal();
        let narrow = ui.ctx().content_rect().width() < 1080.0;
        let expand_key = egui::Id::unique("narrow-navigation-expanded");
        let expanded = ui.data(|data| data.get_temp::<bool>(expand_key).unwrap_or(false));
        // The editor takes the rail too, so a form gets the room while every page stays reachable.
        let editing = self.editor.is_some() && self.active_tab.is_none();
        let collapsed = if narrow {
            !expanded
        } else {
            self.vault.settings.sidebar_collapsed || editing
        };
        let open = motion::ease_panel(ui.ctx(), egui::Id::unique("nav-width"), !collapsed);
        let show_labels = open > 0.5;
        egui::Panel::left("nav")
            .exact_size(tokens::RAIL + (tokens::SIDEBAR - tokens::RAIL) * open)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(pal.sidebar)
                    .inner_margin(egui::Margin::symmetric(8, 12)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let short_nav = ui.available_height() < 320.0;
                egui::ScrollArea::vertical()
                    .id_salt("navigation-items")
                    .max_height(
                        (ui.available_height() - if short_nav { 76.0 } else { 116.0 }).max(0.0),
                    )
                    .min_scrolled_height(0.0)
                    .show(ui, |ui| {
                        for (section, items) in [
                            (
                                "Connect",
                                vec![
                                    (Page::Hosts, crate::icons::HARD_DRIVES, "Hosts"),
                                    (Page::Workspaces, crate::icons::SQUARES_FOUR, "Workspaces"),
                                ],
                            ),
                            (
                                "Tools",
                                vec![
                                    (Page::Keychain, crate::icons::KEY, "Keychain"),
                                    (
                                        Page::Forwarding,
                                        crate::icons::ARROWS_LEFT_RIGHT,
                                        "Port forwarding",
                                    ),
                                    (Page::Snippets, crate::icons::TERMINAL, "Snippets"),
                                ],
                            ),
                            (
                                "Records",
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
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 26.0),
                                Sense::hover(),
                            );
                            ui.painter().with_clip_rect(rect).text(
                                rect.left_bottom() + Vec2::new(10.0, -4.0),
                                egui::Align2::LEFT_BOTTOM,
                                section,
                                egui::FontId::new(12.0, crate::fonts::medium()),
                                pal.faint.gamma_multiply(open),
                            );
                            for (page, icon, label) in items {
                                if nav_item(
                                    ui,
                                    icon,
                                    label,
                                    self.active_tab.is_none() && self.page == page,
                                    open,
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
                            ui.add_space(8.0);
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if !short_nav
                        && nav_item(
                            ui,
                            if show_labels {
                                crate::icons::ARROW_LEFT
                            } else {
                                crate::icons::CARET_RIGHT
                            },
                            if show_labels {
                                "Collapse navigation"
                            } else {
                                "Expand navigation"
                            },
                            false,
                            open,
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
                            open,
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
                        open,
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
        }
        let docked = self.page == Page::Hosts
            && self.editor.is_none()
            && self.inspected_host.is_some()
            && ui.ctx().content_rect().width() >= 1200.0;
        egui::Panel::right("host-inspector")
            .default_size(tokens::INSPECTOR)
            .size_range(288.0..=360.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(pal.sidebar)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show_collapsible(ui, &mut { docked }, |ui| {
                self.host_inspector(ui);
            });
        let page_key = (self.page, self.editor.is_some());
        let enter = motion::since_change(
            ui.ctx(),
            egui::Id::unique("page-enter"),
            page_key,
            motion::PANEL,
        );
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(
                        tokens::PAGE,
                        if ui.ctx().content_rect().height() < 640.0 {
                            12
                        } else {
                            20
                        },
                    )),
            )
            .show(ui, |ui| {
                ui.set_opacity(enter);
                ui.add_space((1.0 - enter) * 6.0);
                if self.editor.is_some() {
                    let column = ui.available_width().min(760.0);
                    let inset = ((ui.available_width() - column) / 2.0).max(0.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(inset);
                        ui.allocate_ui_with_layout(
                            Vec2::new(column, ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(column);
                                self.editor_panel(ui);
                            },
                        );
                    });
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
                .backdrop_color(pal.scrim)
                .frame(
                    egui::Frame::new()
                        .fill(pal.sidebar)
                        .stroke(Stroke::new(1.0, pal.control_border))
                        .corner_radius(tokens::RADIUS_LARGE)
                        .inner_margin(egui::Margin::same(20)),
                )
                .show(&ctx, |ui| {
                    ui.set_width(tokens::INSPECTOR);
                    ui.set_height((ctx.content_rect().height() - 160.0).clamp(320.0, 640.0));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Selected host").family(crate::fonts::semibold()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(forms::Btn::icon(crate::icons::X, "Close host details"))
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
        const LIFE: f64 = 5.0;
        const IN: f32 = 0.2;
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|t| now - t.at < LIFE);
        if self.toasts.is_empty() {
            return;
        }
        let pal = self.pal();
        let screen = ctx.content_rect();
        let mut y = screen.bottom() - crate::themes::tokens::STATUS - 16.0;
        let mut dismiss = None;
        let mut moving = false;
        for (i, t) in self.toasts.iter().enumerate().rev().take(3) {
            let age = (now - t.at) as f32;
            let left = (LIFE - now + t.at) as f32;
            let (appear, leave) = if motion::reduced(ctx) {
                (1.0, 1.0)
            } else {
                (
                    emath::easing::cubic_out((age / IN).clamp(0.0, 1.0)),
                    (left / motion::EXIT).clamp(0.0, 1.0),
                )
            };
            moving |= appear < 1.0 || leave < 1.0;
            let id = egui::Id::unique(("toast", (t.at * 1000.0) as i64, i));
            let area = egui::Area::new(id)
                .order(egui::Order::Tooltip)
                .pivot(egui::Align2::CENTER_BOTTOM)
                .fixed_pos(Pos2::new(screen.center().x, y + (1.0 - appear) * 8.0))
                .fade_in(false)
                .show(ctx, |ui| {
                    ui.set_opacity(appear * leave);
                    egui::Frame::new()
                        .fill(pal.card_hover)
                        .stroke(Stroke::new(
                            1.0,
                            if t.error {
                                pal.danger
                            } else {
                                pal.control_border
                            },
                        ))
                        .corner_radius(CornerRadius::same(crate::themes::tokens::RADIUS))
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .shadow(ui.visuals().popup_shadow)
                        .show(ui, |ui| {
                            let width = (screen.width() - 64.0).clamp(160.0, 480.0);
                            ui.set_max_width(width);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if t.error {
                                        crate::icons::WARNING
                                    } else {
                                        crate::icons::CHECK
                                    })
                                    .color(if t.error {
                                        pal.danger
                                    } else {
                                        pal.success
                                    }),
                                );
                                ui.add(egui::Label::new(RichText::new(&t.text).size(13.0)).wrap());
                                if ui
                                    .add(
                                        forms::Btn::icon(crate::icons::X, "Dismiss notification")
                                            .height(24.0),
                                    )
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
        if moving {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }

    pub fn draw_palette(&mut self, ctx: &egui::Context) {
        let open = self.palette.open;
        let shown = motion::ease(
            ctx,
            egui::Id::unique("palette-open"),
            open,
            if open { 0.2 } else { motion::EXIT },
        );
        if shown <= 0.0 {
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
        let scrim = egui::Area::new(egui::Id::unique("palette-scrim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .fade_in(false)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0, pal.scrim.gamma_multiply(shown));
                resp.clicked()
            });
        if scrim.inner && open {
            self.palette.open = false;
        }
        egui::Area::new(egui::Id::unique("palette"))
            .order(egui::Order::Tooltip)
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(
                screen.center().x,
                screen.top() + 88.0 - (1.0 - shown) * 8.0,
            ))
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_opacity(shown);
                egui::Frame::new()
                    .fill(pal.card)
                    .stroke(Stroke::new(1.0, pal.control_border))
                    .corner_radius(CornerRadius::same(crate::themes::tokens::RADIUS_LARGE))
                    .inner_margin(egui::Margin::ZERO)
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.set_width((screen.width() - 64.0).clamp(280.0, 600.0));
                        ui.spacing_mut().item_spacing.y = 0.0;
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(14, 0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.set_height(48.0);
                                    ui.label(
                                        RichText::new(crate::icons::MAGNIFYING_GLASS)
                                            .size(16.0)
                                            .color(pal.text_dim),
                                    );
                                    let te = ui.add(
                                        egui::TextEdit::singleline(&mut self.palette.query)
                                            .hint_text(
                                                RichText::new("Find a command or type user@host")
                                                    .color(pal.faint),
                                            )
                                            .font(egui::FontId::proportional(15.0))
                                            .desired_width(f32::INFINITY)
                                            .align(egui::Align2::LEFT_CENTER)
                                            .frame(egui::Frame::NONE),
                                    );
                                    te.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::Role::TextInput,
                                            true,
                                            "Find a command",
                                        )
                                    });
                                    if open {
                                        te.request_focus();
                                    }
                                    if te.changed() {
                                        self.palette.selected = 0;
                                    }
                                });
                            });
                        ui.painter().hline(
                            ui.max_rect().x_range(),
                            ui.cursor().top(),
                            Stroke::new(1.0, pal.border),
                        );
                        ui.add_space(6.0);
                        let n = items.len();
                        if open && ui.input(|i| i.key_pressed(Key::ArrowDown)) && n > 0 {
                            self.palette.selected = (self.palette.selected + 1) % n;
                        }
                        if open && ui.input(|i| i.key_pressed(Key::ArrowUp)) && n > 0 {
                            self.palette.selected = (self.palette.selected + n - 1) % n;
                        }
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(6, 0))
                            .show(ui, |ui| {
                                for (i, (_, label, detail, action)) in items.iter().enumerate() {
                                    let sel = i == self.palette.selected;
                                    let (rect, resp) = ui.allocate_exact_size(
                                        Vec2::new(ui.available_width(), 40.0),
                                        Sense::click(),
                                    );
                                    resp.widget_info(|| {
                                        egui::WidgetInfo::labeled(egui::Role::Button, true, label)
                                    });
                                    if sel || resp.hovered() {
                                        ui.painter().rect_filled(rect, 8, pal.card_hover);
                                    }
                                    ui.painter().text(
                                        rect.left_center() + Vec2::new(12.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        label,
                                        egui::FontId::proportional(14.0),
                                        pal.text,
                                    );
                                    ui.painter().text(
                                        rect.right_center() - Vec2::new(12.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        detail,
                                        egui::FontId::monospace(12.0),
                                        pal.text_dim,
                                    );
                                    if resp.clicked() && open {
                                        chosen = Some(action.clone());
                                    }
                                }
                                if items.is_empty() {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("No results").color(pal.text_dim));
                                    ui.add_space(8.0);
                                }
                                if open && ui.input(|i| i.key_pressed(Key::Enter)) {
                                    chosen = items.get(self.palette.selected).map(|x| x.3.clone());
                                }
                            });
                        ui.add_space(6.0);
                    });
            });
        if open && ctx.input(|i| i.key_pressed(Key::Escape)) {
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

fn command_button(ui: &mut egui::Ui, wide: bool, pal: Palette) -> egui::Response {
    let width = if wide { 176.0 } else { 32.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Find a command"));
    let warm = motion::ease(ui.ctx(), resp.id.with("h"), resp.hovered(), motion::FAST);
    let ink = pal.text_dim.lerp_to_gamma(pal.text, warm);
    ui.painter().rect(
        rect,
        crate::themes::tokens::RADIUS,
        pal.card,
        Stroke::new(1.0, pal.border.lerp_to_gamma(pal.control_border, warm)),
        egui::StrokeKind::Inside,
    );
    if wide {
        ui.painter().text(
            rect.left_center() + Vec2::new(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            crate::icons::MAGNIFYING_GLASS,
            egui::FontId::proportional(14.0),
            ink,
        );
        ui.painter().text(
            rect.left_center() + Vec2::new(30.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "Find a command",
            egui::FontId::proportional(13.0),
            ink,
        );
        let hint = command_shortcut("K");
        let galley = ui
            .painter()
            .layout_no_wrap(hint, egui::FontId::monospace(11.0), pal.text_dim);
        let chip = Rect::from_center_size(
            rect.right_center() - Vec2::new(8.0 + (galley.size().x + 10.0) / 2.0, 0.0),
            Vec2::new(galley.size().x + 10.0, 18.0),
        );
        ui.painter().rect(
            chip,
            4,
            pal.card_hover,
            Stroke::new(1.0, pal.border),
            egui::StrokeKind::Inside,
        );
        ui.painter()
            .galley(chip.center() - galley.size() / 2.0, galley, pal.text_dim);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            crate::icons::MAGNIFYING_GLASS,
            egui::FontId::proportional(14.0),
            ink,
        );
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            10,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Outside,
        );
    }
    resp.on_hover_text(format!("Find a command ({})", command_shortcut("K")))
}

fn tab_chip(
    ui: &mut egui::Ui,
    title: &str,
    dot: Option<Color32>,
    active: bool,
    pal: Palette,
    closable: bool,
) -> (egui::Response, bool) {
    let font = egui::FontId::proportional(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_string(), font, Color32::PLACEHOLDER);
    let extra = if dot.is_some() { 16.0 } else { 0.0 } + if closable { 22.0 } else { 0.0 };
    let size = Vec2::new(galley.size().x + 24.0 + extra, crate::themes::tokens::BAR);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, title));
    let ctx = ui.ctx();
    let hover = motion::ease(ctx, resp.id.with("h"), resp.hovered(), motion::FAST);
    let line = motion::ease(ctx, resp.id.with("line"), active, motion::CONTROL);
    let p = ui.painter();
    p.rect_filled(rect, 0, pal.card.gamma_multiply(hover));
    if line > 0.0 {
        let half = (rect.width() / 2.0 - 12.0) * line;
        p.hline(
            (rect.center().x - half)..=(rect.center().x + half),
            rect.bottom() - 1.0,
            Stroke::new(2.0, pal.accent),
        );
    }
    let ink = if active {
        pal.text
    } else {
        pal.text_dim.lerp_to_gamma(pal.text, hover)
    };
    let mut x = rect.left() + 12.0;
    if let Some(c) = dot {
        p.circle_filled(Pos2::new(x + 4.0, rect.center().y), 4.0, c);
        x += 16.0;
    }
    p.galley(
        Pos2::new(x, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    if resp.has_focus() {
        p.rect_stroke(
            rect.shrink(4.0),
            6,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    let mut close = false;
    if closable && (resp.hovered() || active) {
        let cr = Rect::from_center_size(
            Pos2::new(rect.right() - 16.0, rect.center().y),
            Vec2::splat(20.0),
        );
        let cresp = ui.interact(cr, resp.id.with("close"), Sense::click());
        cresp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Close {title}"))
        });
        if cresp.hovered() {
            ui.painter().rect_filled(cr, 5, pal.card_hover);
        }
        ui.painter().text(
            cr.center(),
            egui::Align2::CENTER_CENTER,
            crate::icons::X,
            egui::FontId::proportional(12.0),
            pal.text_dim,
        );
        close = cresp.clicked();
    }
    (resp, close)
}

/// `open` is the navigation's width animation: labels fade with it, and a collapsed
/// item explains itself in a tooltip.
fn nav_item(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    active: bool,
    open: f32,
    pal: Palette,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, active, label));
    let ctx = ui.ctx();
    let hover = motion::ease(
        ctx,
        resp.id.with("h"),
        resp.hovered() && !active,
        motion::FAST,
    );
    let on = motion::ease(ctx, resp.id.with("on"), active, motion::FAST);
    let p = ui.painter();
    p.rect_filled(rect, 8, pal.card.gamma_multiply(hover));
    p.rect_filled(rect, 8, pal.card_hover.gamma_multiply(on));
    if resp.has_focus() {
        p.rect_stroke(
            rect.shrink(1.0),
            8,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    let ink = pal.text_dim.lerp_to_gamma(pal.text, hover.max(on));
    p.text(
        rect.left_center() + Vec2::new(11.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        pal.text_dim
            .lerp_to_gamma(pal.text, hover)
            .lerp_to_gamma(pal.accent, on),
    );
    if open > 0.0 {
        p.with_clip_rect(rect).text(
            rect.left_center() + Vec2::new(40.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(14.0),
            ink.gamma_multiply(open),
        );
    }
    if open < 0.5 {
        resp.on_hover_text(label)
    } else {
        resp
    }
}
