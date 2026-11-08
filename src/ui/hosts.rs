//! Hosts page: groups, hosts, search / quick connect, tags, bulk actions, drag & drop.

use std::collections::HashSet;

use egui::{RichText, Sense, Stroke, Vec2};
use uuid::Uuid;

use super::{forms, motion};
use crate::app::{App, Dialog, Editor, Page, Tab};
use crate::conn::AppEvent;
use crate::model::{Group, Host, Protocol, Vault, now_ts};

enum HostAction {
    Connect(Uuid),
    Sftp(Uuid),
    Edit(Uuid),
    Duplicate(Uuid),
    Delete(Vec<Uuid>),
    OpenGroup(Option<Uuid>),
    EditGroup(Uuid),
    DeleteGroup(Uuid),
    MoveTo(Vec<Uuid>, Option<Uuid>),
    ConnectMany(Vec<Uuid>),
    TogglePin(Uuid),
    CopyAddress(String),
}

impl App {
    fn connected_hosts(&self) -> HashSet<Uuid> {
        let mut s = HashSet::new();
        for t in &self.tabs {
            if let Tab::Terminal(tt) = t {
                for p in &tt.panes {
                    if p.session.is_live() {
                        if let Some(id) = p.session.target.host_id {
                            s.insert(id);
                        }
                    }
                }
            }
        }
        s
    }

    pub fn hosts_page(&mut self, ui: &mut egui::Ui) {
        use crate::themes::tokens;
        let pal = self.pal();
        let mut action = None;
        let short_window = ui.ctx().content_rect().height() < 640.0;
        let total = self.vault.hosts.len();
        let shown = self.filtered_hosts().len();
        let search_id = egui::Id::unique("host-search");
        let mut search_resp = None;
        let wide = ui.available_width() >= 780.0;
        let trailing = |app: &mut App, ui: &mut egui::Ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(forms::Btn::primary(format!(
                        "{}  New host",
                        crate::icons::PLUS
                    )))
                    .clicked()
                {
                    app.editor = Some(Editor::Host(
                        Box::new(Host {
                            group_id: app.current_group,
                            ..Default::default()
                        }),
                        true,
                    ));
                }
                if ui
                    .add(forms::Btn::ghost(format!(
                        "{}  Import",
                        crate::icons::DOWNLOAD_SIMPLE
                    )))
                    .clicked()
                {
                    app.dialog = Some(Dialog::Import { error: None });
                }
                egui::containers::menu::MenuButton::from_button(
                    egui::Button::new(crate::icons::DOTS_THREE).min_size(Vec2::splat(36.0)),
                )
                .ui(ui, |ui| app.host_tools_menu(ui))
                .0
                .on_hover_text("Host actions, view and import");
            });
        };
        ui.horizontal(|ui| {
            ui.set_min_height(tokens::CONTROL);
            ui.spacing_mut().item_spacing.x = 12.0;
            forms::heading(ui, "Hosts", tokens::TITLE);
            ui.label(
                RichText::new(format!("{shown} of {total}"))
                    .size(13.0)
                    .color(pal.text_dim),
            );
            if wide {
                ui.push_id(search_id, |ui| {
                    let width = (ui.available_width() - 510.0).clamp(160.0, 460.0);
                    search_resp = Some(
                        forms::Input::bare("Search saved hosts")
                            .hint("Search hosts, addresses or tags")
                            .icon(crate::icons::MAGNIFYING_GLASS)
                            .width(width)
                            .show(ui, &mut self.search),
                    );
                });
                self.density_control(ui);
            }
            trailing(self, ui);
        });
        ui.add_space(if short_window { 8.0 } else { 12.0 });
        if !wide {
            ui.horizontal(|ui| {
                ui.push_id(search_id, |ui| {
                    let width = (ui.available_width() - 196.0).max(160.0);
                    search_resp = Some(
                        forms::Input::bare("Search saved hosts")
                            .hint("Search hosts, addresses or tags")
                            .icon(crate::icons::MAGNIFYING_GLASS)
                            .width(width)
                            .show(ui, &mut self.search),
                    );
                });
                self.density_control(ui);
            });
            ui.add_space(8.0);
        }
        if let Some(search) = &search_resp {
            if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let hosts = self.filtered_hosts();
                if let Some(h) = hosts
                    .iter()
                    .find(|h| Some(h.id) == self.inspected_host)
                    .or_else(|| hosts.first())
                {
                    action = Some(HostAction::Connect(h.id));
                }
            }
        }
        // Active filters read as removable chips, so a narrowed list never looks like data loss.
        if self.current_group.is_some() || self.tag_filter.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(group) = self.current_group {
                    if ui
                        .add(
                            forms::Btn::secondary(format!(
                                "{}  {}",
                                self.vault.group_path(Some(group)),
                                crate::icons::X
                            ))
                            .small(),
                        )
                        .on_hover_text("Show all groups")
                        .clicked()
                    {
                        action = Some(HostAction::OpenGroup(None));
                    }
                }
                if let Some(tag) = self.tag_filter.clone() {
                    if ui
                        .add(forms::Btn::secondary(format!("#{tag}  {}", crate::icons::X)).small())
                        .on_hover_text("Show every tag")
                        .clicked()
                    {
                        self.tag_filter = None;
                    }
                }
            });
            ui.add_space(8.0);
        }
        let selection_open = !self.selected_hosts.is_empty();
        forms::reveal(ui, egui::Id::unique("bulk-bar"), selection_open, |ui| {
            let selected: Vec<_> = self.selected_hosts.iter().copied().collect();
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} selected", selected.len()));
                if ui
                    .add(forms::Btn::secondary("Connect all").small())
                    .clicked()
                {
                    action = Some(HostAction::ConnectMany(selected.clone()));
                }
                ui.menu_button("Move to group", |ui| {
                    crate::ui::forms::menu_padding(ui);
                    if ui.button("No group").clicked() {
                        action = Some(HostAction::MoveTo(selected.clone(), None));
                        ui.close();
                    }
                    for group in &self.vault.groups {
                        if ui.button(self.vault.group_path(Some(group.id))).clicked() {
                            action = Some(HostAction::MoveTo(selected.clone(), Some(group.id)));
                            ui.close();
                        }
                    }
                });
                if ui.add(forms::Btn::danger("Delete hosts").small()).clicked() {
                    action = Some(HostAction::Delete(selected.clone()));
                }
                if ui
                    .add(forms::Btn::ghost("Clear selection").small())
                    .clicked()
                {
                    self.selected_hosts.clear();
                }
            });
            ui.add_space(8.0);
        });
        self.schedule_host_probes(ui.input(|i| i.time));
        let hosts = self.filtered_hosts();
        if !hosts.iter().any(|h| Some(h.id) == self.inspected_host) {
            self.inspected_host = hosts.first().map(|h| h.id);
        }
        let connected = self.connected_hosts();
        let grid = self.vault.settings.hosts_grid_view;
        let width = ui.available_width();
        let area_height = (ui.available_height() - 28.0).max(80.0);
        let mut focus_next = None;
        egui::ScrollArea::vertical()
            .id_salt("host-library")
            .max_height(area_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if hosts.is_empty() {
                    self.library_empty_state(ui);
                    return;
                }
                if grid {
                    self.host_grid(ui, &hosts, width, &connected, &mut action, &mut focus_next);
                    return;
                }
                let sections = library_sections(&self.vault, &hosts);
                let collapsed = |ui: &egui::Ui, key: Option<Uuid>| {
                    ui.data(|d| {
                        d.get_temp::<bool>(egui::Id::unique(("library-group", key)))
                            .unwrap_or(false)
                    })
                };
                // Arrow keys walk what is on screen, so folded groups are skipped.
                let order: Vec<Uuid> = sections
                    .iter()
                    .filter(|section| !collapsed(ui, section.group))
                    .flat_map(|section| section.hosts.iter().map(|h| h.id))
                    .collect();
                let dense = motion::ease(
                    ui.ctx(),
                    egui::Id::unique("library-density"),
                    self.vault.settings.hosts_compact,
                    motion::PANEL,
                );
                let row_height = tokens::ROW + (tokens::COMPACT_ROW - tokens::ROW) * dense;
                ui.spacing_mut().item_spacing.y = 0.0;
                for section in &sections {
                    let key = section.group;
                    let closed = collapsed(ui, key);
                    let header = self.group_header(ui, section, closed, &mut action);
                    if header.clicked() {
                        ui.data_mut(|d| {
                            d.insert_temp(egui::Id::unique(("library-group", key)), !closed)
                        });
                    }
                    forms::reveal(ui, egui::Id::unique(("library-rows", key)), !closed, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for h in &section.hosts {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), row_height),
                                Sense::hover(),
                            );
                            self.host_row(
                                ui,
                                rect,
                                h,
                                &order,
                                &connected,
                                &mut action,
                                &mut focus_next,
                            );
                        }
                    });
                    ui.add_space(4.0);
                }
            });
        if let Some(id) = focus_next {
            self.inspected_host = Some(id);
            ui.memory_mut(|m| m.request_focus(egui::Id::unique(("host-row", id))));
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new("A TCP probe checks the port, not your login.")
                .small()
                .color(pal.faint),
        );
        if let Some(action) = action {
            self.apply_host_action(action);
        }
    }

    fn density_control(&mut self, ui: &mut egui::Ui) {
        if self.vault.settings.hosts_grid_view {
            return;
        }
        if forms::segmented(
            ui,
            "library-density",
            &mut self.vault.settings.hosts_compact,
            &[(false, "Comfortable"), (true, "Compact")],
        ) {
            crate::store::touch_settings(&mut self.vault);
            self.mark_dirty();
        }
    }

    fn library_empty_state(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        ui.add_space(32.0);
        let filtered =
            !self.search.is_empty() || self.tag_filter.is_some() || self.current_group.is_some();
        if filtered {
            forms::heading(
                ui,
                &if self.search.is_empty() {
                    "No hosts match these filters".to_string()
                } else {
                    format!("No hosts match “{}”", self.search)
                },
                16.0,
            );
            ui.label(
                RichText::new("Check the spelling, or search by address or tag.")
                    .color(pal.text_dim),
            );
        } else {
            forms::heading(ui, "No hosts yet", 16.0);
            ui.label(
                RichText::new("Add a host to save its address and login.").color(pal.text_dim),
            );
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if filtered && ui.add(forms::Btn::secondary("Clear filters")).clicked() {
                self.search.clear();
                self.tag_filter = None;
                self.current_group = None;
            }
            if ui.add(forms::Btn::primary("Add host")).clicked() {
                self.editor = Some(Editor::Host(
                    Box::new(Host {
                        group_id: self.current_group,
                        ..Default::default()
                    }),
                    true,
                ));
            }
            if ui.add(forms::Btn::secondary("Import hosts")).clicked() {
                self.dialog = Some(Dialog::Import { error: None });
            }
            if ui.add(forms::Btn::ghost("Quick connect")).clicked() {
                self.dialog = Some(Dialog::QuickConnect {
                    input: String::new(),
                    error: None,
                });
            }
        });
    }

    fn group_header(
        &mut self,
        ui: &mut egui::Ui,
        section: &LibrarySection,
        closed: bool,
        action: &mut Option<HostAction>,
    ) -> egui::Response {
        let pal = self.pal();
        let id = egui::Id::unique(("library-header", section.group));
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
        let resp = ui.interact(rect, id, Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, section.name.as_str())
        });
        let ctx = ui.ctx();
        let hover = motion::ease(ctx, id.with("h"), resp.hovered(), motion::FAST);
        let open = motion::ease_panel(ctx, id.with("turn"), !closed);
        let p = ui.painter();
        p.rect_filled(rect, 8, super::glass::hover_wash(ui).gamma_multiply(hover));
        let ink = pal.text_dim.lerp_to_gamma(pal.text, hover);
        forms::chevron(
            ui,
            rect.left_center() + Vec2::new(14.0, 0.0),
            std::f32::consts::FRAC_PI_2 * open,
            ink,
        );
        let name = p.layout_no_wrap(
            section.name.clone(),
            egui::FontId::new(12.0, crate::fonts::medium()),
            ink,
        );
        let name_width = name.size().x;
        p.galley(
            egui::pos2(rect.left() + 26.0, rect.center().y - name.size().y / 2.0),
            name,
            ink,
        );
        p.text(
            egui::pos2(rect.left() + 34.0 + name_width, rect.center().y),
            egui::Align2::LEFT_CENTER,
            section.hosts.len().to_string(),
            egui::FontId::monospace(12.0),
            pal.faint,
        );
        if resp.has_focus() {
            p.rect_stroke(
                rect.shrink(1.0),
                6,
                Stroke::new(2.0, pal.accent),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(group) = section.group {
            if resp.dnd_hover_payload::<Uuid>().is_some() {
                ui.painter().rect_stroke(
                    rect,
                    6,
                    Stroke::new(2.0, pal.accent),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(payload) = resp.dnd_release_payload::<Uuid>() {
                let ids = if self.selected_hosts.contains(&payload) {
                    self.selected_hosts.iter().copied().collect()
                } else {
                    vec![*payload]
                };
                *action = Some(HostAction::MoveTo(ids, Some(group)));
            }
            resp.context_menu(|ui| {
                crate::ui::forms::menu_padding(ui);
                ui.label(format!("{} hosts", self.count_in_group(group)));
                if ui.button("Show only this group").clicked() {
                    *action = Some(HostAction::OpenGroup(Some(group)));
                    ui.close();
                }
                if ui.button("Edit group settings").clicked() {
                    *action = Some(HostAction::EditGroup(group));
                    ui.close();
                }
                if ui.button("Connect to group").clicked() {
                    *action = Some(HostAction::ConnectMany(
                        self.vault
                            .hosts
                            .iter()
                            .filter(|h| {
                                self.vault
                                    .group_chain(h.group_id)
                                    .iter()
                                    .any(|parent| parent.id == group)
                            })
                            .map(|h| h.id)
                            .collect(),
                    ));
                    ui.close();
                }
                if ui.button("Delete group").clicked() {
                    *action = Some(HostAction::DeleteGroup(group));
                    ui.close();
                }
            });
        }
        resp
    }

    /// Pointer, keyboard and menu behavior shared by list rows and grid cards.
    #[allow(clippy::too_many_arguments)]
    fn host_interactions(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        id: egui::Id,
        h: &Host,
        order: &[Uuid],
        connected: &HashSet<Uuid>,
        action: &mut Option<HostAction>,
        focus_next: &mut Option<Uuid>,
    ) {
        if host_drag_started(ui, response, id) {
            egui::DragAndDrop::set_payload(ui.ctx(), h.id);
        }
        if response.clicked() {
            if ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                if !self.selected_hosts.insert(h.id) {
                    self.selected_hosts.remove(&h.id);
                }
            } else {
                self.selected_hosts.clear();
                self.inspected_host = Some(h.id);
                self.host_inspector_open = ui.ctx().content_rect().width() < 1200.0;
                response.request_focus();
            }
        }
        if response.double_clicked() {
            *action = Some(HostAction::Connect(h.id));
        }
        if response.has_focus() && !self.palette.open && self.dialog.is_none() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                *action = Some(HostAction::Connect(h.id));
            }
            let direction = ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowDown) {
                    1
                } else if i.key_pressed(egui::Key::ArrowUp) {
                    -1
                } else {
                    0
                }
            });
            if direction != 0 {
                if let Some(index) = order.iter().position(|other| *other == h.id) {
                    *focus_next = Some(
                        order[(index as isize + direction).clamp(0, order.len() as isize - 1)
                            as usize],
                    );
                }
            }
        }
        if response.gained_focus() {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        response.clone().on_hover_text(format!(
            "{}\n{}\n{}{}",
            h.display_name(),
            self.host_subtitle(h),
            if connected.contains(&h.id) {
                "Session connected · "
            } else {
                ""
            },
            self.host_probe_label(h).0
        ));
        response.context_menu(|ui| {
            crate::ui::forms::menu_padding(ui);
            if ui.button("Connect to host").clicked() {
                *action = Some(HostAction::Connect(h.id));
                ui.close();
            }
            if h.protocol == Protocol::Ssh && ui.button("Open SFTP").clicked() {
                *action = Some(HostAction::Sftp(h.id));
                ui.close();
            }
            if ui.button("Edit host").clicked() {
                *action = Some(HostAction::Edit(h.id));
                ui.close();
            }
            if ui.button("Duplicate host").clicked() {
                *action = Some(HostAction::Duplicate(h.id));
                ui.close();
            }
            if ui
                .button(if h.pinned { "Unpin host" } else { "Pin host" })
                .clicked()
            {
                *action = Some(HostAction::TogglePin(h.id));
                ui.close();
            }
            ui.menu_button("Move to group", |ui| {
                crate::ui::forms::menu_padding(ui);
                if ui.button("No group").clicked() {
                    *action = Some(HostAction::MoveTo(vec![h.id], None));
                    ui.close();
                }
                for g in &self.vault.groups {
                    if ui.button(self.vault.group_path(Some(g.id))).clicked() {
                        *action = Some(HostAction::MoveTo(vec![h.id], Some(g.id)));
                        ui.close();
                    }
                }
            });
            if ui.button("Copy address").clicked() {
                *action = Some(HostAction::CopyAddress(h.address.clone()));
                ui.close();
            }
            ui.separator();
            if ui
                .button(RichText::new("Delete host").color(self.pal().danger))
                .clicked()
            {
                *action = Some(HostAction::Delete(vec![h.id]));
                ui.close();
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn host_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        h: &Host,
        order: &[Uuid],
        connected: &HashSet<Uuid>,
        action: &mut Option<HostAction>,
        focus_next: &mut Option<Uuid>,
    ) {
        use crate::themes::tokens;
        let pal = self.pal();
        let id = egui::Id::unique(("host-row", h.id));
        let response = ui.interact(rect, id, Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!(
                    "{}, {}, {}",
                    h.display_name(),
                    h.address,
                    h.protocol.label()
                ),
            )
        });
        let selected = Some(h.id) == self.inspected_host || self.selected_hosts.contains(&h.id);
        if ui.is_rect_visible(rect) {
            let ctx = ui.ctx();
            let hover = motion::ease(ctx, id.with("h"), response.hovered(), motion::FAST);
            let warm = motion::ease(ctx, id.with("warm"), selected, motion::FAST);
            let marker = motion::ease(ctx, id.with("mark"), selected, motion::CONTROL);
            let compact = rect.height() < (tokens::ROW + tokens::COMPACT_ROW) / 2.0;
            let wide = rect.width() > 760.0;
            let p = ui.painter();
            p.rect_filled(rect, 10, super::glass::hover_wash(ui).gamma_multiply(hover));
            p.rect_filled(
                rect,
                10,
                super::glass::selection_fill(ui).gamma_multiply(warm),
            );
            if marker > 0.0 {
                let half = (rect.height() / 2.0 - 10.0) * marker;
                p.vline(
                    rect.left() + 1.0,
                    (rect.center().y - half)..=(rect.center().y + half),
                    Stroke::new(2.0, pal.accent),
                );
            }
            if response.has_focus() {
                p.rect_stroke(
                    rect.shrink(1.0),
                    8,
                    Stroke::new(2.0, pal.accent),
                    egui::StrokeKind::Inside,
                );
            }
            let icon_size = if compact { 28.0 } else { 32.0 };
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 12.0 + icon_size / 2.0, rect.center().y),
                Vec2::splat(icon_size),
            );
            p.rect_filled(icon_rect, 8, super::glass::hover_wash(ui));
            p.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                if h.protocol == Protocol::Serial {
                    crate::icons::USB
                } else {
                    crate::icons::HARD_DRIVES
                },
                egui::FontId::proportional(16.0),
                pal.text_dim.lerp_to_gamma(pal.accent, warm),
            );
            if connected.contains(&h.id) {
                p.circle(
                    icon_rect.right_bottom() - Vec2::splat(2.0),
                    4.0,
                    pal.success,
                    Stroke::new(2.0, pal.bg),
                );
            }
            let (probe, probe_color) = self.host_probe_label(h);
            let probe_right = rect.right() - 12.0;
            let probe_left = probe_right - 124.0;
            let chip_left = probe_left - 12.0 - 56.0;
            let who_left = icon_rect.right() + 12.0;
            let tags_width = if wide {
                (chip_left - who_left) * 0.36
            } else {
                0.0
            };
            let who_right = chip_left - 12.0 - tags_width;
            let who = egui::Rect::from_min_max(
                egui::pos2(who_left, rect.top()),
                egui::pos2(who_right.max(who_left), rect.bottom()),
            );
            let name = format!(
                "{}{}",
                if h.pinned {
                    format!("{}  ", crate::icons::PUSH_PIN)
                } else {
                    String::new()
                },
                h.display_name()
            );
            let name_font = egui::FontId::new(14.0, crate::fonts::medium());
            let address = self.host_subtitle(h);
            let clip = p.with_clip_rect(who);
            if compact {
                let shown = clip.text(
                    who.left_center(),
                    egui::Align2::LEFT_CENTER,
                    &name,
                    name_font,
                    pal.text,
                );
                clip.text(
                    egui::pos2(shown.right() + 8.0, who.center().y),
                    egui::Align2::LEFT_CENTER,
                    address,
                    egui::FontId::monospace(12.0),
                    pal.text_dim,
                );
            } else {
                clip.text(
                    egui::pos2(who_left, rect.center().y - 9.0),
                    egui::Align2::LEFT_CENTER,
                    &name,
                    name_font,
                    pal.text,
                );
                clip.text(
                    egui::pos2(who_left, rect.center().y + 10.0),
                    egui::Align2::LEFT_CENTER,
                    address,
                    egui::FontId::monospace(12.0),
                    pal.text_dim,
                );
            }
            if wide {
                let area = egui::Rect::from_min_max(
                    egui::pos2(who_right + 12.0, rect.top()),
                    egui::pos2(chip_left - 12.0, rect.bottom()),
                );
                let clip = p.with_clip_rect(area);
                let mut x = area.left();
                for tag in &h.tags {
                    let galley = clip.layout_no_wrap(
                        tag.clone(),
                        egui::FontId::proportional(tokens::CAPTION),
                        pal.text_dim,
                    );
                    let chip = egui::Rect::from_min_size(
                        egui::pos2(x, rect.center().y - 10.0),
                        Vec2::new(galley.size().x + 12.0, 20.0),
                    );
                    clip.rect_filled(chip, 9, super::glass::hover_wash(ui));
                    x = chip.right() + 4.0;
                    clip.galley(chip.center() - galley.size() / 2.0, galley, pal.text_dim);
                }
            }
            let chip_galley = p.layout_no_wrap(
                h.protocol.label().to_string(),
                egui::FontId::monospace(tokens::CAPTION),
                pal.text_dim,
            );
            let chip = egui::Rect::from_min_size(
                egui::pos2(chip_left, rect.center().y - 10.0),
                Vec2::new(chip_galley.size().x + 12.0, 20.0),
            );
            p.rect_filled(chip, 9, super::glass::hover_wash(ui));
            p.galley(
                chip.center() - chip_galley.size() / 2.0,
                chip_galley,
                pal.text_dim,
            );
            let drawn = p.text(
                egui::pos2(probe_right, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                &probe,
                egui::FontId::proportional(13.0),
                probe_color,
            );
            if probe_color != pal.text_dim {
                p.circle_filled(
                    egui::pos2(drawn.left() - 10.0, rect.center().y),
                    4.0,
                    probe_color,
                );
            }
        }
        self.host_interactions(ui, &response, id, h, order, connected, action, focus_next);
    }

    fn host_grid(
        &mut self,
        ui: &mut egui::Ui,
        hosts: &[Host],
        width: f32,
        connected: &HashSet<Uuid>,
        action: &mut Option<HostAction>,
        focus_next: &mut Option<Uuid>,
    ) {
        use crate::themes::tokens;
        let pal = self.pal();
        let columns = ((width + 12.0) / 288.0).floor().max(1.0) as usize;
        let card_width = (width - 12.0 * (columns - 1) as f32) / columns as f32;
        let order: Vec<Uuid> = hosts.iter().map(|h| h.id).collect();
        for row in hosts.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for h in row {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(card_width, 120.0), Sense::hover());
                    let id = egui::Id::unique(("host-row", h.id));
                    let response = ui.interact(rect, id, Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            true,
                            format!("{}, {}", h.display_name(), h.address),
                        )
                    });
                    let selected =
                        Some(h.id) == self.inspected_host || self.selected_hosts.contains(&h.id);
                    let ctx = ui.ctx();
                    let hover = motion::ease(ctx, id.with("h"), response.hovered(), motion::FAST);
                    let warm = motion::ease(ctx, id.with("warm"), selected, motion::FAST);
                    let p = ui.painter();
                    p.rect(
                        rect,
                        tokens::RADIUS_LARGE,
                        pal.card
                            .lerp_to_gamma(pal.card_hover, hover)
                            .lerp_to_gamma(pal.accent_soft, warm),
                        Stroke::new(1.0, pal.border.lerp_to_gamma(pal.accent, warm)),
                        egui::StrokeKind::Inside,
                    );
                    if response.has_focus() {
                        p.rect_stroke(
                            rect.expand(2.0),
                            14,
                            Stroke::new(2.0, pal.accent),
                            egui::StrokeKind::Outside,
                        );
                    }
                    let inner = rect.shrink(14.0);
                    let clip = p.with_clip_rect(inner);
                    clip.text(
                        inner.left_top(),
                        egui::Align2::LEFT_TOP,
                        h.display_name(),
                        egui::FontId::new(14.0, crate::fonts::medium()),
                        pal.text,
                    );
                    clip.text(
                        inner.left_top() + Vec2::new(0.0, 22.0),
                        egui::Align2::LEFT_TOP,
                        self.host_subtitle(h),
                        egui::FontId::monospace(12.0),
                        pal.text_dim,
                    );
                    clip.text(
                        inner.left_bottom(),
                        egui::Align2::LEFT_BOTTOM,
                        self.host_group_label(h.group_id),
                        egui::FontId::proportional(tokens::CAPTION),
                        pal.text_dim,
                    );
                    let (probe, color) = self.host_probe_label(h);
                    clip.text(
                        inner.right_bottom(),
                        egui::Align2::RIGHT_BOTTOM,
                        probe,
                        egui::FontId::proportional(13.0),
                        color,
                    );
                    self.host_interactions(
                        ui, &response, id, h, &order, connected, action, focus_next,
                    );
                }
            });
            ui.add_space(12.0);
        }
    }

    pub fn host_inspector(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let Some(host) = self
            .inspected_host
            .and_then(|id| self.vault.host(id))
            .cloned()
        else {
            return;
        };
        let target = self.vault.resolve(&host);
        let mut action = None;
        // Content steps in 8 pt from the side it lives on.
        let enter = motion::since_change(
            ui.ctx(),
            egui::Id::unique("inspector-host"),
            host.id,
            motion::PANEL,
        );
        ui.set_opacity(enter);
        egui::Frame::NONE
            .outer_margin(egui::Margin {
                left: ((1.0 - enter) * 8.0) as i8,
                ..Default::default()
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                    ui.painter().rect_filled(rect, 12, pal.card_hover);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if host.protocol == Protocol::Serial {
                            crate::icons::USB
                        } else {
                            crate::icons::HARD_DRIVES
                        },
                        egui::FontId::proportional(22.0),
                        pal.accent,
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(host.display_name())
                                    .size(16.0)
                                    .family(crate::fonts::semibold()),
                            )
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(self.host_subtitle(&host))
                                    .monospace()
                                    .size(12.0)
                                    .color(pal.text_dim),
                            )
                            .truncate(),
                        );
                    });
                });
                ui.add_space(8.0);
                if ui
                    .add(forms::Btn::primary(format!("{}  Connect", crate::icons::PLAY)).full())
                    .clicked()
                {
                    action = Some(HostAction::Connect(host.id));
                }
                let can_sftp = host.protocol == Protocol::Ssh;
                ui.horizontal(|ui| {
                    let width = if can_sftp {
                        (ui.available_width() - 8.0) / 2.0
                    } else {
                        ui.available_width()
                    };
                    if ui
                        .add(
                            forms::Btn::secondary(format!("{}  Edit", crate::icons::PENCIL_SIMPLE))
                                .min_width(width),
                        )
                        .clicked()
                    {
                        action = Some(HostAction::Edit(host.id));
                    }
                    if can_sftp
                        && ui
                            .add(
                                forms::Btn::secondary(format!(
                                    "{}  SFTP",
                                    crate::icons::ARROWS_DOWN_UP
                                ))
                                .min_width(width),
                            )
                            .clicked()
                    {
                        action = Some(HostAction::Sftp(host.id));
                    }
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("inspector-details")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let (probe, color) = self.host_probe_label(&host);
                        super::glass::card_frame(ui.ctx(), crate::themes::tokens::RADIUS_LARGE)
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 4.0;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    if color != pal.text_dim {
                                        forms::dot(ui, color);
                                    }
                                    ui.label(
                                        RichText::new(
                                            if color == pal.success || color == pal.warning {
                                                format!("Port open, {probe}")
                                            } else {
                                                probe.clone()
                                            },
                                        )
                                        .family(crate::fonts::medium())
                                        .color(color),
                                    );
                                });
                                forms::help(ui, probe_note(&probe), pal);
                                if self.connected_hosts().contains(&host.id) {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        forms::dot(ui, pal.success);
                                        ui.label(
                                            RichText::new("Session connected")
                                                .size(12.0)
                                                .color(pal.success),
                                        );
                                    });
                                }
                            });
                        ui.add_space(8.0);
                        let auth = if host.protocol == Protocol::Serial {
                            "Not applicable".to_string()
                        } else if let Some(key) = &target.key {
                            format!("Key: {}", key.label)
                        } else if target.use_agent {
                            "SSH agent".into()
                        } else if target.password.is_some() {
                            "Saved password".into()
                        } else {
                            "Ask when connecting".into()
                        };
                        let route = if host.protocol == Protocol::Serial {
                            format!("{} baud", host.serial.baud_rate)
                        } else if !target.chain.is_empty() {
                            format!("Via {} jump hosts", target.chain.len())
                        } else if target.proxy.is_some() {
                            "Via proxy".into()
                        } else {
                            "Direct".into()
                        };
                        let port = if matches!(host.protocol, Protocol::Serial | Protocol::Local) {
                            "–".to_string()
                        } else {
                            target.port.to_string()
                        };
                        let username = if target.username.is_empty() {
                            "Not set".to_string()
                        } else {
                            target.username.clone()
                        };
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (label, value, mono) in [
                            ("Protocol", host.protocol.label().to_string(), false),
                            ("Port", port, true),
                            ("Username", username, true),
                            ("Sign-in", auth, false),
                            ("Route", route, false),
                            ("Group", self.host_group_label(host.group_id), false),
                            (
                                "System",
                                host.os
                                    .clone()
                                    .unwrap_or_else(|| "Detected after sign-in".into()),
                                false,
                            ),
                        ] {
                            detail_row(ui, label, &value, mono, pal);
                        }
                        ui.spacing_mut().item_spacing.y = 8.0;
                        if !host.tags.is_empty() {
                            ui.add_space(12.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                                for tag in &host.tags {
                                    forms::chip(ui, tag, false, pal);
                                }
                            });
                        }
                        if !host.notes.trim().is_empty() {
                            ui.add_space(12.0);
                            forms::label(ui, "Notes", pal);
                            ui.add(egui::Label::new(&host.notes).wrap());
                        }
                    });
            });
        if let Some(action) = action {
            self.apply_host_action(action);
        }
    }

    fn host_group_label(&self, group: Option<Uuid>) -> String {
        let label = self.vault.group_path(group);
        if label.is_empty() {
            "Ungrouped".into()
        } else {
            label
        }
    }

    fn host_probe_label(&self, host: &Host) -> (String, egui::Color32) {
        let pal = self.pal();
        if matches!(host.protocol, Protocol::Serial | Protocol::Local) {
            return ("No TCP probe".into(), pal.text_dim);
        }
        if !self.vault.settings.show_host_status {
            return ("Probes off".into(), pal.text_dim);
        }
        let target = self.vault.resolve(host);
        if !target.chain.is_empty() || target.proxy.is_some() {
            return ("Probe skipped".into(), pal.text_dim);
        }
        match self.host_status.get(&host.id) {
            Some(Some(ms)) => (
                format!("{ms} ms"),
                if *ms < 200 { pal.success } else { pal.warning },
            ),
            Some(None) => ("Unreachable".into(), pal.danger),
            None => (
                if self.host_probe_running {
                    "Checking…"
                } else {
                    "Not checked"
                }
                .into(),
                pal.text_dim,
            ),
        }
    }

    fn host_tools_menu(&mut self, ui: &mut egui::Ui) {
        crate::ui::forms::menu_padding(ui);
        ui.set_min_width(230.0);
        let before = self.vault.settings.clone();
        ui.checkbox(&mut self.vault.settings.hosts_grid_view, "Grid view");
        if before != self.vault.settings {
            crate::store::touch_settings(&mut self.vault);
            self.mark_dirty();
        }
        if !self.vault.all_tags().is_empty() {
            ui.menu_button("Filter by tag", |ui| {
                crate::ui::forms::menu_padding(ui);
                if ui
                    .selectable_label(self.tag_filter.is_none(), "Any tag")
                    .clicked()
                {
                    self.tag_filter = None;
                    ui.close();
                }
                for tag in self.vault.all_tags() {
                    if ui
                        .selectable_label(self.tag_filter.as_ref() == Some(&tag), &tag)
                        .clicked()
                    {
                        self.tag_filter = Some(tag);
                        ui.close();
                    }
                }
            });
        }
        if !self.vault.groups.is_empty() {
            ui.menu_button("Show one group", |ui| {
                crate::ui::forms::menu_padding(ui);
                if ui
                    .selectable_label(self.current_group.is_none(), "All groups")
                    .clicked()
                {
                    self.current_group = None;
                    ui.close();
                }
                for group in self.vault.groups.clone() {
                    if ui
                        .selectable_label(
                            self.current_group == Some(group.id),
                            self.vault.group_path(Some(group.id)),
                        )
                        .clicked()
                    {
                        self.current_group = Some(group.id);
                        self.inspected_host = None;
                        ui.close();
                    }
                }
            });
        }
        ui.separator();
        if ui.button("Import ~/.ssh/config").clicked() {
            self.import_default_ssh_config();
            ui.close();
        }
        if ui.button("Quick connect").clicked() {
            self.dialog = Some(Dialog::QuickConnect {
                input: String::new(),
                error: None,
            });
            ui.close();
        }
        if ui.button("\u{E25A}  New group").clicked() {
            let g = Group {
                parent_id: self.current_group,
                ..Default::default()
            };
            self.editor = Some(Editor::Group(g, true));
            ui.close();
        }
        if ui
            .button("\u{E20C}  Import (ssh config, PuTTY, MobaXterm, SecureCRT, CSV)…")
            .clicked()
        {
            self.dialog = Some(Dialog::Import { error: None });
            ui.close();
        }
        ui.menu_button("\u{E1AA}  Import from cloud", |ui| {
            crate::ui::forms::menu_padding(ui);
            for (name, which) in [
                ("DigitalOcean", 0u8),
                ("AWS EC2", 1),
                ("Microsoft Azure", 2),
            ] {
                if ui
                    .add_enabled(!self.cloud_busy, egui::Button::new(name))
                    .clicked()
                {
                    self.cloud_import(which);
                    ui.close();
                }
            }
            ui.label(
                RichText::new("Credentials: Settings → Integrations")
                    .small()
                    .weak(),
            );
        });
        if ui.button("\u{E4C0}  Export hosts as CSV…").clicked() {
            if let Some(p) = rfd::FileDialog::new()
                .set_file_name("ferrum-hosts.csv")
                .save_file()
            {
                let r = std::fs::write(&p, crate::importers::export_csv(&self.vault, false));
                self.toast(
                    if r.is_ok() {
                        "Exported hosts"
                    } else {
                        "Export failed"
                    },
                    r.is_err(),
                );
            }
            ui.close();
        }
        if ui.button("\u{E4C0}  Export as ssh_config…").clicked() {
            if let Some(p) = rfd::FileDialog::new().set_file_name("config").save_file() {
                let r = std::fs::write(&p, crate::importers::export_ssh_config(&self.vault));
                self.toast(
                    if r.is_ok() {
                        "Exported ssh_config"
                    } else {
                        "Export failed"
                    },
                    r.is_err(),
                );
            }
            ui.close();
        }
        ui.separator();
        if ui.button("\u{EB56}  Serial connection…").clicked() {
            self.dialog = Some(Dialog::Serial {
                cfg: crate::model::SerialConfig::default(),
            });
            ui.close();
        }
    }

    fn count_in_group(&self, gid: Uuid) -> usize {
        let mut ids = vec![gid];
        let mut i = 0;
        while i < ids.len() {
            let cur = ids[i];
            ids.extend(
                self.vault
                    .groups
                    .iter()
                    .filter(|g| g.parent_id == Some(cur))
                    .map(|g| g.id),
            );
            i += 1;
        }
        self.vault
            .hosts
            .iter()
            .filter(|h| h.group_id.map(|g| ids.contains(&g)).unwrap_or(false))
            .count()
    }

    /// Starts a reachability round for the saved hosts once the last one is a minute old.
    /// Hosts behind a jump host or proxy are skipped: a direct TCP connect would say nothing.
    fn schedule_host_probes(&mut self, now: f64) {
        const EVERY: f64 = 60.0;
        const PARALLEL: usize = 8;
        if self.qa.is_some()
            || !self.vault.settings.show_host_status
            || self.host_probe_running
            || self.host_probe_at.is_some_and(|at| now - at < EVERY)
        {
            return;
        }
        self.host_probe_at = Some(now);
        let targets: Vec<(Uuid, String, u16)> = self
            .vault
            .hosts
            .iter()
            .filter(|h| {
                matches!(
                    h.protocol,
                    Protocol::Ssh | Protocol::Mosh | Protocol::Telnet
                )
            })
            .filter_map(|h| {
                let r = self.vault.resolve(h);
                let direct = r.chain.is_empty() && r.proxy.is_none();
                (direct && !r.address.trim().is_empty()).then(|| (h.id, r.address, r.port))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        self.host_probe_running = true;
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(PARALLEL));
            let mut tasks = tokio::task::JoinSet::new();
            for (host_id, address, port) in targets {
                let (permits, bridge) = (permits.clone(), bridge.clone());
                tasks.spawn(async move {
                    let _slot = permits.acquire().await;
                    let latency_ms = crate::health::probe(&address, port)
                        .await
                        .map(|d| d.as_millis().max(1) as u32);
                    bridge.send(AppEvent::HostLatency {
                        host_id,
                        latency_ms,
                    });
                });
            }
            while tasks.join_next().await.is_some() {}
            bridge.send(AppEvent::HostProbeDone);
        });
    }

    fn host_subtitle(&self, h: &Host) -> String {
        let r = self.vault.resolve(h);
        match h.protocol {
            Protocol::Serial => format!(
                "serial, {} @ {}",
                if h.serial.device.is_empty() {
                    &h.address
                } else {
                    &h.serial.device
                },
                h.serial.baud_rate
            ),
            Protocol::Local => "local shell".into(),
            p => {
                let user = if r.username.is_empty() {
                    String::new()
                } else {
                    format!("{}@", r.username)
                };
                let port = if r.port == p.default_port() {
                    String::new()
                } else {
                    format!(":{}", r.port)
                };
                let chain = if r.chain.is_empty() {
                    String::new()
                } else {
                    format!("  \u{E026} via {}", r.chain.len())
                };
                format!("{user}{}{port}{chain}", h.address)
            }
        }
    }

    pub fn filtered_hosts(&self) -> Vec<Host> {
        filter_hosts(
            &self.vault,
            &self.search,
            self.current_group,
            self.tag_filter.as_deref(),
        )
    }

    fn apply_host_action(&mut self, a: HostAction) {
        if matches!(
            &a,
            HostAction::Connect(_) | HostAction::Sftp(_) | HostAction::Edit(_)
        ) {
            self.host_inspector_open = false;
        }
        match a {
            HostAction::Connect(id) => self.open_host(id),
            HostAction::Sftp(id) => self.open_sftp(Some(id)),
            HostAction::Edit(id) => {
                if let Some(h) = self.vault.host(id).cloned() {
                    self.editor = Some(Editor::Host(Box::new(h), false));
                }
            }
            HostAction::Duplicate(id) => {
                if let Some(mut h) = self.vault.host(id).cloned() {
                    h.id = Uuid::new_v4();
                    h.label = format!("{} copy", h.display_name());
                    h.last_connected = None;
                    h.updated = now_ts();
                    self.editor = Some(Editor::Host(Box::new(h), true));
                }
            }
            HostAction::Delete(ids) => {
                let what = if ids.len() == 1 {
                    self.vault
                        .host(ids[0])
                        .map(|h| h.display_name())
                        .unwrap_or_default()
                } else {
                    format!("{} hosts", ids.len())
                };
                self.dialog = Some(Dialog::ConfirmDelete { ids, what });
            }
            HostAction::OpenGroup(g) => {
                self.current_group = g;
                self.inspected_host = None;
                self.selected_hosts.clear();
            }
            HostAction::EditGroup(id) => {
                if let Some(g) = self.vault.group(id).cloned() {
                    self.editor = Some(Editor::Group(g, false));
                }
            }
            HostAction::DeleteGroup(id) => {
                let what = format!(
                    "group “{}” (hosts are kept)",
                    self.vault
                        .group(id)
                        .map(|g| g.name.clone())
                        .unwrap_or_default()
                );
                self.dialog = Some(Dialog::ConfirmDelete {
                    ids: vec![id],
                    what,
                });
            }
            HostAction::MoveTo(ids, g) => {
                for h in self.vault.hosts.iter_mut().filter(|h| ids.contains(&h.id)) {
                    h.group_id = g;
                    h.updated = now_ts();
                }
                self.selected_hosts.clear();
                self.toast(format!("Moved {} host(s)", ids.len()), false);
                self.mark_dirty();
            }
            HostAction::ConnectMany(ids) => {
                for id in ids {
                    self.open_host(id);
                }
                self.selected_hosts.clear();
            }
            HostAction::TogglePin(id) => {
                if let Some(h) = self.vault.hosts.iter_mut().find(|h| h.id == id) {
                    h.pinned = !h.pinned;
                    h.updated = now_ts();
                }
                self.mark_dirty();
            }
            HostAction::CopyAddress(a) => {
                self.bridge.ctx.copy_text(a);
                self.toast("Address copied", false);
            }
        }
    }

    pub fn import_default_ssh_config(&mut self) {
        let path = directories::UserDirs::new().map(|u| u.home_dir().join(".ssh").join("config"));
        match path.and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => {
                let res = crate::importers::parse_ssh_config(&text, &self.vault);
                let msg = res.apply(&mut self.vault);
                self.toast(msg, false);
                self.mark_dirty();
            }
            None => self.toast("~/.ssh/config not found", true),
        }
    }

    pub fn cloud_import(&mut self, which: u8) {
        self.cloud_busy = true;
        let s = self.vault.settings.clone();
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            let res = match which {
                0 => crate::cloud::digitalocean(&s.do_token).await,
                1 => crate::cloud::aws(&s).await,
                _ => crate::cloud::azure(&s).await,
            };
            bridge.send(AppEvent::CloudImport(res.map_err(|e| format!("{e:#}"))));
        });
        self.toast("Importing from cloud…", false);
        let _ = Page::Hosts;
    }
}

/// One folding group in the library list.
struct LibrarySection {
    group: Option<Uuid>,
    name: String,
    hosts: Vec<Host>,
}

/// Groups `hosts` for display, in vault order, with ungrouped hosts last. Empty groups are
/// left out so a search only shows sections that have matches.
fn library_sections(vault: &Vault, hosts: &[Host]) -> Vec<LibrarySection> {
    let mut sections: Vec<LibrarySection> = vault
        .groups
        .iter()
        .map(|group| LibrarySection {
            group: Some(group.id),
            name: vault.group_path(Some(group.id)),
            hosts: hosts
                .iter()
                .filter(|host| host.group_id == Some(group.id))
                .cloned()
                .collect(),
        })
        .collect();
    sections.push(LibrarySection {
        group: None,
        name: "Ungrouped".into(),
        // A host pointing at a deleted group still has to appear somewhere.
        hosts: hosts
            .iter()
            .filter(|host| host.group_id.is_none_or(|id| vault.group(id).is_none()))
            .cloned()
            .collect(),
    });
    sections.retain(|section| !section.hosts.is_empty());
    sections
}

/// Host ids in on-screen order, top to bottom.
pub fn library_order(vault: &Vault, hosts: &[Host]) -> Vec<Uuid> {
    library_sections(vault, hosts)
        .iter()
        .flat_map(|section| section.hosts.iter().map(|host| host.id))
        .collect()
}

fn probe_note(probe: &str) -> &'static str {
    match probe {
        "Unreachable" => "The TCP probe timed out. You can still try to connect.",
        "Probe skipped" => {
            "This host is behind a jump host or proxy, so a direct probe says nothing."
        }
        "Probes off" => "Turn reachability on in Settings to check ports.",
        "No TCP probe" => "This connection has no network port to check.",
        "Checking…" => "Opening a TCP connection to the port.",
        "Not checked" => "Ferrum has not probed this port yet.",
        _ => "TCP probe. This checks the port, not your login.",
    }
}

fn detail_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    mono: bool,
    pal: crate::themes::Palette,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::hover());
    let p = ui.painter();
    let shown = p.text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        pal.text_dim,
    );
    let clip = egui::Rect::from_min_max(egui::pos2(shown.right() + 12.0, rect.top()), rect.max);
    p.with_clip_rect(clip).text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        value,
        if mono {
            egui::FontId::monospace(13.0)
        } else {
            egui::FontId::proportional(13.0)
        },
        pal.text,
    );
    p.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, pal.border),
    );
    ui.interact(rect, ui.scope_id().with(("detail", label)), Sense::hover())
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{label}: {value}"))
        });
}

/// Search, tag, and group constraints compose; the root library includes every group.
fn filter_hosts(vault: &Vault, query: &str, group: Option<Uuid>, tag: Option<&str>) -> Vec<Host> {
    let query = query.trim().to_lowercase();
    let mut hosts: Vec<_> = vault
        .hosts
        .iter()
        .filter(|host| {
            if tag.is_some_and(|tag| !host.tags.iter().any(|value| value == tag)) {
                return false;
            }
            if group.is_some_and(|group| {
                !vault
                    .group_chain(host.group_id)
                    .iter()
                    .any(|ancestor| ancestor.id == group)
            }) {
                return false;
            }
            if query.is_empty() {
                return true;
            }
            let resolved = vault.resolve(host);
            let text = format!(
                "{} {} {} {} {}",
                host.label,
                host.address,
                resolved.username,
                host.tags.join(" "),
                vault.group_path(host.group_id)
            )
            .to_lowercase();
            query.split_whitespace().all(|word| text.contains(word))
        })
        .cloned()
        .collect();
    hosts.sort_by(|a, b| {
        b.pinned.cmp(&a.pinned).then_with(|| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
        })
    });
    hosts
}

/// Pointer travel before a press on a host card becomes a drag, well past egui's click slop.
const HOST_DRAG_DIST: f32 = 12.0;

/// True on the frame a press that began on this card has moved far enough to count as a drag.
fn host_drag_started(ui: &egui::Ui, resp: &egui::Response, id: egui::Id) -> bool {
    let key = id.with("press");
    if resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
        ui.data_mut(|d| d.insert_temp(key, true));
    }
    if !ui.data(|d| d.get_temp::<bool>(key).unwrap_or(false)) {
        return false;
    }
    let (down, origin, pos) = ui.input(|i| {
        (
            i.pointer.primary_down(),
            i.pointer.press_origin(),
            i.pointer.latest_pos(),
        )
    });
    if !down {
        ui.data_mut(|d| d.remove::<bool>(key));
        return false;
    }
    match (origin, pos) {
        (Some(o), Some(p)) if o.distance(p) > HOST_DRAG_DIST => {
            ui.data_mut(|d| d.remove::<bool>(key));
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn library() -> Vault {
        let production = Group {
            id: Uuid::from_u128(1),
            name: "Production".into(),
            ..Default::default()
        };
        let child = Group {
            id: Uuid::from_u128(2),
            name: "Workers".into(),
            parent_id: Some(production.id),
            ..Default::default()
        };
        let mut api = Host {
            id: Uuid::from_u128(3),
            label: "Alpha API".into(),
            address: "10.0.0.3".into(),
            group_id: Some(production.id),
            tags: vec!["web".into()],
            ..Default::default()
        };
        api.settings.username = Some("deploy".into());
        let mut worker = Host {
            id: Uuid::from_u128(4),
            label: "Beta worker".into(),
            address: "10.0.0.4".into(),
            group_id: Some(child.id),
            tags: vec!["jobs".into()],
            ..Default::default()
        };
        worker.settings.username = Some("deploy".into());
        let home = Host {
            id: Uuid::from_u128(5),
            label: "Home database".into(),
            address: "192.168.1.4".into(),
            tags: vec!["db".into()],
            pinned: true,
            ..Default::default()
        };
        Vault {
            hosts: vec![worker, home, api],
            // unslop-ignore PLACEHOLDER_COPY -- Typed group fixtures for search tests, not product copy.
            groups: vec![production, child],
            ..Default::default()
        }
    }
    fn names(hosts: Vec<Host>) -> Vec<String> {
        hosts.into_iter().map(|host| host.label).collect()
    }
    #[test]
    fn root_library_includes_groups_and_sorts_pinned_hosts_first() {
        assert_eq!(
            names(filter_hosts(&library(), "", None, None)),
            vec!["Home database", "Alpha API", "Beta worker"]
        );
    }
    #[test]
    fn sections_follow_vault_order_with_ungrouped_last_and_skip_empty_groups() {
        let vault = library();
        let hosts = filter_hosts(&vault, "", None, None);
        let sections = library_sections(&vault, &hosts);
        let names: Vec<_> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Production", "Production / Workers", "Ungrouped"]
        );
        assert_eq!(
            sections[0].hosts[0].label, "Alpha API",
            "rows keep the filter's pinned-then-name order"
        );
        let found = filter_hosts(&vault, "home", None, None);
        let only: Vec<_> = library_sections(&vault, &found)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(only, vec!["Ungrouped"]);
    }
    #[test]
    fn a_host_in_a_deleted_group_still_appears_under_ungrouped() {
        let mut vault = library();
        vault.groups.retain(|g| g.id != Uuid::from_u128(2));
        let hosts = filter_hosts(&vault, "worker", None, None);
        let sections = library_sections(&vault, &hosts);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "Ungrouped");
    }
    #[test]
    fn group_search_and_tag_filters_compose_including_descendants() {
        let vault = library();
        let production = Some(Uuid::from_u128(1));
        assert_eq!(
            names(filter_hosts(&vault, "DEPLOY production", production, None)),
            vec!["Alpha API", "Beta worker"]
        );
        assert_eq!(
            names(filter_hosts(&vault, "deploy", production, Some("jobs"))),
            vec!["Beta worker"]
        );
        assert!(filter_hosts(&vault, "database", production, None).is_empty());
        assert!(filter_hosts(&vault, "worker", production, Some("web")).is_empty());
    }
}
