//! Hosts page: groups, hosts, search / quick connect, tags, bulk actions, drag & drop.

use std::collections::HashSet;

use egui::{CornerRadius, RichText, Sense, Stroke, Vec2};
use uuid::Uuid;

use super::{card_frame, ghost_button, pill, primary_button};
use crate::app::{App, Dialog, Editor, Page, Tab};
use crate::conn::AppEvent;
use crate::model::{Group, Host, Protocol, now_ts};
use crate::themes::os_color;

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
        let pal = self.pal();
        let mut action: Option<HostAction> = None;

        // ---------- Header ----------
        ui.label(RichText::new("Hosts").size(28.0).strong());
        ui.label(RichText::new("Your servers, one connection away.").color(pal.text_dim));
        ui.add_space(16.0);
        ui.vertical(|ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("\u{E30C}  Search hosts or connect with user@host:port")
                    .desired_width(ui.available_width())
                    .margin(egui::Margin::symmetric(12, 8)),
            );
            if search.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !self.search.trim().is_empty()
            {
                let q = self.search.trim().to_string();
                let matches: Vec<Uuid> = self.filtered_hosts().iter().map(|h| h.id).collect();
                if matches.len() == 1 {
                    action = Some(HostAction::Connect(matches[0]));
                } else if crate::model::parse_quick_connect(&q).is_some() {
                    self.open_quick_connect(&q);
                    self.search.clear();
                }
            }
            let show_terminal = ui.available_width() > 360.0;
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new("\u{E1FE}").size(18.0), |ui| {
                        ui.set_min_width(230.0);
                        if ui.button("\u{E25A}  New group").clicked() {
                            let g = Group {
                                parent_id: self.current_group,
                                ..Default::default()
                            };
                            self.editor = Some(Editor::Group(g, true));
                            ui.close();
                        }
                        if ui
                            .button(
                                "\u{E20C}  Import (ssh config, PuTTY, MobaXterm, SecureCRT, CSV)…",
                            )
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
                                let r = std::fs::write(
                                    &p,
                                    crate::importers::export_csv(&self.vault, false),
                                );
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
                            if let Some(p) =
                                rfd::FileDialog::new().set_file_name("config").save_file()
                            {
                                let r = std::fs::write(
                                    &p,
                                    crate::importers::export_ssh_config(&self.vault),
                                );
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
                    });
                    let grid = self.vault.settings.hosts_grid_view;
                    if ui
                        .add(ghost_button(if grid { "\u{E2F0}" } else { "\u{E464}" }))
                        .on_hover_text("Toggle grid / list")
                        .clicked()
                    {
                        self.vault.settings.hosts_grid_view = !grid;
                        self.mark_dirty();
                    }
                    if show_terminal
                        && ui
                            .button("\u{E2D8}  Terminal")
                            .on_hover_text("Open a local terminal (⌘T)")
                            .clicked()
                    {
                        self.open_local();
                    }
                    if ui.add(primary_button("+  New Host", pal)).clicked() {
                        let h = Host {
                            group_id: self.current_group,
                            ..Default::default()
                        };
                        self.editor = Some(Editor::Host(Box::new(h), true));
                    }
                });
            });
        });
        ui.add_space(10.0);

        // ---------- Breadcrumb + tags ----------
        ui.horizontal_wrapped(|ui| {
            if ui.link(RichText::new("All hosts").strong()).clicked() {
                action = Some(HostAction::OpenGroup(None));
            }
            let mut chain = self.vault.group_chain(self.current_group);
            chain.reverse();
            for g in chain {
                ui.label(RichText::new("›").color(pal.text_dim));
                if ui.link(&g.name).clicked() {
                    action = Some(HostAction::OpenGroup(Some(g.id)));
                }
            }
            let tags = self.vault.all_tags();
            if !tags.is_empty() {
                ui.add_space(16.0);
                for t in tags {
                    let active = self.tag_filter.as_deref() == Some(t.as_str());
                    if pill(
                        ui,
                        &format!("#{t}"),
                        if active { pal.accent } else { pal.text_dim },
                    )
                    .clicked()
                    {
                        self.tag_filter = if active { None } else { Some(t) };
                    }
                }
            }
        });
        ui.add_space(8.0);

        // ---------- Bulk bar ----------
        if !self.selected_hosts.is_empty() {
            let sel: Vec<Uuid> = self.selected_hosts.iter().copied().collect();
            card_frame(pal).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} selected", sel.len())).strong());
                    if ui.button("\u{E3D0}  Connect all").clicked() {
                        action = Some(HostAction::ConnectMany(sel.clone()));
                    }
                    ui.menu_button("\u{E25A}  Move to group", |ui| {
                        if ui.button("(No group)").clicked() {
                            action = Some(HostAction::MoveTo(sel.clone(), None));
                            ui.close();
                        }
                        for g in &self.vault.groups {
                            if ui.button(self.vault.group_path(Some(g.id))).clicked() {
                                action = Some(HostAction::MoveTo(sel.clone(), Some(g.id)));
                                ui.close();
                            }
                        }
                    });
                    if ui.add(super::danger_button("Delete", pal)).clicked() {
                        action = Some(HostAction::Delete(sel.clone()));
                    }
                    if ui.button("Clear selection").clicked() {
                        self.selected_hosts.clear();
                    }
                });
            });
            ui.add_space(8.0);
        }

        self.schedule_host_probes(ui.input(|i| i.time));
        let connected = self.connected_hosts();
        let searching = !self.search.trim().is_empty() || self.tag_filter.is_some();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // ---------- Recents ----------
            if !searching && self.current_group.is_none() {
                let mut recent: Vec<&Host> = self.vault.hosts.iter().filter(|h| h.last_connected.is_some()).collect();
                recent.sort_by_key(|h| std::cmp::Reverse(h.last_connected));
                recent.truncate(4);
                if !recent.is_empty() {
                    super::section_title(ui, "Recent", pal);
                    ui.horizontal_wrapped(|ui| {
                        for h in recent {
                            let resp = egui::Frame::new()
                                .fill(pal.card)
                                .stroke(Stroke::new(1.0, pal.border))
                                .corner_radius(CornerRadius::same(10))
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        super::os_avatar(ui, crate::icons::operating_system(h.os.as_deref()).0, os_color(h.os.as_deref()), 24.0);
                                        ui.label(&h.display_name());
                                    });
                                })
                                .response
                                .interact(Sense::click());
                            if resp.clicked() {
                                action = Some(HostAction::Connect(h.id));
                            }
                        }
                    });
                    ui.add_space(10.0);
                }
            }

            // ---------- Groups ----------
            if !searching {
                let groups: Vec<Group> = self.vault.groups.iter().filter(|g| g.parent_id == self.current_group).cloned().collect();
                if !groups.is_empty() {
                    super::section_title(ui, "Groups", pal);
                    ui.horizontal_wrapped(|ui| {
                        for g in &groups {
                            let count = self.count_in_group(g.id);
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(220.0, 64.0), Sense::click());
                            let hovered = resp.hovered();
                            let dropping = resp.dnd_hover_payload::<Uuid>().is_some();
                            ui.painter().rect(
                                rect,
                                CornerRadius::same(12),
                                if hovered || dropping { pal.card_hover } else { pal.card },
                                Stroke::new(1.0, if dropping { pal.accent } else { pal.border }),
                                egui::StrokeKind::Inside,
                            );
                            let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(12.0, 14.0), Vec2::splat(36.0));
                            ui.painter().rect_filled(icon_rect, CornerRadius::same(10), pal.accent_soft);
                            ui.painter().text(icon_rect.center(), egui::Align2::CENTER_CENTER, "\u{E25A}", egui::FontId::proportional(18.0), pal.accent);
                            ui.painter().text(
                                rect.min + Vec2::new(60.0, 16.0),
                                egui::Align2::LEFT_TOP,
                                &g.name,
                                egui::FontId::proportional(14.5),
                                pal.text,
                            );
                            ui.painter().text(
                                rect.min + Vec2::new(60.0, 36.0),
                                egui::Align2::LEFT_TOP,
                                format!("{count} {}", if count == 1 { "host" } else { "hosts" }),
                                egui::FontId::proportional(12.0),
                                pal.text_dim,
                            );
                            if let Some(payload) = resp.dnd_release_payload::<Uuid>() {
                                let ids = if self.selected_hosts.contains(&payload) {
                                    self.selected_hosts.iter().copied().collect()
                                } else {
                                    vec![*payload]
                                };
                                action = Some(HostAction::MoveTo(ids, Some(g.id)));
                            }
                            if resp.clicked() {
                                action = Some(HostAction::OpenGroup(Some(g.id)));
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Open").clicked() {
                                    action = Some(HostAction::OpenGroup(Some(g.id)));
                                    ui.close();
                                }
                                if ui.button("Edit group settings").clicked() {
                                    action = Some(HostAction::EditGroup(g.id));
                                    ui.close();
                                }
                                let ids: Vec<Uuid> = self.vault.hosts.iter().filter(|h| h.group_id == Some(g.id)).map(|h| h.id).collect();
                                if ui.button("Connect to all hosts").clicked() {
                                    action = Some(HostAction::ConnectMany(ids));
                                    ui.close();
                                }
                                if ui.button("Delete group").clicked() {
                                    action = Some(HostAction::DeleteGroup(g.id));
                                    ui.close();
                                }
                            });
                        }
                    });
                    ui.add_space(12.0);
                }
            }

            // ---------- Hosts ----------
            let hosts = self.filtered_hosts();
            super::section_title(ui, &format!("Hosts · {}", hosts.len()), pal);
            if hosts.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("\u{E32E}").size(42.0).color(pal.text_dim));
                    ui.label(RichText::new(if searching { "No hosts match your search" } else { "No hosts here yet" }).size(16.0));
                    ui.label(
                        RichText::new("Add a host, import your ~/.ssh/config, or type user@host above to connect instantly.")
                            .color(pal.text_dim),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 330.0).max(0.0) / 2.0);
                        if ui.add(primary_button("+  New Host", pal)).clicked() {
                            let h = Host { group_id: self.current_group, ..Default::default() };
                            self.editor = Some(Editor::Host(Box::new(h), true));
                        }
                        if ui.button("Import ~/.ssh/config").clicked() {
                            self.import_default_ssh_config();
                        }
                    });
                });
            }
            let grid = self.vault.settings.hosts_grid_view;
            let columns = ((ui.available_width() + 16.0) / 296.0).floor().max(1.0);
            let card_width = (ui.available_width() - (columns - 1.0) * 16.0) / columns;
            let mut layout = |ui: &mut egui::Ui| {
                for (index, h) in hosts.iter().enumerate() {
                    let selected = self.selected_hosts.contains(&h.id);
                    let live = connected.contains(&h.id);
                    let status = if self.vault.settings.show_host_status {
                        self.host_status.get(&h.id).copied()
                    } else {
                        None
                    };
                    let subtitle = self.host_subtitle(h);
                    let subtitle = match status {
                        Some(Some(ms)) => format!("{subtitle} · {ms} ms"),
                        Some(None) => format!("{subtitle} · not reachable"),
                        None => subtitle,
                    };
                    let id = egui::Id::new(("host-card", h.id));
                    let size = if grid { Vec2::new(card_width, 132.0) } else { Vec2::new(ui.available_width(), 72.0) };
                    // Cards only sense clicks; dragging into a group is detected by hand below.
                    // `dnd_drag_source` or a drag sense lets egui turn slow or slightly
                    // shaky clicks into drags, so the card would never open or connect.
                    let inner = ui.scope(|ui| {
                        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                        let hovered = ui.rect_contains_pointer(rect);
                        let p = ui.painter();
                        p.rect(
                            rect,
                            CornerRadius::same(16),
                            if hovered { pal.card_hover } else { pal.card },
                            Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { pal.accent } else { pal.border }),
                            egui::StrokeKind::Inside,
                        );
                        let av = if grid { 44.0 } else { 40.0 };
                        let av_cy = if grid { rect.top() + 16.0 + av / 2.0 } else { rect.center().y };
                        let av_rect = egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, av_cy - av / 2.0), Vec2::splat(av));
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(av_rect));
                        let (symbol, _) = crate::icons::operating_system(h.os.as_deref());
                        super::os_avatar(&mut child, if h.protocol == Protocol::Serial { crate::icons::USB } else { symbol }, os_color(h.os.as_deref()), av)
                            .on_hover_text(h.os.as_deref().unwrap_or("OS detected after SSH login"));
                        let tx = rect.left() + 24.0 + av;
                        let text_right = if !grid { rect.right() - 116.0 } else { rect.right() - 16.0 };
                        let text_painter = ui.painter().with_clip_rect(egui::Rect::from_min_max(egui::pos2(tx, rect.top()), egui::pos2(text_right, rect.bottom())));
                        let p = ui.painter();
                        let title_y = rect.top() + 18.0;
                        let mut title = h.display_name();
                        if h.pinned {
                            title = format!("\u{E46A} {title}");
                        }
                        text_painter.text(egui::pos2(tx, title_y), egui::Align2::LEFT_TOP, title, egui::FontId::proportional(16.0), pal.text);
                        text_painter.text(
                            egui::pos2(tx, title_y + 24.0),
                            egui::Align2::LEFT_TOP,
                            &subtitle,
                            egui::FontId::proportional(12.0),
                            pal.text_dim,
                        );
                        if live {
                            p.circle_filled(egui::pos2(av_rect.right() - 2.0, av_rect.bottom() - 2.0), 5.0, pal.success);
                            p.circle_stroke(egui::pos2(av_rect.right() - 2.0, av_rect.bottom() - 2.0), 5.0, Stroke::new(2.0, pal.card));
                        }
                        if let Some(latency) = status {
                            let color = match latency {
                                Some(ms) if ms < 200 => pal.success,
                                Some(_) => pal.warning,
                                None => pal.danger,
                            };
                            let center = egui::pos2(av_rect.right() - 2.0, av_rect.top() + 2.0);
                            p.circle_filled(center, 5.0, color);
                            p.circle_stroke(center, 5.0, Stroke::new(2.0, pal.card));
                        }
                        if grid {
                            // Chips: tags, or the protocol when there are none.
                            let labels: Vec<String> = if h.tags.is_empty() {
                                vec![h.protocol.label().to_string()]
                            } else {
                                h.tags.iter().take(3).map(|t| format!("#{t}")).collect()
                            };
                            let mut cx = rect.left() + 16.0;
                            let cy = rect.bottom() - 24.0;
                            for label in labels {
                                let galley = p.layout_no_wrap(label, egui::FontId::proportional(12.0), pal.text_dim);
                                let chip = egui::Rect::from_min_size(
                                    egui::pos2(cx, cy - 10.0),
                                    Vec2::new(galley.size().x + 14.0, 20.0),
                                );
                                if chip.right() > rect.right() - 116.0 {
                                    break;
                                }
                                p.rect_filled(chip, CornerRadius::same(6), pal.bg);
                                p.galley(egui::pos2(chip.left() + 7.0, cy - galley.size().y / 2.0), galley, pal.text_dim);
                                cx = chip.right() + 6.0;
                            }
                        }
                        if !grid && !h.tags.is_empty() && rect.width() > 620.0 {
                            p.text(
                                egui::pos2(rect.right() - 120.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                h.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join("  "),
                                egui::FontId::proportional(11.5),
                                pal.accent,
                            );
                        }
                        // Hover action buttons.
                        if hovered {
                            let mut x = rect.right() - 12.0;
                            for (icon, tip, act) in [
                                ("\u{E3B4}", "Edit", 2u8),
                                (if h.protocol == Protocol::Ssh { "\u{E098}" } else { "" }, "SFTP", 1),
                                ("\u{E3D0}", "Connect", 0),
                            ] {
                                if icon.is_empty() {
                                    continue;
                                }
                                let br_y = if grid { rect.bottom() - 24.0 } else { rect.center().y };
                                let br = egui::Rect::from_center_size(egui::pos2(x - 13.0, br_y), Vec2::splat(26.0));
                                let bresp = ui.interact(br, id.with(act), Sense::click()).on_hover_text(tip);
                                ui.painter().rect_filled(
                                    br,
                                    CornerRadius::same(7),
                                    if bresp.hovered() { pal.accent } else { pal.accent_soft },
                                );
                                ui.painter().text(
                                    br.center(),
                                    egui::Align2::CENTER_CENTER,
                                    icon,
                                    egui::FontId::proportional(13.0),
                                    if bresp.hovered() { pal.on_accent } else { pal.accent },
                                );
                                if bresp.clicked() {
                                    action = Some(match act {
                                        0 => HostAction::Connect(h.id),
                                        1 => HostAction::Sftp(h.id),
                                        _ => HostAction::Edit(h.id),
                                    });
                                }
                                x -= 32.0;
                            }
                        }
                        resp
                    });
                    let resp = inner.inner;
                    if host_drag_started(ui, &resp, id) {
                        egui::DragAndDrop::set_payload(ui.ctx(), h.id);
                    }
                    if egui::DragAndDrop::payload::<Uuid>(ui.ctx()).is_some_and(|p| *p == h.id) {
                        if let Some(pos) = ui.ctx().pointer_interact_pos() {
                            egui::Area::new(id.with("drag"))
                                .order(egui::Order::Tooltip)
                                .fixed_pos(pos + Vec2::new(12.0, 12.0))
                                .interactable(false)
                                .show(ui.ctx(), |ui| {
                                    card_frame(pal).show(ui, |ui| ui.label(h.display_name()));
                                });
                        }
                    }
                    if resp.double_clicked() {
                        ui.data_mut(|d| d.remove::<(Uuid, f64)>(pending_edit_id()));
                        action = Some(HostAction::Connect(h.id));
                    } else if resp.clicked() {
                        let multi = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                        if multi {
                            if !self.selected_hosts.insert(h.id) {
                                self.selected_hosts.remove(&h.id);
                            }
                        } else if self.selected_hosts.is_empty() {
                            // Opening the editor reshapes the page, which would eat the second
                            // click of a double-click, so wait until it can't be one.
                            let now = ui.input(|i| i.time);
                            ui.data_mut(|d| d.insert_temp(pending_edit_id(), (h.id, now)));
                        } else {
                            self.selected_hosts.clear();
                        }
                    }
                    resp.context_menu(|ui| {
                        ui.set_min_width(190.0);
                        if ui.button("\u{E3D0}  Connect").clicked() {
                            action = Some(HostAction::Connect(h.id));
                            ui.close();
                        }
                        if h.protocol == Protocol::Ssh && ui.button("\u{E098}  Open SFTP").clicked() {
                            action = Some(HostAction::Sftp(h.id));
                            ui.close();
                        }
                        if ui.button("\u{E3B4}  Edit").clicked() {
                            action = Some(HostAction::Edit(h.id));
                            ui.close();
                        }
                        if ui.button("\u{E1CA}  Duplicate").clicked() {
                            action = Some(HostAction::Duplicate(h.id));
                            ui.close();
                        }
                        if ui.button(if h.pinned { "\u{E46A}  Unpin" } else { "\u{E46A}  Pin to top" }).clicked() {
                            action = Some(HostAction::TogglePin(h.id));
                            ui.close();
                        }
                        if ui.button("\u{E198}  Copy address").clicked() {
                            action = Some(HostAction::CopyAddress(h.address.clone()));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("\u{E4A6}  Delete").clicked() {
                            action = Some(HostAction::Delete(vec![h.id]));
                            ui.close();
                        }
                    });
                    if grid && (index + 1) % columns as usize == 0 {
                        ui.end_row();
                    }
                }
            };
            if grid {
                egui::Grid::new("host-grid")
                    .num_columns(columns as usize)
                    .spacing(Vec2::new(16.0, 16.0))
                    .show(ui, |ui| layout(ui));
            } else {
                ui.spacing_mut().item_spacing.y = 6.0;
                layout(ui);
            }
        });

        if let Some((id, at)) = ui.data(|d| d.get_temp::<(Uuid, f64)>(pending_edit_id())) {
            let now = ui.input(|i| i.time);
            let delay = ui.ctx().options(|o| o.input_options.max_double_click_delay);
            if now - at > delay {
                ui.data_mut(|d| d.remove::<(Uuid, f64)>(pending_edit_id()));
                action = action.or(Some(HostAction::Edit(id)));
            } else {
                ui.ctx()
                    .request_repaint_after_secs((at + delay - now) as f32 + 0.01);
            }
        }

        // Drop on "All hosts" breadcrumb area is handled through group cards; apply action.
        if let Some(a) = action {
            self.apply_host_action(a);
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
        if !self.vault.settings.show_host_status
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
                format!(
                    "{}, {user}{}{port}{chain}",
                    p.label().to_lowercase(),
                    h.address
                )
            }
        }
    }

    pub fn filtered_hosts(&self) -> Vec<Host> {
        let q = self.search.trim().to_lowercase();
        let mut v: Vec<Host> = self
            .vault
            .hosts
            .iter()
            .filter(|h| {
                if let Some(t) = &self.tag_filter {
                    if !h.tags.contains(t) {
                        return false;
                    }
                }
                if q.is_empty() {
                    return self.tag_filter.is_some() || h.group_id == self.current_group;
                }
                let r = self.vault.resolve(h);
                let hay = format!(
                    "{} {} {} {} {}",
                    h.label,
                    h.address,
                    r.username,
                    h.tags.join(" "),
                    self.vault.group_path(h.group_id)
                )
                .to_lowercase();
                q.split_whitespace().all(|w| hay.contains(w))
            })
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            b.pinned.cmp(&a.pinned).then_with(|| {
                a.display_name()
                    .to_lowercase()
                    .cmp(&b.display_name().to_lowercase())
            })
        });
        v
    }

    fn apply_host_action(&mut self, a: HostAction) {
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
                self.search.clear();
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

/// A single click on a host card that opens the editor once it can't become a double-click.
fn pending_edit_id() -> egui::Id {
    egui::Id::new("host-pending-edit")
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
