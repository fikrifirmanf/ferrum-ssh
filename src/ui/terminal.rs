//! Terminal tab: toolbar, split panes, broadcast input, snippets / history / AI side panel.

use egui::{CornerRadius, Pos2, Rect, RichText, Stroke, Vec2};

use super::term_view::{self, ContextAction, ViewConfig};
use super::{forms, motion, status_color};
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
        runtime: Option<crate::containers::Runtime>,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
    },
    ContainerInventory {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        request: crate::containers::inventory::Request,
    },
    BrowseVolume {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        name: String,
        directory: Vec<Vec<u8>>,
        revision: u64,
    },
    ExportVolume {
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        name: String,
    },
    EditVolume {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        request: crate::containers::volume_files::EditRequest,
    },
    ImportVolume {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        name: String,
    },
    UnifiedLogs {
        tab: u64,
        session: u64,
        revision: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        sources: Vec<crate::containers::unified_logs::Source>,
    },
    EditLogPreset {
        host: uuid::Uuid,
        edit: crate::containers::unified_logs::PresetEdit,
    },
    ExportLogs(Vec<crate::containers::unified_logs::Row>),
    ContainerResources {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        listing: crate::containers::Listing,
    },
    ContainerDetail {
        tab: u64,
        session: u64,
        sudo: bool,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        item: crate::containers::Item,
        sequence: u64,
        detail: crate::containers::manager::DetailTab,
    },
    ContainerCredentials {
        tab: u64,
        session: u64,
        key: String,
        item: crate::containers::Item,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        sudo: bool,
    },
    ContainerOpenPort {
        tab: u64,
        session: u64,
        item: crate::containers::Item,
        port: crate::containers::ports::PublishedPort,
        mode: crate::containers::ports::OpenAs,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        sudo: bool,
    },
    ContainerBrowser {
        tab: u64,
        session: u64,
        key: String,
        url: String,
    },
    ContainerTerminal {
        target: crate::model::ResolvedTarget,
        client: std::sync::Arc<crate::conn::ssh::SshClient>,
        item: crate::containers::Item,
        sudo: bool,
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

const SESSION_TOOLS: [(SidePanel, &str); 5] = [
    (SidePanel::Health, "Server health"),
    (SidePanel::Containers, "Containers"),
    (SidePanel::Snippets, "Snippets"),
    (SidePanel::History, "Command history"),
    (SidePanel::Ai, "AI command help"),
];

/// Seconds between container list refreshes while the panel is open.
const CONTAINER_INTERVAL: f64 = 5.0;

/// Seconds between server-health polls while the panel is open.
const HEALTH_INTERVAL: f64 = 3.0;

fn pane_rects(area: Rect, n: usize, layout: SplitLayout) -> Vec<Rect> {
    if n <= 1 {
        return vec![area];
    }
    let gap = 1.0;
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
            .show_separator_line(false)
            .exact_size(crate::themes::tokens::BAR)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 0)))
            .show(ui, |ui| {
                super::glass::surface_behind(
                    ui,
                    egui::Margin::symmetric(12, 0),
                    pal.bg,
                    Some(super::glass::Edge::Bottom),
                );
                ui.horizontal_centered(|ui| {
                    if let Some(pane) = tab.panes.get(tab.active) {
                        let status = pane.session.status.lock().clone();
                        let color = status_color(&status, pal);
                        ui.spacing_mut().item_spacing.x = 6.0;
                        forms::dot(ui, color);
                        // State changes show at once, with a word as well as a color.
                        let word = match &status {
                            Status::Connected => "Connected".to_string(),
                            Status::Connecting(message) => message.clone(),
                            Status::Closed(_) => "Closed".to_string(),
                            Status::Failed(_) => "Failed".to_string(),
                        };
                        ui.add(
                            egui::Label::new(RichText::new(word).size(13.0).color(color))
                                .truncate(),
                        );
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if matches!(status, Status::Connecting(_)) {
                            ui.spinner();
                        }
                        let target = &pane.session.target;
                        let address = if target.username.is_empty() {
                            target.address.clone()
                        } else {
                            format!("{}@{}", target.username, target.address)
                        };
                        if ui.ctx().content_rect().width() > 960.0 {
                            ui.label(RichText::new(address).monospace().color(pal.text_dim))
                                .on_hover_text(format!(
                                    "{} · {}:{}",
                                    target.protocol.label(),
                                    target.address,
                                    target.port
                                ));
                        }
                        if matches!(status, Status::Failed(_) | Status::Closed(_))
                            && ui.add(forms::Btn::secondary("Reconnect").small()).clicked()
                        {
                            actions.push(TermAction::Reconnect(pane.session.id));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().interact_size.y = 32.0;
                        ui.menu_button(crate::icons::DOTS_THREE, |ui| {
                            crate::ui::forms::menu_padding(ui);
                            if ui.button("Increase terminal font").clicked() {
                                actions.push(TermAction::FontDelta(1.0));
                                ui.close();
                            }
                            if ui.button("Decrease terminal font").clicked() {
                                actions.push(TermAction::FontDelta(-1.0));
                                ui.close();
                            }
                        })
                        .response
                        .on_hover_text("Terminal preferences");
                        ui.menu_button(
                            format!("{}  Split", crate::icons::SQUARE_SPLIT_HORIZONTAL),
                            |ui| {
                                crate::ui::forms::menu_padding(ui);
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
                            },
                        );
                        ui.menu_button(format!("{}  Tools", crate::icons::MONITOR), |ui| {
                            crate::ui::forms::menu_padding(ui);
                            for (panel, label) in SESSION_TOOLS {
                                if forms::option(ui, tab.side_panel == Some(panel), label).clicked()
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
                                .add_enabled(
                                    ssh,
                                    // Same face as the Tools and Split menus beside it.
                                    egui::Button::new(format!(
                                        "{}  Files",
                                        crate::icons::ARROWS_DOWN_UP
                                    )),
                                )
                                .on_disabled_hover_text(
                                    "Open an SSH session to browse its remote files.",
                                )
                                .clicked()
                            {
                                actions.push(TermAction::OpenSftp(pane.session.target.host_id));
                            }
                        }
                        if tab.panes.len() > 1 {
                            forms::switch(ui, &mut tab.broadcast, "Broadcast input to all panes");
                            ui.label(RichText::new("Broadcast").size(13.0).color(pal.text_dim));
                        }
                    });
                });
            });

        // ---------- Server health polling ----------
        if tab.side_panel == Some(SidePanel::Health) && !tab.containers.manager.open {
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
        if tab.side_panel == Some(SidePanel::Containers) || tab.containers.manager.open {
            if let Some(pane) = tab.panes.get(tab.active) {
                let session = &pane.session;
                if tab.containers.session != session.id {
                    let open = tab.containers.manager.open;
                    tab.containers.reset(session.id);
                    tab.containers.manager.open = open;
                }
                let now = ui.input(|i| i.time);
                if !tab.containers.in_flight
                    && tab.containers.busy.is_none()
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
                            runtime: tab.containers.retry_runtime.take(),
                            client,
                        });
                    }
                }
                if tab.containers.manager.open
                    && tab.containers.manager.section == crate::containers::manager::Section::Containers
                    && session.is_live()
                    && session.target.protocol == crate::model::Protocol::Ssh
                {
                    if let Some(client) = session.ssh.lock().clone() {
                        let manager = &mut tab.containers.manager;
                        if !manager.stats_pending
                            && now - manager.last_stats_poll >= CONTAINER_INTERVAL
                        {
                            if let Some(listing) = &tab.containers.listing {
                                manager.stats_pending = true;
                                manager.last_stats_poll = now;
                                actions.push(TermAction::ContainerResources {
                                    tab: tab_id,
                                    session: session.id,
                                    sudo: tab.containers.use_sudo,
                                    client: client.clone(),
                                    listing: listing.clone(),
                                });
                            }
                        }
                        if let Some(item) = tab
                            .containers
                            .listing
                            .as_ref()
                            .and_then(|list| {
                                list.items
                                    .iter()
                                    .find(|item| manager.selected.as_deref() == Some(&item.key()))
                            })
                            .cloned()
                        {
                            use crate::containers::manager::DetailTab;
                            let due = match manager.tab {
                                DetailTab::Logs => {
                                    !manager.pause_logs && now - manager.last_detail_poll >= 2.0
                                }
                                DetailTab::Ports => {
                                    item.runtime != crate::containers::Runtime::Kubernetes
                                        && manager.ports.is_none()
                                }
                                DetailTab::Inspect => {
                                    item.runtime != crate::containers::Runtime::Kubernetes
                                        && manager.inspect.is_none()
                                }
                                _ => false,
                            };
                            if due && manager.detail_pending.is_none() {
                                let detail = manager.tab;
                                let sequence = manager.begin_detail(now);
                                actions.push(TermAction::ContainerDetail {
                                    tab: tab_id,
                                    session: session.id,
                                    sudo: tab.containers.use_sudo,
                                    client,
                                    item,
                                    sequence,
                                    detail,
                                });
                            }
                        }
                    }
                }
            }
        }

        // ---------- Side panel ----------
        if let Some(panel) = tab.side_panel.filter(|_| !tab.containers.manager.open) {
            // The terminal reflows once when the panel opens; only the panel's contents fade in,
            // because terminal text must never animate.
            let enter = motion::since_change(
                ui.ctx(),
                egui::Id::unique(("term-side-enter", tab_id)),
                format!("{panel:?}"),
                motion::PANEL,
            );
            egui::Panel::right(egui::Id::unique(("term-side", tab_id)))
            .show_separator_line(false)
                .default_size(340.0)
                .size_range(260.0..=440.0)
                .resizable(true)
                .frame(egui::Frame::new().inner_margin(egui::Margin::same(16)))
                .show(ui, |ui| {
                    super::glass::surface_behind(
                        ui,
                        egui::Margin::same(16),
                        pal.panel,
                        Some(super::glass::Edge::Left),
                    );
                    ui.set_opacity(enter);
                    ui.horizontal(|ui|{
                        let width=(ui.available_width()-40.0).max(120.0);
                        let shown=SESSION_TOOLS.iter().find(|(p,_)|*p==panel).map(|(_,l)|*l).unwrap_or("");
                        forms::select(ui,"session-tool",width,shown,false,|ui|{
                            for (next,label) in SESSION_TOOLS {
                                if forms::option(ui,panel==next,label).clicked(){tab.side_panel=Some(next);ui.close();}
                            }
                        });
                        if ui.add(forms::Btn::icon(crate::icons::X,"Close session tools")).on_hover_text("Close session tools").clicked(){tab.side_panel=None;}
                    });
                    ui.add_space(4.0);
                    match panel {
                    SidePanel::Snippets => {
                        forms::Input::bare("Filter snippets").hint("Filter").icon(crate::icons::MAGNIFYING_GLASS).show(ui,&mut tab.snippet_filter);
                        let q = tab.snippet_filter.to_lowercase();
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for s in self.vault.snippets.iter().filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q)) {
                                let r = super::card_frame(pal)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(RichText::new(&s.name).family(crate::fonts::semibold()));
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
                        forms::help(ui,"Click to paste into the terminal",pal);
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
                                (Command::OpenManager, _) => { tab.containers.manager.open = true; tab.containers.manager.last_stats_poll = f64::NEG_INFINITY; },
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
                        forms::help(ui,"Describe what you want to do. The command is inserted, never run automatically.",pal);
                        let te = forms::text_area(ui,"",&mut tab.ai_query,3,false,"Find the 10 largest files under /var");
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
                                super::glass::well_frame(ui.ctx(), 10).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(&cmd).monospace());
                                });
                                ui.horizontal(|ui| {
                                    if ui.add(forms::Btn::secondary("Insert").small()).clicked() {
                                        if let Some(p) = tab.panes.get(tab.active) {
                                            p.session.send_text(&cmd);
                                        }
                                    }
                                    if ui.add(forms::Btn::ghost("Copy").small()).clicked() {
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

        if tab.containers.manager.open {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(24, 20)))
                .show(ui, |ui| {
                    super::glass::surface_behind(ui, egui::Margin::symmetric(24, 20), pal.bg, None);
                    let source = match tab.panes.get(tab.active) {
                        Some(pane)
                            if pane.session.target.protocol != crate::model::Protocol::Ssh =>
                        {
                            super::container_view::Source::Unsupported
                        }
                        Some(pane) if !pane.session.is_live() => {
                            super::container_view::Source::Disconnected(&pane.session.target.label)
                        }
                        Some(pane) => {
                            if *pane.session.status.lock() == Status::Connected {
                                super::container_view::Source::Ssh(&pane.session.target.label)
                            } else {
                                super::container_view::Source::Connecting(&pane.session.target.label)
                            }
                        }
                        None => super::container_view::Source::Unsupported,
                    };
                    let saved_host = tab.panes.get(tab.active)
                        .and_then(|pane| pane.session.target.host_id)
                        .filter(|id| self.vault.host(*id).is_some());
                    let presets = saved_host.map(|id| {
                        settings.log_filter_presets.get(&id).map(Vec::as_slice).unwrap_or_default()
                    });
                    let command = super::container_manager::show(
                        ui, pal, &mut tab.containers, source, presets,
                    );
                    if let (Some(command), Some(pane)) = (command, tab.panes.get(tab.active)) {
                        if let Some(client) = pane.session.ssh.lock().clone() {
                            match command {
                                super::container_manager::Command::Inventory(request) => {
                                    let page = tab.containers.manager.inventory.page(request.kind());
                                    page.pending = true;
                                    if !matches!(request, crate::containers::inventory::Request::List(_)
                                        | crate::containers::inventory::Request::Inspect(..))
                                    {
                                        page.notice = None;
                                    }
                                    actions.push(TermAction::ContainerInventory {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        sudo: tab.containers.use_sudo,
                                        client,
                                        request,
                                    });
                                }
                                super::container_manager::Command::BrowseVolume {
                                    name,
                                    directory,
                                    revision,
                                } => {
                                    tab.containers.manager.inventory
                                        .page(crate::containers::inventory::Kind::Volumes)
                                        .files.pending = true;
                                    actions.push(TermAction::BrowseVolume {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        sudo: tab.containers.use_sudo,
                                        client,
                                        name,
                                        directory,
                                        revision,
                                    });
                                }
                                super::container_manager::Command::ExportVolume(name) => {
                                    actions.push(TermAction::ExportVolume {
                                        sudo: tab.containers.use_sudo,
                                        client,
                                        name,
                                    });
                                }
                                super::container_manager::Command::EditVolume(request) => {
                                    let files = &mut tab.containers.manager.inventory
                                        .page(crate::containers::inventory::Kind::Volumes).files;
                                    if let Some(editor) = &mut files.editor {
                                        editor.pending = true;
                                        editor.error = None;
                                        actions.push(TermAction::EditVolume {
                                            tab: tab_id, session: pane.session.id,
                                            sudo: tab.containers.use_sudo, client, request,
                                        });
                                    }
                                }
                                super::container_manager::Command::ImportVolume(name) => {
                                    actions.push(TermAction::ImportVolume {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        sudo: tab.containers.use_sudo,
                                        client,
                                        name,
                                    });
                                }
                                super::container_manager::Command::UnifiedLogs { revision, sources } => {
                                    let view = &mut tab.containers.manager.unified_logs;
                                    view.pending = true;
                                    view.last_request = Some(ui.input(|input| input.time));
                                    view.refresh_requested = false;
                                    actions.push(TermAction::UnifiedLogs {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        revision,
                                        sudo: tab.containers.use_sudo,
                                        client,
                                        sources,
                                    });
                                }
                                super::container_manager::Command::EditLogPreset(edit) => {
                                    if let Some(host) = saved_host {
                                        actions.push(TermAction::EditLogPreset { host, edit });
                                    }
                                }
                                super::container_manager::Command::ExportLogs(rows) => {
                                    actions.push(TermAction::ExportLogs(rows));
                                }
                                super::container_manager::Command::Action(item, action) => {
                                    tab.containers.busy =
                                        Some(format!("{} {}…", action.label(), item.name));
                                    tab.containers.notice = None;
                                    actions.push(TermAction::ContainerAct {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        sudo: tab.containers.use_sudo,
                                        client,
                                        item,
                                        action,
                                    });
                                }
                                super::container_manager::Command::OpenPort(item, port, mode) => {
                                    actions.push(TermAction::ContainerOpenPort {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        item,
                                        port,
                                        mode,
                                        client,
                                        sudo: tab.containers.use_sudo,
                                    });
                                }
                                super::container_manager::Command::ReadCredentials(key, item) => {
                                    actions.push(TermAction::ContainerCredentials {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        key,
                                        item,
                                        client,
                                        sudo: tab.containers.use_sudo,
                                    });
                                }
                                super::container_manager::Command::Browser { key, url } => {
                                    actions.push(TermAction::ContainerBrowser {
                                        tab: tab_id,
                                        session: pane.session.id,
                                        key,
                                        url,
                                    });
                                }
                                super::container_manager::Command::Terminal(item) => {
                                    actions.push(TermAction::ContainerTerminal {
                                        target: pane.session.target.clone(),
                                        client,
                                        item,
                                        sudo: tab.containers.use_sudo,
                                    });
                                }
                            }
                        }
                    }
                });
        } else {
            if tab.broadcast && tab.panes.len() > 1 {
                egui::Panel::top(egui::Id::unique(("broadcast-warning", tab_id)))
                    .show_separator_line(false)
                    .frame(
                        egui::Frame::new()
                            .fill(pal.warning.gamma_multiply(0.14))
                            .stroke(Stroke::new(1.0, pal.warning.gamma_multiply(0.4)))
                            .corner_radius(crate::themes::tokens::RADIUS_LARGE)
                            .outer_margin(egui::Margin {
                                left: 8,
                                right: 8,
                                top: 8,
                                bottom: 0,
                            })
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
                            if ui
                                .add(forms::Btn::secondary("Stop broadcast").small())
                                .clicked()
                            {
                                tab.broadcast = false;
                            }
                        });
                    });
            }
            // ---------- Panes ----------
            let mut close_pane = None;
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let area = ui.available_rect_before_wrap();
                    let rects = pane_rects(area, tab.panes.len(), tab.layout);
                    // The terminal runs full bleed on a flat surface; the scheme's own
                    // background is painted over it by the terminal view.
                    super::glass::pane(ui, area, 0.0, super::glass::Tier::Surface(pal.bg));
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
                        let suggestion = if settings.autocomplete
                            && i == tab.active
                            && !pane.session.line_untracked
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
                            // The active pane wears the heat line along its top edge.
                            let heat = motion::ease(
                                ui.ctx(),
                                egui::Id::unique(("pane-heat", pane.session.id)),
                                i == tab.active,
                                motion::HEAT,
                            );
                            ui.painter().rect_stroke(
                                *rect,
                                0.0,
                                Stroke::new(1.0, pal.border),
                                egui::StrokeKind::Inside,
                            );
                            if heat > 0.0 {
                                let half = rect.width() / 2.0 * heat;
                                ui.painter().hline(
                                    (rect.center().x - half)..=(rect.center().x + half),
                                    rect.top() + 1.0,
                                    Stroke::new(2.0, pal.accent),
                                );
                            }
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
                            actions
                                .push(TermAction::History(line, pane.session.target.label.clone()));
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
                                        alacritty_terminal::grid::Dimensions::screen_lines(grid)
                                            as i32
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
                            super::glass::card_frame(
                                &bui.ctx().clone(),
                                crate::themes::tokens::RADIUS_LARGE,
                            )
                            .fill(pal.panel.gamma_multiply(0.92))
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
        }

        if let Some(pane) = tab
            .panes
            .get(tab.active)
            .filter(|pane| pane.session.is_live())
        {
            for (key, tunnel) in &mut tab.containers.manager.tunnels {
                if let Some(url) = tunnel.take_browser_url() {
                    actions.push(TermAction::ContainerBrowser {
                        tab: tab_id,
                        session: pane.session.id,
                        key: key.clone(),
                        url,
                    });
                }
            }
        }

        // unslop-ignore N_PLUS_ONE_IN_LOOP -- Dispatch distinct UI actions; this is not a per-row DB/HTTP query loop.
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
                TermAction::EditLogPreset { host, edit } => {
                    if crate::containers::unified_logs::edit_presets(&mut self.vault.settings, host, edit) {
                        crate::store::touch_settings(&mut self.vault);
                        self.mark_dirty();
                        ui.ctx().request_repaint();
                    }
                }
                TermAction::ExportLogs(rows) => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Export logs")
                        .set_file_name("ferrum-logs.jsonl")
                        .add_filter("JSON Lines", &["jsonl"])
                        .save_file()
                    {
                        let bridge = self.bridge.clone();
                        self.rt.spawn_blocking(move || {
                            match crate::containers::unified_logs::export(&path, &rows) {
                                Ok(()) => bridge.toast(format!("Exported {} log lines.", rows.len()), false),
                                Err(error) => bridge.toast(format!("Log export failed. {error:#}"), true),
                            }
                        });
                    }
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
                    runtime,
                    client,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::list(&client, sudo, runtime)
                            .await
                            .map_err(|e| format!("{e:#}"));
                        bridge.send(AppEvent::Containers {
                            tab,
                            session,
                            runtime,
                            result,
                        });
                    });
                }
                TermAction::ContainerInventory {
                    tab,
                    session,
                    sudo,
                    client,
                    request,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::inventory::run(&client, &request, sudo)
                            .await
                            .map_err(|error| error.to_string());
                        bridge.send(AppEvent::ContainerInventory {
                            tab,
                            session,
                            request,
                            result,
                        });
                    });
                }
                TermAction::BrowseVolume {
                    tab,
                    session,
                    sudo,
                    client,
                    name,
                    directory,
                    revision,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::volume_files::browse(
                            &client, &name, &directory, sudo,
                        ).await.map_err(|error| format!("{error:#}"));
                        bridge.send(AppEvent::VolumeFiles { tab, session, revision, result });
                    });
                }
                TermAction::ExportVolume { sudo, client, name } => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Export Docker volume")
                        .set_file_name(format!("{name}.tar"))
                        .add_filter("Tar archive", &["tar"])
                        .save_file()
                    {
                        self.toast(format!("Exporting {name}…"), false);
                        let bridge = self.bridge.clone();
                        self.rt.spawn(async move {
                            match crate::containers::volume_files::export(&client, &name, &path, sudo).await {
                                Ok(_) => bridge.toast(format!("Exported {name}."), false),
                                Err(error) => bridge.toast(format!("Volume export failed. {error:#}"), true),
                            }
                        });
                    }
                }
                TermAction::EditVolume { tab, session, sudo, client, request } => {
                    let revision = request.revision;
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::volume_files::edit(&client, request, sudo)
                            .await.map_err(|error| format!("{error:#}"));
                        bridge.send(AppEvent::VolumeEdit { tab, session, revision, result });
                    });
                }
                TermAction::ImportVolume { tab, session, sudo, client, name } => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title(format!("Import into new volume {name}"))
                        .add_filter("Tar archives", &["tar", "gz", "tgz"])
                        .pick_file()
                        && let Some(tt) = self.container_tab(tab, session)
                    {
                        let page = tt.containers.manager.inventory
                            .page(crate::containers::inventory::Kind::Volumes);
                        page.pending = true;
                        page.notice = Some(Ok(format!("Importing {name}…")));
                        let bridge = self.bridge.clone();
                        self.rt.spawn(async move {
                            let result = crate::containers::volume_import::import(
                                &client, &name, &path, sudo,
                            ).await.map_err(|error| format!("{error:#}"));
                            bridge.send(AppEvent::VolumeImport { tab, session, name, result });
                        });
                    }
                }
                TermAction::UnifiedLogs {
                    tab,
                    session,
                    revision,
                    sudo,
                    client,
                    sources,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let snapshot = crate::containers::unified_logs::load(&client, sources, sudo).await;
                        bridge.send(AppEvent::UnifiedLogs {
                            tab,
                            session,
                            revision,
                            snapshot,
                        });
                    });
                }
                TermAction::ContainerResources {
                    tab,
                    session,
                    sudo,
                    client,
                    listing,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = crate::containers::manager::stats(&client, &listing, sudo)
                            .await
                            .map_err(|error| error.to_string());
                        bridge.send(AppEvent::ContainerResources {
                            tab,
                            session,
                            result,
                        });
                    });
                }
                TermAction::ContainerDetail {
                    tab,
                    session,
                    sudo,
                    client,
                    item,
                    sequence,
                    detail,
                } => {
                    let bridge = self.bridge.clone();
                    self.rt.spawn(async move {
                        let result = match detail {
                            crate::containers::manager::DetailTab::Logs => {
                                crate::containers::logs(&client, &item, sudo).await
                            }
                            crate::containers::manager::DetailTab::Ports => {
                                crate::containers::ports::fetch(&client, &item, sudo).await
                            }
                            crate::containers::manager::DetailTab::Inspect => {
                                crate::containers::manager::inspect(&client, &item, sudo).await
                            }
                            _ => return,
                        }
                        .map_err(|error| error.to_string());
                        bridge.send(AppEvent::ContainerDetail {
                            tab,
                            session,
                            key: item.key(),
                            sequence,
                            detail,
                            result,
                        });
                    });
                }
                TermAction::ContainerCredentials {
                    tab,
                    session,
                    key,
                    item,
                    client,
                    sudo,
                } => {
                    if let Some(Tab::Terminal(source)) =
                        self.tabs.iter_mut().find(|t| t.id() == tab)
                    {
                        if source.containers.session == session {
                            if let Some(tunnel) = source.containers.manager.tunnels.get_mut(&key) {
                                tunnel.read_credentials(
                                    self.rt.handle(),
                                    client,
                                    item,
                                    sudo,
                                    ui.ctx().clone(),
                                );
                            }
                        }
                    }
                }
                TermAction::ContainerOpenPort {
                    tab,
                    session,
                    item,
                    port,
                    mode,
                    client,
                    sudo,
                } => {
                    if let Some(Tab::Terminal(source)) =
                        self.tabs.iter_mut().find(|t| t.id() == tab)
                    {
                        if source.containers.session != session {
                            continue;
                        }
                        let tunnels = &mut source.containers.manager.tunnels;
                        let key = port.key(&item.key());
                        if tunnels.get(&key).is_some_and(|t| {
                            matches!(
                                t.status(),
                                crate::containers::ports::TunnelStatus::Starting
                                    | crate::containers::ports::TunnelStatus::Ready(_)
                            )
                        }) {
                            continue;
                        }
                        tunnels.retain(|_, t| {
                            matches!(
                                t.status(),
                                crate::containers::ports::TunnelStatus::Starting
                                    | crate::containers::ports::TunnelStatus::Ready(_)
                            )
                        });
                        if tunnels.len() >= 16 {
                            source.containers.notice = Some(Err("Stop a tunnel before opening another. Each SSH session supports up to 16 temporary tunnels.".into()));
                            continue;
                        }
                        let tunnel = crate::containers::ports::Tunnel::start(
                            self.rt.handle(),
                            client,
                            item,
                            port,
                            mode,
                            sudo,
                            ui.ctx().clone(),
                        );
                        tunnels.insert(key, tunnel);
                    }
                }
                TermAction::ContainerBrowser {
                    tab,
                    session,
                    key,
                    url,
                } => {
                    let valid = self
                        .tabs
                        .iter()
                        .find_map(|t| match t {
                            Tab::Terminal(source)
                                if source.id == tab && source.containers.session == session =>
                            {
                                source.containers.manager.tunnels.get(&key)
                            }
                            _ => None,
                        })
                        .is_some_and(|tunnel| match tunnel.status() {
                            crate::containers::ports::TunnelStatus::Ready(address) => [
                                crate::containers::ports::OpenAs::Http,
                                crate::containers::ports::OpenAs::Https,
                            ]
                            .iter()
                            .any(|mode| mode.url(address).as_deref() == Some(url.as_str())),
                            _ => false,
                        });
                    if !valid {
                        continue;
                    }
                    let result = open::that_detached(&url).map_err(|error| error.to_string());
                    if let Some(Tab::Terminal(source)) =
                        self.tabs.iter_mut().find(|t| t.id() == tab)
                    {
                        if source.containers.session == session {
                            if let Some(tunnel) = source.containers.manager.tunnels.get_mut(&key) {
                                tunnel.browser_error = result.err();
                            }
                        }
                    }
                }
                TermAction::ContainerTerminal {
                    mut target,
                    client,
                    item,
                    sudo,
                } => {
                    if let Some(command) = item.terminal_command(sudo) {
                        target.host_id = None;
                        target.label = format!("{} · {}", target.label, item.name);
                        target.startup_script = None;
                        target.exec_command = Some(command);
                        target.env.clear();
                        target.agent_forwarding = false;
                        let session = crate::conn::spawn_ssh_channel(
                            self.rt.handle(),
                            target,
                            self.conn_template(),
                            self.spawn_opts(),
                            client,
                        );
                        self.vault
                            .connection_logs
                            .push(crate::model::ConnectionLog {
                                id: session.log_id,
                                host_label: session.target.label.clone(),
                                address: session.target.address.clone(),
                                protocol: session.target.protocol,
                                started: crate::model::now_ts(),
                                ended: None,
                                status: "Connecting".into(),
                                log_file: session.term.lock().log_path.clone(),
                            });
                        if self.vault.connection_logs.len() > 1000 {
                            self.vault.connection_logs.remove(0);
                        }
                        self.mark_dirty();
                        self.open_terminal_session(session);
                    }
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
