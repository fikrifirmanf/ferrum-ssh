//! Hosts page: groups, hosts, search / quick connect, tags, bulk actions, drag & drop.

use std::collections::HashSet;

use egui::{RichText, Sense, Stroke, Vec2};
use uuid::Uuid;

use super::{ghost_button, primary_button};
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
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Hosts").size(tokens::TITLE).strong());
                if !short_window {
                    ui.label(
                        RichText::new(format!(
                            "{} saved hosts · {} groups",
                            self.vault.hosts.len(),
                            self.vault.groups.len()
                        ))
                        .small()
                        .color(pal.text_dim),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary_button("\u{E3D4}  New host", pal)).clicked() {
                    self.editor = Some(Editor::Host(
                        Box::new(Host {
                            group_id: self.current_group,
                            ..Default::default()
                        }),
                        true,
                    ));
                }
                ui.menu_button("\u{E1FE}", |ui| self.host_tools_menu(ui))
                    .response
                    .on_hover_text("Host actions and import");
                if ui.add(ghost_button("\u{E20C}  Import")).clicked() {
                    self.dialog = Some(Dialog::Import { error: None });
                }
            });
        });
        ui.add_space(if short_window { 8.0 } else { 16.0 });
        if !short_window {
            ui.label(
                RichText::new("Search saved hosts")
                    .small()
                    .color(pal.text_dim),
            );
        }
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search hosts, addresses, or tags")
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(12, 8)),
        );
        search.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Search saved hosts")
        });
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
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(self.current_group.is_none(), "All hosts")
                .clicked()
            {
                action = Some(HostAction::OpenGroup(None));
            }
            let groups = self.vault.groups.clone();
            for g in &groups {
                let response = ui
                    .selectable_label(self.current_group == Some(g.id), &g.name)
                    .on_hover_text(self.vault.group_path(Some(g.id)));
                if response.clicked() {
                    action = Some(HostAction::OpenGroup(Some(g.id)));
                }
                if response.dnd_hover_payload::<Uuid>().is_some() {
                    ui.painter().rect_stroke(
                        response.rect,
                        4,
                        Stroke::new(2.0, pal.accent),
                        egui::StrokeKind::Inside,
                    );
                }
                if let Some(payload) = response.dnd_release_payload::<Uuid>() {
                    let ids = if self.selected_hosts.contains(&payload) {
                        self.selected_hosts.iter().copied().collect()
                    } else {
                        vec![*payload]
                    };
                    action = Some(HostAction::MoveTo(ids, Some(g.id)));
                }
                response.context_menu(|ui| {
                    ui.label(format!("{} hosts", self.count_in_group(g.id)));
                    if ui.button("Edit group settings").clicked() {
                        action = Some(HostAction::EditGroup(g.id));
                        ui.close();
                    }
                    if ui.button("Connect to group").clicked() {
                        action = Some(HostAction::ConnectMany(
                            self.vault
                                .hosts
                                .iter()
                                .filter(|h| {
                                    self.vault
                                        .group_chain(h.group_id)
                                        .iter()
                                        .any(|parent| parent.id == g.id)
                                })
                                .map(|h| h.id)
                                .collect(),
                        ));
                        ui.close();
                    }
                    if ui.button("Delete group").clicked() {
                        action = Some(HostAction::DeleteGroup(g.id));
                        ui.close();
                    }
                });
            }
            ui.menu_button("\u{E2F0}  View", |ui| {
                let before = self.vault.settings.clone();
                ui.checkbox(&mut self.vault.settings.hosts_grid_view, "Grid view");
                ui.checkbox(&mut self.vault.settings.hosts_compact, "Compact rows");
                ui.separator();
                ui.label("Filter by tag");
                if ui
                    .selectable_label(self.tag_filter.is_none(), "Any tag")
                    .clicked()
                {
                    self.tag_filter = None;
                }
                for tag in self.vault.all_tags() {
                    if ui
                        .selectable_label(self.tag_filter.as_ref() == Some(&tag), &tag)
                        .clicked()
                    {
                        self.tag_filter = Some(tag);
                    }
                }
                if before != self.vault.settings {
                    crate::store::touch_settings(&mut self.vault);
                    self.mark_dirty();
                }
            });
            if let Some(tag) = self.tag_filter.clone() {
                if ui.small_button(format!("#{tag} ×")).clicked() {
                    self.tag_filter = None;
                }
            }
        });
        ui.add_space(8.0);
        if !self.selected_hosts.is_empty() {
            let selected: Vec<_> = self.selected_hosts.iter().copied().collect();
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("{} selected", selected.len()));
                if ui.button("Connect all").clicked() {
                    action = Some(HostAction::ConnectMany(selected.clone()));
                }
                ui.menu_button("Move to group", |ui| {
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
                if ui.add(super::danger_button("Delete hosts", pal)).clicked() {
                    action = Some(HostAction::Delete(selected.clone()));
                }
                if ui.button("Clear selection").clicked() {
                    self.selected_hosts.clear();
                }
            });
            ui.add_space(8.0);
        }
        self.schedule_host_probes(ui.input(|i| i.time));
        let hosts = self.filtered_hosts();
        if !hosts.iter().any(|h| Some(h.id) == self.inspected_host) {
            self.inspected_host = hosts.first().map(|h| h.id);
        }
        let connected = self.connected_hosts();
        let grid = self.vault.settings.hosts_grid_view;
        let width = ui.available_width();
        if !grid {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::hover());
            ui.painter().text(
                rect.left_center() + Vec2::new(12.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "HOST / ADDRESS",
                egui::FontId::proportional(tokens::CAPTION),
                pal.text_dim,
            );
            if width > 560.0 {
                ui.painter().text(
                    rect.right_center() - Vec2::new(220.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    "GROUP",
                    egui::FontId::proportional(tokens::CAPTION),
                    pal.text_dim,
                );
            }
            ui.painter().text(
                rect.right_center() - Vec2::new(12.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                "TCP PROBE",
                egui::FontId::proportional(tokens::CAPTION),
                pal.text_dim,
            );
            ui.painter()
                .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, pal.border));
        }
        let area_height = (ui.available_height() - 36.0).max(80.0);
        let mut focus_next = None;
        egui::ScrollArea::vertical()
            .id_salt("host-library")
            .max_height(area_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if hosts.is_empty() {
                    ui.add_space(32.0);
                    let filtered = !self.search.is_empty()
                        || self.tag_filter.is_some()
                        || self.current_group.is_some();
                    ui.label(
                        RichText::new(if filtered {
                            "No matching hosts"
                        } else {
                            "No hosts yet"
                        })
                        .size(16.0)
                        .strong(),
                    );
                    ui.label(
                        RichText::new(if filtered {
                            "Try another name or clear the filters."
                        } else {
                            "Add a host to save its address and login."
                        })
                        .color(pal.text_dim),
                    );
                    ui.add_space(12.0);
                    ui.horizontal_wrapped(|ui| {
                        if filtered && ui.button("Clear filters").clicked() {
                            self.search.clear();
                            self.tag_filter = None;
                            self.current_group = None;
                        }
                        if ui.add(primary_button("Add host", pal)).clicked() {
                            self.editor = Some(Editor::Host(
                                Box::new(Host {
                                    group_id: self.current_group,
                                    ..Default::default()
                                }),
                                true,
                            ));
                        }
                        if ui.button("Import hosts").clicked() {
                            self.dialog = Some(Dialog::Import { error: None });
                        }
                        if ui.button("Quick connect").clicked() {
                            self.dialog = Some(Dialog::QuickConnect {
                                input: String::new(),
                                error: None,
                            });
                        }
                    });
                    return;
                }
                let columns = if grid {
                    ((width + 16.0) / 296.0).floor().max(1.0) as usize
                } else {
                    1
                };
                let row_width = (width - 16.0 * (columns - 1) as f32) / columns as f32;
                let row_height = if grid {
                    128.0
                } else if self.vault.settings.hosts_compact {
                    tokens::COMPACT_ROW
                } else {
                    tokens::ROW
                };
                egui::Grid::new("workbench-hosts")
                    .num_columns(columns)
                    .spacing(Vec2::new(
                        if grid { 16.0 } else { 0.0 },
                        if grid { 16.0 } else { 0.0 },
                    ))
                    .show(ui, |ui| {
                        for (index, h) in hosts.iter().enumerate() {
                            let id = egui::Id::unique(("host-row", h.id));
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::new(row_width, row_height),
                                Sense::hover(),
                            );
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
                            let selected = Some(h.id) == self.inspected_host
                                || self.selected_hosts.contains(&h.id);
                            let p = ui.painter();
                            if selected || response.hovered() {
                                p.rect_filled(
                                    rect,
                                    if grid { 8 } else { 0 },
                                    if selected {
                                        pal.accent_soft
                                    } else {
                                        pal.card_hover
                                    },
                                );
                            }
                            p.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, pal.border));
                            if selected {
                                p.vline(
                                    rect.left(),
                                    rect.y_range().shrink(8.0),
                                    Stroke::new(2.0, pal.accent),
                                );
                            }
                            if response.has_focus() {
                                p.rect_stroke(
                                    rect.shrink(2.0),
                                    4,
                                    Stroke::new(2.0, pal.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            let icon_rect = egui::Rect::from_center_size(
                                egui::pos2(
                                    rect.left() + 28.0,
                                    if grid {
                                        rect.top() + 36.0
                                    } else {
                                        rect.center().y
                                    },
                                ),
                                Vec2::splat(32.0),
                            );
                            p.rect_stroke(
                                icon_rect,
                                4,
                                Stroke::new(1.0, pal.border),
                                egui::StrokeKind::Inside,
                            );
                            p.text(
                                icon_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                if h.protocol == Protocol::Serial {
                                    crate::icons::USB
                                } else {
                                    crate::icons::HARD_DRIVES
                                },
                                egui::FontId::proportional(16.0),
                                pal.text_dim,
                            );
                            let tx = rect.left() + 56.0;
                            let right = if grid {
                                rect.right() - 12.0
                            } else {
                                rect.right() - if row_width > 560.0 { 232.0 } else { 112.0 }
                            };
                            let text = p.with_clip_rect(egui::Rect::from_min_max(
                                egui::pos2(tx, rect.top()),
                                egui::pos2(right.max(tx), rect.bottom()),
                            ));
                            let cy = if grid {
                                rect.top() + 36.0
                            } else {
                                rect.center().y
                            };
                            text.text(
                                egui::pos2(tx, cy - 10.0),
                                egui::Align2::LEFT_CENTER,
                                format!(
                                    "{}{}  {}",
                                    if h.pinned { "\u{E46A}  " } else { "" },
                                    h.display_name(),
                                    h.protocol.label()
                                ),
                                egui::FontId::proportional(tokens::BODY),
                                pal.text,
                            );
                            text.text(
                                egui::pos2(tx, cy + 10.0),
                                egui::Align2::LEFT_CENTER,
                                self.host_subtitle(h),
                                egui::FontId::monospace(tokens::CAPTION),
                                pal.text_dim,
                            );
                            let (probe, color) = self.host_probe_label(h);
                            if grid {
                                p.text(
                                    rect.left_bottom() + Vec2::new(12.0, -20.0),
                                    egui::Align2::LEFT_CENTER,
                                    self.host_group_label(h.group_id),
                                    egui::FontId::proportional(tokens::CAPTION),
                                    pal.text_dim,
                                );
                            } else if row_width > 560.0 {
                                let group_rect = egui::Rect::from_min_max(
                                    egui::pos2(rect.right() - 220.0, rect.top()),
                                    egui::pos2(rect.right() - 116.0, rect.bottom()),
                                );
                                p.with_clip_rect(group_rect).text(
                                    group_rect.left_center(),
                                    egui::Align2::LEFT_CENTER,
                                    self.host_group_label(h.group_id),
                                    egui::FontId::proportional(tokens::CAPTION),
                                    pal.text_dim,
                                );
                            }
                            p.text(
                                egui::pos2(
                                    rect.right() - 12.0,
                                    if grid { rect.bottom() - 20.0 } else { cy },
                                ),
                                egui::Align2::RIGHT_CENTER,
                                probe,
                                egui::FontId::proportional(tokens::CAPTION),
                                color,
                            );
                            if connected.contains(&h.id) {
                                p.circle_filled(icon_rect.right_bottom(), 3.0, pal.success);
                            }
                            if host_drag_started(ui, &response, id) {
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
                                    self.host_inspector_open =
                                        ui.ctx().content_rect().width() < 1200.0;
                                    response.request_focus();
                                }
                            }
                            if response.double_clicked() {
                                action = Some(HostAction::Connect(h.id));
                            }
                            if response.has_focus() && !self.palette.open && self.dialog.is_none() {
                                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    action = Some(HostAction::Connect(h.id));
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
                                    focus_next = Some(
                                        hosts[(index as isize + direction)
                                            .clamp(0, hosts.len() as isize - 1)
                                            as usize]
                                            .id,
                                    );
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
                                if ui.button("Connect to host").clicked() {
                                    action = Some(HostAction::Connect(h.id));
                                    ui.close();
                                }
                                if h.protocol == Protocol::Ssh && ui.button("Open SFTP").clicked() {
                                    action = Some(HostAction::Sftp(h.id));
                                    ui.close();
                                }
                                if ui.button("Edit host").clicked() {
                                    action = Some(HostAction::Edit(h.id));
                                    ui.close();
                                }
                                if ui.button("Duplicate host").clicked() {
                                    action = Some(HostAction::Duplicate(h.id));
                                    ui.close();
                                }
                                if ui
                                    .button(if h.pinned { "Unpin host" } else { "Pin host" })
                                    .clicked()
                                {
                                    action = Some(HostAction::TogglePin(h.id));
                                    ui.close();
                                }
                                ui.menu_button("Move to group", |ui| {
                                    if ui.button("No group").clicked() {
                                        action = Some(HostAction::MoveTo(vec![h.id], None));
                                        ui.close();
                                    }
                                    for g in &self.vault.groups {
                                        if ui.button(self.vault.group_path(Some(g.id))).clicked() {
                                            action =
                                                Some(HostAction::MoveTo(vec![h.id], Some(g.id)));
                                            ui.close();
                                        }
                                    }
                                });
                                if ui.button("Copy address").clicked() {
                                    action = Some(HostAction::CopyAddress(h.address.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.add(super::danger_button("Delete host", pal)).clicked() {
                                    action = Some(HostAction::Delete(vec![h.id]));
                                    ui.close();
                                }
                            });
                            if (index + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        if let Some(id) = focus_next {
            self.inspected_host = Some(id);
            ui.memory_mut(|m| m.request_focus(egui::Id::unique(("host-row", id))));
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "{} of {} hosts · TCP reachability is separate from session state.",
                hosts.len(),
                self.vault.hosts.len()
            ))
            .small()
            .color(pal.text_dim),
        );
        if let Some(action) = action {
            self.apply_host_action(action);
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
        egui::ScrollArea::vertical()
            .id_salt("inspector-details")
            .max_height((ui.available_height() - 148.0).max(80.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(RichText::new("HOST DETAILS").small().color(pal.text_dim));
                ui.add_space(24.0);
                super::os_avatar(
                    ui,
                    if host.protocol == Protocol::Serial {
                        crate::icons::USB
                    } else {
                        crate::icons::HARD_DRIVES
                    },
                    pal.accent,
                    48.0,
                );
                ui.add_space(8.0);
                ui.label(RichText::new(host.display_name()).size(16.0).strong());
                ui.label(
                    RichText::new(&host.address)
                        .monospace()
                        .small()
                        .color(pal.text_dim),
                );
                ui.label(
                    RichText::new(host.os.as_deref().unwrap_or("OS detected after SSH login"))
                        .small()
                        .color(pal.text_dim),
                );
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                let auth = if let Some(key) = &target.key {
                    key.label.clone()
                } else if target.use_agent {
                    "SSH agent".into()
                } else if target.password.is_some() {
                    "Saved password".into()
                } else {
                    "Ask when connecting".into()
                };
                let route = if !target.chain.is_empty() {
                    format!("Via {} jump hosts", target.chain.len())
                } else if target.proxy.is_some() {
                    "Via proxy".into()
                } else {
                    "Direct connection".into()
                };
                for (label, value) in [
                    ("Protocol", host.protocol.label().to_string()),
                    (
                        "Port",
                        if matches!(host.protocol, Protocol::Serial | Protocol::Local) {
                            "Not applicable".into()
                        } else {
                            target.port.to_string()
                        },
                    ),
                    (
                        "Username",
                        if target.username.is_empty() {
                            "Not set".into()
                        } else {
                            target.username.clone()
                        },
                    ),
                    (
                        "Authentication",
                        if host.protocol == Protocol::Serial {
                            "Not applicable".into()
                        } else {
                            auth
                        },
                    ),
                    (
                        "Route",
                        if host.protocol == Protocol::Serial {
                            format!("{} baud", host.serial.baud_rate)
                        } else {
                            route
                        },
                    ),
                    ("Group", self.host_group_label(host.group_id)),
                ] {
                    ui.label(RichText::new(label).small().color(pal.text_dim));
                    ui.label(RichText::new(value).size(14.0));
                    ui.add_space(4.0);
                }
                let (probe, color) = self.host_probe_label(&host);
                ui.label(RichText::new("TCP probe").small().color(pal.text_dim));
                ui.label(RichText::new(probe).color(color));
                if self.connected_hosts().contains(&host.id) {
                    ui.label(
                        RichText::new("\u{E184}  Session connected")
                            .small()
                            .color(pal.success),
                    );
                }
                if !host.tags.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            host.tags
                                .iter()
                                .map(|t| format!("#{t}"))
                                .collect::<Vec<_>>()
                                .join(" "),
                        )
                        .small()
                        .color(pal.text_dim),
                    );
                }
                if !host.notes.trim().is_empty() {
                    ui.separator();
                    ui.label(&host.notes);
                }
            });
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width());
            if ui
                .add_sized(
                    [ui.available_width(), 36.0],
                    primary_button("\u{E3D0}  Connect to host", pal),
                )
                .clicked()
            {
                action = Some(HostAction::Connect(host.id));
            }
            if ui
                .add_sized(
                    [ui.available_width(), 36.0],
                    egui::Button::new("\u{E3B4}  Edit host"),
                )
                .clicked()
            {
                action = Some(HostAction::Edit(host.id));
            }
            if host.protocol == Protocol::Ssh && ui.button("\u{E0A0}  Open SFTP").clicked() {
                action = Some(HostAction::Sftp(host.id));
            }
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
        ui.set_min_width(230.0);
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
                ports: crate::conn::serial::list_ports(),
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
