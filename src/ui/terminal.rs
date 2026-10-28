//! Terminal tab: toolbar, split panes, broadcast input, snippets / history / AI side panel.

use egui::{CornerRadius, Pos2, Rect, RichText, Stroke, Vec2};

use super::term_view::{self, ContextAction, ViewConfig};
use super::{ghost_button, status_color};
use crate::app::{App, Dialog, SidePanel, SplitLayout, Tab};
use crate::conn::{AppEvent, Status};
use crate::model::Snippet;

enum TermAction {
    Split(SplitLayout),
    ClosePane(usize),
    Reconnect(u64),
    OpenSftp(Option<uuid::Uuid>),
    History(String, String),
    RunSnippet(Snippet),
    AskAi(u64, String, String),
    Copy(String),
    OpenUrl(String),
    FontDelta(f32),
    ListContainers {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
    },
    ContainerAct {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        item: crate::containers::Item,
        action: crate::containers::Action,
    },
    ContainerLogs {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        item: crate::containers::Item,
    },
    PollHealth {
        tab: u64,
        session: u64,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        previous: Option<crate::health::CpuTimes>,
    },
}

/// Seconds between container list refreshes while the panel is open.
const CONTAINER_INTERVAL: f64 = 5.0;

/// Seconds between server-health polls while the panel is open.
const HEALTH_INTERVAL: f64 = 3.0;

fn pane_rects(area: Rect, n: usize, layout: SplitLayout) -> Vec<Rect> {
    if n <= 1 {
        return vec![area];
    }
    let gap = 2.0;
    match layout {
        SplitLayout::Horizontal => {
            let w = (area.width() - gap * (n - 1) as f32) / n as f32;
            (0..n)
                .map(|i| {
                    Rect::from_min_size(
                        Pos2::new(area.left() + i as f32 * (w + gap), area.top()),
                        Vec2::new(w, area.height()),
                    )
                })
                .collect()
        }
        SplitLayout::Vertical => {
            let h = (area.height() - gap * (n - 1) as f32) / n as f32;
            (0..n)
                .map(|i| {
                    Rect::from_min_size(
                        Pos2::new(area.left(), area.top() + i as f32 * (h + gap)),
                        Vec2::new(area.width(), h),
                    )
                })
                .collect()
        }
        SplitLayout::Grid => {
            let cols = (n as f32).sqrt().ceil() as usize;
            let rows = n.div_ceil(cols);
            let w = (area.width() - gap * (cols - 1) as f32) / cols as f32;
            let h = (area.height() - gap * (rows - 1) as f32) / rows as f32;
            (0..n)
                .map(|i| {
                    let (c, r) = (i % cols, i / cols);
                    Rect::from_min_size(
                        Pos2::new(
                            area.left() + c as f32 * (w + gap),
                            area.top() + r as f32 * (h + gap),
                        ),
                        Vec2::new(w, h),
                    )
                })
                .collect()
        }
    }
}

impl App {
    pub fn terminal_tab(&mut self, ui: &mut egui::Ui, idx: usize) {
        let pal = self.pal();
        let settings = self.vault.settings.clone();
        let keywords = if settings.keyword_highlighting {
            self.keyword_cache.1.clone()
        } else {
            vec![]
        };
        let mut actions: Vec<TermAction> = vec![];
        let modal_open = !self.prompts.is_empty() || self.dialog.is_some() || self.palette.open;

        let Some(Tab::Terminal(tab)) = self.tabs.get_mut(idx) else {
            return;
        };
        let tab_id = tab.id;

        // Session actions stay in a compact toolbar; shell input keeps its own keybindings.
        egui::Panel::top(egui::Id::unique(("term-toolbar", tab_id)))
            .exact_size(48.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::symmetric(16, 6))
                    .stroke(Stroke::new(1.0, pal.border)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(pane) = tab.panes.get(tab.active) {
                        let status = pane.session.status.lock().clone();
                        ui.label(RichText::new("●").color(status_color(&status, pal)));
                        let target = &pane.session.target;
                        ui.add(egui::Label::new(RichText::new(&target.label).strong()).truncate())
                            .on_hover_text(format!(
                                "{} · {}@{}:{}",
                                target.protocol.label(),
                                target.username,
                                target.address,
                                target.port
                            ));
                        if ui.ctx().content_rect().width() > 1100.0 {
                            ui.label(
                                RichText::new(target.protocol.label())
                                    .small()
                                    .color(pal.text_dim),
                            );
                        }
                        match &status {
                            Status::Connecting(message) => {
                                ui.spinner();
                                ui.label(RichText::new(message).small().color(pal.warning));
                            }
                            Status::Failed(_) | Status::Closed(_) => {
                                if ui.button("Reconnect").clicked() {
                                    actions.push(TermAction::Reconnect(pane.session.id));
                                }
                            }
                            _ => {}
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("\u{E1FE}", |ui| {
                            if ui.button("Increase terminal font").clicked() {
                                actions.push(TermAction::FontDelta(1.0));
                                ui.close();
                            }
                            if ui.button("Decrease terminal font").clicked() {
                                actions.push(TermAction::FontDelta(-1.0));
                                ui.close();
                            }
                            if tab.panes.len() > 1 {
                                ui.separator();
                                ui.checkbox(&mut tab.broadcast, "Broadcast input to all panes");
                            }
                        })
                        .response
                        .on_hover_text("Terminal preferences");
                        ui.menu_button("\u{E870}  Split", |ui| {
                            if ui.button("Split right").clicked() {
                                actions.push(TermAction::Split(SplitLayout::Horizontal));
                                ui.close();
                            }
                            if ui.button("Split down").clicked() {
                                actions.push(TermAction::Split(SplitLayout::Vertical));
                                ui.close();
                            }
                            if ui.button("Add grid pane").clicked() {
                                actions.push(TermAction::Split(SplitLayout::Grid));
                                ui.close();
                            }
                        });
                        ui.menu_button("\u{E32E}  Tools", |ui| {
                            for (panel, label) in [
                                (SidePanel::Health, "Server health"),
                                (SidePanel::Containers, "Containers"),
                                (SidePanel::Snippets, "Snippets"),
                                (SidePanel::History, "Command history"),
                                (SidePanel::Ai, "AI command help"),
                            ] {
                                if ui
                                    .selectable_label(tab.side_panel == Some(panel), label)
                                    .clicked()
                                {
                                    tab.side_panel = Some(panel);
                                    ui.close();
                                }
                            }
                            if tab.side_panel.is_some() {
                                ui.separator();
                                if ui.button("Close session tools").clicked() {
                                    tab.side_panel = None;
                                    ui.close();
                                }
                            }
                        });
                        if let Some(pane) = tab.panes.get(tab.active) {
                            let ssh = pane.session.target.protocol == crate::model::Protocol::Ssh;
                            if ui
                                .add_enabled(ssh, egui::Button::new("\u{E0A0}  Files"))
                                .on_disabled_hover_text(
                                    "Open an SSH session to browse its remote files.",
                                )
                                .clicked()
                            {
                                actions.push(TermAction::OpenSftp(pane.session.target.host_id));
                            }
                        }
                    });
                });
            });

        // ---------- Server health polling ----------
        if tab.side_panel == Some(SidePanel::Health) {
            if let Some(pane) = tab.panes.get(tab.active) {
                let session = &pane.session;
                if tab.health.session != session.id {
                    tab.health.reset(session.id);
                }
                let now = ui.input(|i| i.time);
                if !tab.health.in_flight
                    && now - tab.health.last_poll >= HEALTH_INTERVAL
                    && session.is_live()
                {
                    let client = session.ssh.lock().clone();
                    if let Some(client) = client {
                        tab.health.in_flight = true;
                        tab.health.last_poll = now;
                        actions.push(TermAction::PollHealth {
                            tab: tab_id,
                            session: session.id,
                            client,
                            previous: tab.health.snapshot.as_ref().and_then(|s| s.cpu_times),
                        });
                    }
                }
            }
        }

        // ---------- Container polling ----------
        if tab.side_panel == Some(SidePanel::Containers) {
            if let Some(pane) = tab.panes.get(tab.active) {
                let session = &pane.session;
                if tab.containers.session != session.id {
                    tab.containers.reset(session.id);
                }
                let now = ui.input(|i| i.time);
                if !tab.containers.in_flight
                    && now - tab.containers.last_poll >= CONTAINER_INTERVAL
                    && session.is_live()
                    && session.target.protocol == crate::model::Protocol::Ssh
                {
                    let client = session.ssh.lock().clone();
                    if let Some(client) = client {
                        tab.containers.in_flight = true;
                        tab.containers.last_poll = now;
                        actions.push(TermAction::ListContainers {
                            tab: tab_id,
                            session: session.id,
                            sudo: tab.containers.use_sudo,
                            client,
                        });
                    }
                }
            }
        }

        // ---------- Side panel ----------
        if let Some(panel) = tab.side_panel {
            egui::Panel::right(egui::Id::unique(("term-side", tab_id)))
                .default_size(288.0)
                .size_range(240.0..=400.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(pal.panel).inner_margin(egui::Margin::same(12)).stroke(Stroke::new(1.0, pal.border)))
                .show(ui, |ui| {
                    ui.horizontal(|ui|{
                        ui.label(RichText::new("SESSION TOOLS").small().color(pal.text_dim));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{if ui.add(ghost_button("×")).on_hover_text("Close session tools").clicked(){tab.side_panel=None;}});
                    });
                    ui.horizontal_wrapped(|ui|{
                        for(next,label)in [(SidePanel::Health,"Health"),(SidePanel::Containers,"Containers"),(SidePanel::Snippets,"Snippets"),(SidePanel::History,"History"),(SidePanel::Ai,"AI")]{if ui.selectable_label(panel==next,label).clicked(){tab.side_panel=Some(next);}}
                    });ui.separator();
                    match panel {
                    SidePanel::Snippets => {
                        ui.label(RichText::new("Snippets").strong().size(16.0));
                        ui.add(egui::TextEdit::singleline(&mut tab.snippet_filter).hint_text("Filter").desired_width(f32::INFINITY));
                        let q = tab.snippet_filter.to_lowercase();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for s in self.vault.snippets.iter().filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q)) {
                                let r = super::card_frame(pal)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(RichText::new(&s.name).strong());
                                        ui.label(RichText::new(s.script.lines().next().unwrap_or("")).monospace().small().color(pal.text_dim));
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if r.clicked() {
                                    actions.push(TermAction::RunSnippet(s.clone()));
                                }
                            }
                            if self.vault.snippets.is_empty() {
                                ui.label(RichText::new("No snippets yet").color(pal.text_dim));
                            }
                        });
                    }
                    SidePanel::History => {
                        ui.label(RichText::new("History").strong().size(16.0));
                        ui.label(RichText::new("Click to paste into the terminal").small().color(pal.text_dim));
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for h in self.vault.history.iter().rev().take(300) {
                                let r = ui.add(egui::Button::new(RichText::new(&h.command).monospace()).frame(false).wrap());
                                if r.clicked() {
                                    if let Some(p) = tab.panes.get(tab.active) {
                                        p.session.send_text(&h.command);
                                    }
                                }
                            }
                        });
                    }
                    SidePanel::Health => {
                        let source = match tab.panes.get(tab.active) {
                            Some(p) if p.session.target.protocol != crate::model::Protocol::Ssh => {
                                super::health_view::Source::Unsupported
                            }
                            Some(p) if !p.session.is_live() => {
                                super::health_view::Source::Disconnected(&p.session.target.label)
                            }
                            Some(p) => super::health_view::Source::Ssh(&p.session.target.label),
                            None => super::health_view::Source::Unsupported,
                        };
                        super::health_view::show(ui, pal, &tab.health, source);
                    }
                    SidePanel::Containers => {
                        let source = match tab.panes.get(tab.active) {
                            Some(p) if p.session.target.protocol != crate::model::Protocol::Ssh => {
                                super::container_view::Source::Unsupported
                            }
                            Some(p) if !p.session.is_live() => {
                                super::container_view::Source::Disconnected(&p.session.target.label)
                            }
                            Some(p) => super::container_view::Source::Ssh(&p.session.target.label),
                            None => super::container_view::Source::Unsupported,
                        };
                        let command = super::container_view::show(ui, pal, &mut tab.containers, source);
                        if let (Some(command), Some(pane)) = (command, tab.panes.get(tab.active)) {
                            use super::container_view::Command;
                            let session = pane.session.id;
                            let sudo = tab.containers.use_sudo;
                            let client = pane.session.ssh.lock().clone();
                            match (command, client) {
                                (Command::Shell(text), _) => pane.session.send_text(&text),
                                (Command::UseSudo, _) => {
                                    tab.containers.use_sudo = true;
                                    tab.containers.last_poll = f64::NEG_INFINITY;
                                }
                                (Command::Act(item, action), Some(client)) => {
                                    tab.containers.busy = Some(format!("{} {}…", action.label(), item.name));
                                    tab.containers.notice = None;
                                    actions.push(TermAction::ContainerAct { tab: tab_id, session, sudo, client, item, action });
                                }
                                (Command::Logs(item), Some(client)) => {
                                    tab.containers.logs = Some(crate::containers::LogView { item: item.clone(), text: None });
                                    actions.push(TermAction::ContainerLogs { tab: tab_id, session, sudo, client, item });
                                }
                                _ => {}
                            }
                        }
                    }
                    SidePanel::Ai => {
                        ui.label(RichText::new("\u{E6A2} AI command assistant").strong().size(16.0));
                        ui.label(RichText::new("Describe what you want to do. The command is inserted, never run automatically.").small().color(pal.text_dim));
                        let te = ui.add(
                            egui::TextEdit::multiline(&mut tab.ai_query)
                                .hint_text("e.g. find the 10 largest files under /var")
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                        let submit = te.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.command);
                        ui.horizontal(|ui| {
                            if (ui.add_enabled(!tab.ai_busy, super::primary_button("Generate", pal)).clicked() || submit) && !tab.ai_query.trim().is_empty() {
                                let ctxs = tab
                                    .panes
                                    .get(tab.active)
                                    .map(|p| format!("{} via {}", p.session.target.label, p.session.target.protocol.label()))
                                    .unwrap_or_default();
                                tab.ai_busy = true;
                                actions.push(TermAction::AskAi(tab_id, tab.ai_query.clone(), ctxs));
                            }
                            if tab.ai_busy {
                                ui.spinner();
                            }
                        });
                        match &tab.ai_result {
                            Some(Ok(cmd)) => {
                                let cmd = cmd.clone();
                                egui::Frame::new().fill(pal.bg).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(&cmd).monospace());
                                });
                                ui.horizontal(|ui| {
                                    if ui.button("Insert").clicked() {
                                        if let Some(p) = tab.panes.get(tab.active) {
                                            p.session.send_text(&cmd);
                                        }
                                    }
                                    if ui.button("Copy").clicked() {
                                        actions.push(TermAction::Copy(cmd.clone()));
                                    }
                                });
                            }
                            Some(Err(e)) => {
                                ui.colored_label(pal.danger, e);
                            }
                            None => {}
                        }
                    }
                    }
                });
        }

        if tab.broadcast && tab.panes.len() > 1 {
            egui::Panel::top(egui::Id::unique(("broadcast-warning", tab_id)))
                .frame(
                    egui::Frame::new()
                        .fill(pal.panel)
                        .inner_margin(egui::Margin::symmetric(16, 8)),
                )
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} Broadcast input: {}",
                                crate::icons::BROADCAST,
                                tab.panes
                                    .iter()
                                    .map(|p| p.session.target.label.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ))
                            .color(pal.warning),
                        );
                        if ui.button("Stop broadcast").clicked() {
                            tab.broadcast = false;
                        }
                    });
                });
        }
        // ---------- Panes ----------
        let mut close_pane = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.bg))
            .show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let rects = pane_rects(area, tab.panes.len(), tab.layout);
                let multi = tab.panes.len() > 1;
                let mut broadcast_bytes: Vec<u8> = vec![];
                let mut direct: Vec<(usize, Vec<u8>)> = vec![];
                let mut new_active = None;
                for (i, (pane, rect)) in tab.panes.iter_mut().zip(rects.iter()).enumerate() {
                    let (scheme, font) = {
                        let s = &settings;
                        let sc = crate::themes::scheme_by_name(
                            pane.session
                                .target
                                .color_scheme
                                .as_deref()
                                .unwrap_or(&s.terminal_scheme),
                        );
                        (
                            sc,
                            egui::FontId::new(
                                pane.session.target.font_size.unwrap_or(s.font_size),
                                crate::fonts::terminal_family(),
                            ),
                        )
                    };
                    let suggestion =
                        if settings.autocomplete && i == tab.active && !pane.session.line_untracked
                        {
                            crate::suggest::suggest(
                                &pane.session.line_buf,
                                &pane.session.target.label,
                                &self.vault.history,
                                &self.vault.snippets,
                            )
                        } else {
                            None
                        };
                    let cfg = ViewConfig {
                        scheme: &scheme,
                        font,
                        line_height: settings.line_height,
                        cursor_shape: settings.cursor_shape,
                        cursor_blink: settings.cursor_blink,
                        keywords: &keywords,
                        copy_on_select: settings.copy_on_select,
                        right_click_paste: settings.right_click_paste,
                        option_is_meta: settings.option_is_meta,
                        brighten_bold: settings.brighten_bold,
                        suggestion,
                        focused: i == tab.active && !modal_open,
                    };
                    let backspace = pane.session.target.backspace;
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(*rect)
                            .id_salt(("pane", pane.session.id)),
                    );
                    let out = term_view::show(
                        &mut child,
                        &mut pane.session,
                        &mut pane.view,
                        &cfg,
                        backspace,
                    );
                    if multi {
                        let border = if i == tab.active {
                            pal.accent
                        } else {
                            pal.border
                        };
                        ui.painter().rect_stroke(
                            *rect,
                            0.0,
                            Stroke::new(1.0, border),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if out.clicked {
                        new_active = Some(i);
                    }
                    if !out.input.is_empty() {
                        if tab.broadcast && multi {
                            broadcast_bytes.extend(out.input);
                        } else {
                            direct.push((i, out.input));
                        }
                    }
                    if let Some(line) = out.entered_line {
                        actions.push(TermAction::History(line, pane.session.target.label.clone()));
                    }
                    if let Some(c) = out.copied {
                        actions.push(TermAction::Copy(c));
                    }
                    if let Some(u) = out.open_url {
                        actions.push(TermAction::OpenUrl(u));
                    }
                    match out.context {
                        Some(ContextAction::SplitRight) => {
                            actions.push(TermAction::Split(SplitLayout::Horizontal))
                        }
                        Some(ContextAction::SplitDown) => {
                            actions.push(TermAction::Split(SplitLayout::Vertical))
                        }
                        Some(ContextAction::ClosePane) => close_pane = Some(i),
                        Some(ContextAction::OpenSftp) => {
                            actions.push(TermAction::OpenSftp(pane.session.target.host_id))
                        }
                        Some(ContextAction::Clear) => pane.session.send(b"\x0c".to_vec()),
                        Some(ContextAction::Find) => pane.view.search_open = true,
                        Some(ContextAction::SelectAll) => {
                            let mut t = pane.session.term.lock();
                            let grid = t.term.grid();
                            let top = alacritty_terminal::index::Point::new(
                                alacritty_terminal::index::Line(
                                    -(alacritty_terminal::grid::Dimensions::history_size(grid)
                                        as i32),
                                ),
                                alacritty_terminal::index::Column(0),
                            );
                            let bottom = alacritty_terminal::index::Point::new(
                                alacritty_terminal::index::Line(
                                    alacritty_terminal::grid::Dimensions::screen_lines(grid) as i32
                                        - 1,
                                ),
                                alacritty_terminal::index::Column(
                                    alacritty_terminal::grid::Dimensions::columns(grid) - 1,
                                ),
                            );
                            let mut sel = alacritty_terminal::selection::Selection::new(
                                alacritty_terminal::selection::SelectionType::Simple,
                                top,
                                alacritty_terminal::index::Side::Left,
                            );
                            sel.update(bottom, alacritty_terminal::index::Side::Right);
                            t.term.selection = Some(sel);
                        }
                        None => {}
                    }
                    // Disconnected banner.
                    let st = pane.session.status.lock().clone();
                    if let Status::Closed(reason) | Status::Failed(reason) = &st {
                        let banner = Rect::from_min_size(
                            Pos2::new(rect.left() + 12.0, rect.bottom() - 52.0),
                            Vec2::new(rect.width() - 24.0, 40.0),
                        );
                        let mut bui = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(banner)
                                .id_salt(("banner", pane.session.id)),
                        );
                        egui::Frame::new()
                            .fill(pal.panel)
                            .stroke(Stroke::new(
                                1.0,
                                if matches!(st, Status::Failed(_)) {
                                    pal.danger
                                } else {
                                    pal.border
                                },
                            ))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(egui::Margin::symmetric(12, 6))
                            .show(&mut bui, |ui| {
                                ui.horizontal(|ui| {
                                    let short: String = reason.chars().take(90).collect();
                                    ui.label(RichText::new(short).color(pal.text_dim));
                                    if let Some(at) = pane.session.reconnect_at {
                                        let left = (at - ui.input(|i| i.time)).max(0.0);
                                        ui.label(
                                            RichText::new(format!("reconnecting in {:.0}s", left))
                                                .small()
                                                .color(pal.warning),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui.button("Close").clicked() {
                                                close_pane = Some(i);
                                            }
                                            if ui
                                                .add(super::primary_button(
                                                    "\u{E036} Reconnect",
                                                    pal,
                                                ))
                                                .clicked()
                                            {
                                                actions
                                                    .push(TermAction::Reconnect(pane.session.id));
                                            }
                                        },
                                    );
                                });
                            });
                        // Enter reconnects too.
                        if i == tab.active
                            && !modal_open
                            && ui.input(|inp| inp.key_pressed(egui::Key::Enter))
                            && pane.session.reconnect_at.is_none()
                        {
                            actions.push(TermAction::Reconnect(pane.session.id));
                        }
                    }
                }
                if let Some(a) = new_active {
                    tab.active = a;
                }
                if !broadcast_bytes.is_empty() {
                    for p in tab.panes.iter() {
                        if p.session.is_live() {
                            p.session.send(broadcast_bytes.clone());
                        }
                    }
                }
                for (i, bytes) in direct {
                    if let Some(p) = tab.panes.get(i) {
                        p.session.send(bytes);
                    }
                }
            });
        if let Some(i) = close_pane {
            actions.push(TermAction::ClosePane(i));
        }

        for a in actions {
            match a {
                TermAction::Split(l) => self.split_active(l),
                TermAction::ClosePane(i) => {
                    let mut close_tab = false;
                    if let Some(Tab::Terminal(t)) = self.tabs.get_mut(idx) {
                        if i < t.panes.len() {
                            let mut p = t.panes.remove(i);
                            p.session.user_closed = true;
                            p.session.send_input(crate::term::Input::Close);
                        }
                        t.active = t.active.min(t.panes.len().saturating_sub(1));
                        close_tab = t.panes.is_empty();
                    }
                    if close_tab {
                        self.close_tab(tab_id);
                    }
                }
                TermAction::Reconnect(sid) => self.reconnect_session(sid),
                TermAction::OpenSftp(h) => self.open_sftp(h),
                TermAction::History(line, host) => {
                    self.vault.record_history(&line, &host);
                    self.mark_dirty();
                }
                TermAction::RunSnippet(s) => self.begin_run_snippet(s),
                TermAction::AskAi(tab, q, c) => {
                    let s = self.vault.settings.clone();
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::ai::suggest_command(&s, &q, &c)
                            .await
                            .map_err(|e| format!("{e:#}"));
                        bridge.send(AppEvent::AiResult { tab, result });
                    });
                }
                TermAction::Copy(c) => {
                    ui.ctx().copy_text(c);
                }
                TermAction::OpenUrl(u) => {
                    let _ = open::that_detached(u);
                }
                TermAction::PollHealth {
                    tab,
                    session,
                    client,
                    previous,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::health::poll(&client, previous)
                            .await
                            .map_err(|e| format!("{e:#}"));
                        bridge.send(AppEvent::Health {
                            tab,
                            session,
                            result,
                        });
                    });
                }
                TermAction::ListContainers {
                    tab,
                    session,
                    sudo,
                    client,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::list(&client, sudo)
                            .await
                            .map_err(|e| format!("{e:#}"));
                        bridge.send(AppEvent::Containers {
                            tab,
                            session,
                            result,
                        });
                    });
                }
                TermAction::ContainerAct {
                    tab,
                    session,
                    sudo,
                    client,
                    item,
                    action,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::act(&client, &item, action, sudo)
                            .await
                            .map(|()| format!("{} {}: done", action.label(), item.name))
                            .map_err(|e| format!("{} {}: {e:#}", action.label(), item.name));
                        bridge.send(AppEvent::ContainerAction {
                            tab,
                            session,
                            result,
                        });
                    });
                }
                TermAction::ContainerLogs {
                    tab,
                    session,
                    sudo,
                    client,
                    item,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::logs(&client, &item, sudo)
                            .await
                            .map_err(|e| format!("{e:#}"));
                        bridge.send(AppEvent::ContainerLogs {
                            tab,
                            session,
                            key: item.key(),
                            result,
                        });
                    });
                }
                TermAction::FontDelta(d) => {
                    self.vault.settings.font_size =
                        (self.vault.settings.font_size + d).clamp(8.0, 36.0);
                    self.mark_dirty();
                }
            }
        }
    }

    /// Prepare running a snippet: ask for {{variables}} and target terminals.
    pub fn begin_run_snippet(&mut self, snippet: Snippet) {
        let re = regex::Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_-]*)\s*\}\}").unwrap();
        let mut vars: Vec<(String, String)> = vec![];
        for c in re.captures_iter(&snippet.script) {
            let name = c[1].to_string();
            if !vars.iter().any(|(n, _)| *n == name) {
                vars.push((name, String::new()));
            }
        }
        let active_tab = self.active_tab;
        let mut targets = vec![];
        for t in &self.tabs {
            if let Tab::Terminal(tt) = t {
                for (i, p) in tt.panes.iter().enumerate() {
                    if p.session.is_live() {
                        targets.push((p.session.id, Some(tt.id) == active_tab && i == tt.active));
                    }
                }
            }
        }
        if targets.is_empty() {
            self.toast("Open a terminal first, then run the snippet", true);
            return;
        }
        if vars.is_empty() && targets.iter().filter(|t| t.1).count() == 1 && targets.len() == 1 {
            self.run_snippet_now(&snippet.script, &[targets[0].0]);
            return;
        }
        if !targets.iter().any(|t| t.1) {
            targets[0].1 = true;
        }
        self.dialog = Some(Dialog::RunSnippet {
            snippet,
            values: vars,
            targets,
        });
    }

    pub fn run_snippet_now(&mut self, script: &str, sessions: &[u64]) {
        let mut text = script.replace("\r\n", "\n").replace('\n', "\r");
        if !text.ends_with('\r') {
            text.push('\r');
        }
        for t in &self.tabs {
            if let Tab::Terminal(tt) = t {
                for p in &tt.panes {
                    if sessions.contains(&p.session.id) {
                        p.session.send_text(&text);
                    }
                }
            }
        }
    }
}
