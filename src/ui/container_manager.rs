//! Full-width container workspace, sharing the terminal sidebar's connection and state.
use std::collections::BTreeMap;

use egui::{RichText, Sense};
use egui_extras::{Column, TableBuilder};

use super::{container_view::Source, forms};
use crate::containers::{
    Action, Browser, Item, Runtime,
    manager::{DetailTab, Sample},
};
use crate::themes::Palette;

pub enum Command {
    Action(Item, Action),
    Terminal(Item),
    OpenPort(
        Item,
        crate::containers::ports::PublishedPort,
        crate::containers::ports::OpenAs,
    ),
    ReadCredentials(String, Item),
    Browser {
        key: String,
        url: String,
    },
}

pub fn show(
    ui: &mut egui::Ui,
    pal: Palette,
    browser: &mut Browser,
    source: Source,
) -> Option<Command> {
    let host = match source {
        Source::Ssh(host) => host,
        Source::Disconnected(host) => {
            forms::heading(ui, "Container manager", crate::themes::tokens::TITLE);
            ui.label(format!(
                "{host} is disconnected. Reconnect the SSH session to manage containers."
            ));
            if ui.add(forms::Btn::secondary("Back to terminal")).clicked() {
                browser.manager.open = false;
            }
            return None;
        }
        Source::Unsupported => {
            ui.label("Open an SSH session to manage remote containers.");
            if ui.add(forms::Btn::secondary("Back to terminal")).clicked() {
                browser.manager.open = false;
            }
            return None;
        }
    };
    ui.horizontal_wrapped(|ui| {
        forms::heading(ui, "Container manager", crate::themes::tokens::TITLE);
        ui.label(RichText::new(host).monospace().color(pal.text_dim));
        if ui
            .add(forms::Btn::secondary("Back to terminal").small())
            .clicked()
        {
            browser.manager.open = false;
        }
    });
    feedback(ui, pal, browser);
    let compact = ui.available_height() < 430.0;
    if (compact || browser.manager.tab == DetailTab::Ports)
        && browser.manager.focus_details
        && browser.manager.selected.is_some()
    {
        if ui
            .add(forms::Btn::secondary("All containers").small())
            .clicked()
        {
            browser.manager.focus_details = false;
        }
        return details(ui, pal, browser);
    }
    ui.horizontal_wrapped(|ui| {
        forms::Input::bare("Search containers")
            .hint("Search containers, images or projects")
            .icon(crate::icons::MAGNIFYING_GLASS)
            .width(320.0)
            .show(ui, &mut browser.filter);
        if ui
            .add_enabled(
                !browser.in_flight,
                forms::Btn::secondary("Refresh containers").small(),
            )
            .clicked()
        {
            browser.last_poll = f64::NEG_INFINITY;
            browser.manager.last_stats_poll = f64::NEG_INFINITY;
        }
        if browser.in_flight {
            ui.spinner();
        }
    });
    let Some(listing) = &browser.listing else {
        ui.label("Loading Docker, Podman and Kubernetes containers…");
        return None;
    };
    for (runtime, error) in &listing.errors {
        ui.colored_label(pal.warning, format!("{}: {error}", runtime.label()));
        if crate::containers::is_permission_error(error)
            && !browser.use_sudo
            && ui
                .add(forms::Btn::secondary("Retry with sudo").small())
                .clicked()
        {
            browser.use_sudo = true;
            browser.last_poll = f64::NEG_INFINITY;
            browser.manager.last_stats_poll = f64::NEG_INFINITY;
            browser.manager.invalidate_detail();
        }
    }
    let running = listing
        .items
        .iter()
        .filter(|item| item.is_running())
        .count();
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "{} containers and pods · {running} running",
            listing.items.len()
        ));
        if let Some(updated) = browser.manager.stats_updated {
            let age = updated.elapsed().as_secs();
            ui.label(
                RichText::new(format!(
                    "Resources updated {age}s ago{}",
                    if age > 15 { " · stale" } else { "" }
                ))
                .small()
                .color(if age > 15 { pal.warning } else { pal.text_dim }),
            );
        }
    });
    if let Some(error) = &browser.manager.stats_error {
        ui.colored_label(pal.warning, format!("Resources unavailable: {error}"));
    }
    let query = browser.filter.to_lowercase();
    let mut groups: BTreeMap<String, Vec<Item>> = BTreeMap::new();
    for item in &listing.items {
        let project = item
            .project
            .as_deref()
            .unwrap_or(if item.runtime == Runtime::Kubernetes {
                &item.scope
            } else {
                "Standalone"
            });
        if !query.is_empty()
            && ![
                item.name.as_str(),
                item.detail.as_str(),
                project,
                item.service.as_deref().unwrap_or(""),
            ]
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
        {
            continue;
        }
        groups
            .entry(format!("{} / {project}", item.runtime.label()))
            .or_default()
            .push(item.clone());
    }
    let mut rows = Vec::new();
    for (name, mut items) in groups {
        items.sort_by(|a, b| a.name.cmp(&b.name));
        rows.push(Row::Group(name, items.len()));
        rows.extend(items.into_iter().map(Row::Container));
    }
    if rows.is_empty() {
        ui.label(if listing.items.is_empty() {
            "No containers found on this server."
        } else {
            "No containers match your search."
        });
        if !browser.filter.is_empty() && ui.add(forms::Btn::ghost("Clear search")).clicked() {
            browser.filter.clear();
        }
    }
    let height = if compact {
        (ui.available_height() - 40.0).max(80.0)
    } else {
        // The details below need about 330 points for a few log lines; the table gives way.
        (ui.available_height() * 0.4)
            .min(ui.available_height() - 330.0)
            .clamp(80.0, 280.0)
    };
    egui::ScrollArea::horizontal()
        .id_salt("container-table-horizontal")
        .show(ui, |ui| {
            ui.set_min_width(820.0);
            // A soft wash for the selected row, as in the host list, not a solid slab.
            ui.visuals_mut().selection.bg_fill = pal.accent.gamma_multiply(0.14);
            ui.visuals_mut().widgets.hovered.weak_bg_fill = super::glass::hover_wash(ui);
            TableBuilder::new(ui)
                .id_salt("container-table")
                .striped(false)
                .sense(Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::remainder().at_least(180.0).clip(true))
                .column(Column::remainder().at_least(180.0).clip(true))
                .column(Column::initial(110.0).at_least(90.0).clip(true))
                .column(Column::exact(70.0))
                .column(Column::initial(130.0).at_least(90.0).clip(true))
                .column(Column::initial(140.0).at_least(80.0).clip(true))
                .max_scroll_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .header(28.0, |mut header| {
                    for title in [
                        "NAME / PROJECT",
                        "IMAGE",
                        "STATE / HEALTH",
                        "CPU",
                        "MEMORY",
                        "PORTS",
                    ] {
                        header.col(|ui| {
                            ui.label(RichText::new(title).small().color(pal.text_dim));
                        });
                    }
                })
                .body(|body| {
                    // Group titles sit close to their rows; containers keep a full row.
                    let heights = rows.iter().map(|row| match row {
                        Row::Group(..) => 30.0,
                        Row::Container(_) => 36.0,
                    });
                    body.heterogeneous_rows(heights, |mut row| match &rows[row.index()] {
                        Row::Group(name, count) => {
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(name)
                                        .size(13.0)
                                        .family(crate::fonts::semibold())
                                        .color(pal.text_dim),
                                );
                                ui.label(
                                    RichText::new(count.to_string()).size(13.0).color(pal.faint),
                                );
                            });
                            for _ in 0..5 {
                                row.col(|_| {});
                            }
                        }
                        Row::Container(item) => {
                            let selected = browser.manager.selected.as_deref() == Some(&item.key());
                            row.set_selected(selected);
                            row.col(|ui| {
                                // The whole row selects; the name carries the same marker as a
                                // selected host instead of a chip of its own.
                                if selected {
                                    let r = ui.max_rect();
                                    ui.painter().rect_filled(
                                        egui::Rect::from_min_size(
                                            egui::pos2(r.left(), r.center().y - 10.0),
                                            egui::vec2(3.0, 20.0),
                                        ),
                                        1.5,
                                        pal.accent,
                                    );
                                }
                                ui.add_space(12.0);
                                ui.add(
                                    egui::Label::new(RichText::new(&item.name).color(pal.text))
                                        .selectable(false)
                                        .truncate(),
                                )
                                .on_hover_text(format!("{}\n{}", item.name, item.scope));
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(&item.detail).color(pal.text))
                                    .on_hover_text(&item.detail);
                            });
                            row.col(|ui| {
                                ui.label(RichText::new(health(item)).color(
                                    if item.is_failing() || item.status.contains("unhealthy") {
                                        pal.danger
                                    } else {
                                        pal.text
                                    },
                                ));
                            });
                            let sample = browser
                                .manager
                                .stats
                                .get(&item.key())
                                .and_then(|history| history.back());
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(
                                        sample
                                            .and_then(|s| s.stats.cpu)
                                            .map(|cpu| format!("{cpu:.1}%"))
                                            .unwrap_or_else(|| "—".into()),
                                    )
                                    .color(pal.steel),
                                );
                            });
                            row.col(|ui| {
                                ui.label(
                                    RichText::new(
                                        sample.map(|s| s.stats.memory.as_str()).unwrap_or("—"),
                                    )
                                    .color(pal.steel),
                                );
                            });
                            row.col(|ui| {
                                ui.label(&item.ports).on_hover_text(&item.ports);
                            });
                            if row.response().clicked() {
                                browser.manager.select(item.key());
                            }
                        }
                    });
                });
        });
    ui.separator();
    if compact {
        None
    } else {
        details(ui, pal, browser)
    }
}

enum Row {
    Group(String, usize),
    Container(Item),
}

fn feedback(ui: &mut egui::Ui, pal: Palette, browser: &Browser) {
    if let Some(error) = &browser.error {
        ui.colored_label(pal.warning, format!("Last update failed: {error}"));
    }
    if let Some(notice) = &browser.notice {
        match notice {
            Ok(text) => {
                ui.label(text);
            }
            Err(error) => {
                ui.colored_label(pal.danger, error);
            }
        }
    }
    if let Some(busy) = &browser.busy {
        ui.label(busy);
    }
}

fn health(item: &Item) -> &str {
    if item.status.contains("unhealthy") {
        "Unhealthy"
    } else if item.status.contains("(healthy)") {
        "Healthy"
    } else {
        &item.state
    }
}

fn details(ui: &mut egui::Ui, pal: Palette, browser: &mut Browser) -> Option<Command> {
    let item = browser
        .listing
        .as_ref()
        .and_then(|list| {
            list.items
                .iter()
                .find(|item| browser.manager.selected.as_deref() == Some(&item.key()))
        })
        .cloned();
    let Some(item) = item else {
        ui.label("Select a container to view its logs, resources and configuration.");
        return None;
    };
    let mut command = None;
    ui.horizontal_wrapped(|ui| {
        forms::heading(ui, &item.name, 16.0);
        ui.label(
            RichText::new(item.service.as_deref().unwrap_or(&item.detail))
                .small()
                .color(pal.text_dim),
        );
        if item.runtime != Runtime::Kubernetes {
            let actions: &[Action] = if item.is_running() {
                &[Action::Stop, Action::Restart]
            } else {
                &[Action::Start]
            };
            for &action in actions {
                if ui
                    .add_enabled(
                        browser.busy.is_none(),
                        forms::Btn::secondary(action.label()).small(),
                    )
                    .clicked()
                {
                    command = Some(Command::Action(item.clone(), action));
                }
            }
        }
    });
    ui.add_space(4.0);
    let mut tab = browser.manager.tab;
    if forms::segmented_tabs(
        ui,
        "container-detail-tab",
        &mut tab,
        &[
            (DetailTab::Logs, "Logs"),
            (DetailTab::Stats, "Stats"),
            (DetailTab::Inspect, "Inspect"),
            (DetailTab::Terminal, "Terminal"),
            (DetailTab::Ports, "Ports"),
        ],
    ) {
        browser.manager.choose_tab(tab);
    }
    ui.add_space(4.0);
    match browser.manager.tab {
        DetailTab::Logs => {
            ui.horizontal_wrapped(|ui| {
                let label = if browser.manager.pause_logs {
                    "Resume logs"
                } else {
                    "Pause logs"
                };
                if ui.add(forms::Btn::secondary(label).small()).clicked() {
                    browser.manager.pause_logs = !browser.manager.pause_logs;
                    browser.manager.last_detail_poll = f64::NEG_INFINITY;
                }
                ui.label(if browser.manager.pause_logs {
                    "Paused · last 300 lines"
                } else {
                    "Refreshes every 2 seconds · last 300 lines"
                });
                if let Some(at) = browser.manager.logs_updated {
                    ui.label(format!("Updated {}s ago", at.elapsed().as_secs()));
                }
            });
            forms::Input::bare("Search log lines")
                .hint("Search log lines")
                .show(ui, &mut browser.manager.log_filter);
            let query = browser.manager.log_filter.to_lowercase();
            match &browser.manager.logs {
                Some(Ok(logs)) => {
                    super::glass::well_frame(ui.ctx(), 10)
                        .inner_margin(egui::Margin::same(10))
                        .show(ui, |ui| {
                            // Leaves room for the well's own margin so the logs end inside the page.
                            egui::ScrollArea::both()
                                .id_salt(("container-live-logs", item.key()))
                                .max_height((ui.available_height() - 24.0).max(40.0))
                                .auto_shrink([false, false])
                                .stick_to_bottom(!browser.manager.pause_logs)
                                .show(ui, |ui| {
                                    let filtered = logs
                                        .lines()
                                        .filter(|line| {
                                            query.is_empty() || line.to_lowercase().contains(&query)
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    if filtered.is_empty() {
                                        ui.label("No matching log lines.");
                                    } else {
                                        ui.add(
                                            egui::Label::new(RichText::new(filtered).monospace())
                                                .selectable(true),
                                        );
                                    }
                                });
                        });
                }
                Some(Err(error)) => {
                    ui.colored_label(pal.danger, error);
                }
                None if browser.manager.pause_logs => {
                    ui.label("Logs paused. Resume to load log lines.");
                }
                None => {
                    ui.spinner();
                    ui.label("Loading logs…");
                }
            }
        }
        DetailTab::Inspect => {
            ui.horizontal_wrapped(|ui| {
                ui.label("Configuration, mounts, networks and ports. Environment values and command arguments stay hidden.");
                if ui.add_enabled(browser.manager.detail_pending.is_none(), forms::Btn::secondary("Refresh inspect").small()).clicked() {
                    browser.manager.inspect = None; browser.manager.invalidate_detail();
                }
            });
            if item.runtime == Runtime::Kubernetes {
                ui.label("Inspect is available for Docker and Podman containers.");
            } else {
                match &browser.manager.inspect {
                    Some(Ok(text)) => {
                        egui::ScrollArea::both()
                            .id_salt(("container-inspect", item.key()))
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(text).monospace())
                                        .selectable(true),
                                );
                            });
                    }
                    Some(Err(error)) => {
                        ui.colored_label(pal.danger, error);
                    }
                    None => {
                        ui.spinner();
                        ui.label("Loading configuration…");
                    }
                }
            }
        }
        DetailTab::Stats => {
            let samples = browser.manager.stats.get(&item.key());
            if let Some(samples) = samples.filter(|samples| !samples.is_empty()) {
                let latest = &samples[samples.len() - 1];
                ui.label(format!(
                    "Sampled {}s ago · up to 60 samples · CPU can exceed 100% on multiple cores",
                    latest.at.elapsed().as_secs()
                ));
                egui::ScrollArea::vertical()
                    .id_salt("container-resource-details")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.columns(2, |columns| {
                            chart(&mut columns[0], pal, "CPU", samples, true);
                            chart(&mut columns[1], pal, "Memory", samples, false);
                        });
                        ui.label(format!(
                            "Memory: {} · Processes: {}",
                            latest.stats.memory, latest.stats.pids
                        ));
                        ui.label(format!(
                            "Network I/O: {} · Disk I/O: {}",
                            latest.stats.network, latest.stats.disk
                        ));
                    });
            } else {
                ui.label(if item.runtime == Runtime::Kubernetes {
                    "Pod resource metrics are not available in this view."
                } else if !item.is_running() {
                    "Start the container to collect resource samples."
                } else {
                    "Waiting for a resource sample…"
                });
            }
        }
        DetailTab::Ports => {
            command = port_view(ui, pal, browser, &item);
        }
        DetailTab::Terminal => {
            ui.label("Open an interactive shell in a dedicated terminal tab. The host terminal stays available.");
            if let Some(command_text) = item.terminal_command(browser.use_sudo) {
                ui.add(egui::Label::new(RichText::new(command_text).monospace()).wrap());
                if ui
                    .add_enabled(
                        item.is_running(),
                        forms::Btn::primary("Open container terminal"),
                    )
                    .clicked()
                {
                    command = Some(Command::Terminal(item));
                }
            }
        }
    }
    command
}

fn port_view(
    ui: &mut egui::Ui,
    pal: Palette,
    browser: &mut Browser,
    item: &Item,
) -> Option<Command> {
    use crate::containers::ports::{OpenAs, TunnelStatus};
    if item.runtime == Runtime::Kubernetes {
        ui.label("Published-port tunnels support Docker and Podman containers.");
        return None;
    }
    let mut command = None;
    ui.horizontal_wrapped(|ui| {
        ui.label("Open as:");
        for (mode, label) in [
            (OpenAs::Http, "HTTP"),
            (OpenAs::Https, "HTTPS"),
            (OpenAs::Tcp, "TCP only"),
        ] {
            ui.selectable_value(&mut browser.manager.port_mode, mode, label);
        }
        if ui
            .add_enabled(
                browser.manager.detail_pending.is_none(),
                forms::Btn::secondary("Refresh ports").small(),
            )
            .clicked()
        {
            browser.manager.ports = None;
            browser.manager.invalidate_detail();
        }
    });
    ui.label("Uses a free local loopback port. Database and messaging presets use TCP. Stop closes connections.");
    egui::ScrollArea::vertical().id_salt("container-ports").auto_shrink([false, false]).show(ui, |ui| {
        match &browser.manager.ports {
            None => { ui.spinner(); ui.label("Loading published ports…"); }
            Some(Err(error)) => { ui.colored_label(pal.danger, error); }
            Some(Ok(ports)) if ports.is_empty() => { ui.label("No published ports. Publish a TCP port on the SSH server to open it locally."); }
            Some(Ok(ports)) => {
                for port in ports {
                    let key = port.key(&item.key());
                    ui.push_id(&key, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("{} → {}/{}", std::net::SocketAddr::new(port.host, port.host_port), port.container_port, port.protocol));
                            ui.label(format!("Preset: {}", crate::containers::services::detect(&item.detail, port.container_port).label()));
                            let status = browser.manager.tunnels.get(&key).map(|t| t.status());
                            let active = matches!(status, Some(TunnelStatus::Starting | TunnelStatus::Ready(_)));
                            if ui.add_enabled(port.protocol == "tcp" && item.is_running() && !active, forms::Btn::secondary("Open locally").small())
                                .on_disabled_hover_text(if port.protocol != "tcp" { "SSH forwards TCP ports only." } else if active { "Use the active tunnel below." } else { "Start the container first." }).clicked() {
                                let detected = crate::containers::services::detect(&item.detail, port.container_port);
                                let mode = if detected.mode() == OpenAs::Tcp { OpenAs::Tcp }
                                    else if browser.manager.port_mode == OpenAs::Http && matches!(port.container_port,443 | 8443) { OpenAs::Https }
                                    else { browser.manager.port_mode };
                                command = Some(Command::OpenPort(item.clone(), port.clone(), mode));
                            }
                        });
                    });
                }
            }
        }
        for (key, tunnel) in browser.manager.tunnels.iter_mut().filter(|(_, t)| t.owner == item.key()) {
            ui.push_id(("tunnel", key), |ui| {
                ui.separator();
                ui.label(format!("Container {}/{} via {}", tunnel.port.container_port, tunnel.port.protocol, std::net::SocketAddr::new(tunnel.port.destination(), tunnel.port.host_port)));
                match tunnel.status() {
                    TunnelStatus::Starting => { ui.spinner(); ui.label("Checking the remote port and starting the tunnel…"); }
                    TunnelStatus::Ready(address) => {
                        ui.label(format!("Ready · {address}"));
                        ui.horizontal_wrapped(|ui| {
                            if ui.add(forms::Btn::secondary("Copy address").small()).clicked() { ui.ctx().copy_text(address.to_string()); }
                            if let Some(url) = tunnel.mode.url(address) {
                                if ui.add(forms::Btn::primary("Open browser").small()).clicked() { command = Some(Command::Browser { key: key.clone(), url }); }
                            }
                        });
                        tunnel.poll_credentials();
                        if connection_view(ui, &mut tunnel.connection, address, tunnel.credentials_pending) {
                            command = Some(Command::ReadCredentials(key.clone(), item.clone()));
                        }
                        if let Some(error) = tunnel.connection_error() { ui.colored_label(pal.warning, error); }
                        if let Some(error) = &tunnel.browser_error { ui.colored_label(pal.warning, format!("Browser did not open: {error}. Copy the address to open it yourself.")); }
                    }
                    TunnelStatus::Failed(error) => { ui.colored_label(pal.danger, error); }
                    TunnelStatus::Stopped => { ui.label("Tunnel stopped."); }
                }
                if matches!(tunnel.status(), TunnelStatus::Starting | TunnelStatus::Ready(_)) && ui.add(forms::Btn::secondary("Stop tunnel").small()).clicked() { tunnel.stop(); }
            });
        }
    });
    command
}

fn connection_view(
    ui: &mut egui::Ui,
    connection: &mut crate::containers::services::Connection,
    address: std::net::SocketAddr,
    pending: bool,
) -> bool {
    use crate::containers::services::{Connection, Service};
    let mut read = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Connection preset");
        let old = connection.service;
        ui.add_enabled_ui(!pending, |ui| {
            egui::ComboBox::from_id_salt("connection-preset")
                .selected_text(connection.service.label())
                .show_ui(ui, |ui| {
                    for service in Service::ALL {
                        ui.selectable_value(&mut connection.service, service, service.label());
                    }
                });
        });
        if connection.service != old {
            *connection = Connection::new(connection.service, 0);
        }
        if !connection.service.env_keys().is_empty()
            && ui
                .add_enabled(!pending, forms::Btn::secondary("Read credentials").small())
                .clicked()
        {
            read = true;
        }
        if pending {
            ui.spinner();
        }
    });
    if connection.service.has_credentials() {
        egui::Grid::new("connection-fields")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(if connection.service == Service::S3 {
                    "Access key"
                } else {
                    "User"
                });
                ui.add(egui::TextEdit::singleline(&mut connection.user).desired_width(220.0));
                ui.end_row();
                ui.label(if connection.service == Service::S3 {
                    "Secret key"
                } else {
                    "Password"
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut connection.password)
                            .password(!connection.reveal)
                            .desired_width(170.0),
                    );
                    ui.checkbox(&mut connection.reveal, "Show");
                });
                ui.end_row();
                if !matches!(
                    connection.service,
                    Service::S3 | Service::Http | Service::Https
                ) {
                    ui.label(if connection.service == Service::Amqp {
                        "Virtual host"
                    } else {
                        "Database"
                    });
                    ui.add(
                        egui::TextEdit::singleline(&mut connection.database).desired_width(220.0),
                    );
                    ui.end_row();
                }
                if connection.service == Service::Mongo {
                    ui.label("Auth database");
                    ui.add(
                        egui::TextEdit::singleline(&mut connection.auth_source)
                            .desired_width(220.0),
                    );
                    ui.end_row();
                }
            });
        if matches!(
            connection.service,
            Service::Postgres
                | Service::Mongo
                | Service::Redis
                | Service::ClickHouse
                | Service::ClickHouseHttp
                | Service::Amqp
                | Service::S3
        ) {
            ui.checkbox(&mut connection.tls, "Service requires TLS");
        }
        ui.label(&connection.note);
    }
    // Preview and ordinary clipboard action never contain the password.
    ui.add(
        egui::Label::new(egui::RichText::new(connection.uri(address, false)).monospace()).wrap(),
    );
    ui.horizontal_wrapped(|ui| {
        if ui.add(forms::Btn::secondary("Copy URI").small()).clicked() {
            ui.ctx().copy_text(connection.uri(address, false));
        }
        if !connection.password.is_empty()
            && connection.service != Service::S3
            && ui
                .add(forms::Btn::secondary("Copy with password").small())
                .clicked()
        {
            ui.ctx().copy_text(connection.uri(address, true));
        }
        if connection.service == Service::S3
            && ui
                .add(forms::Btn::secondary("Copy S3 config with secret").small())
                .clicked()
        {
            ui.ctx().copy_text(connection.s3_config(address, true));
        }
    });
    read
}

fn chart(
    ui: &mut egui::Ui,
    pal: Palette,
    title: &str,
    samples: &std::collections::VecDeque<Sample>,
    cpu: bool,
) {
    let values: Vec<f64> = samples
        .iter()
        .filter_map(|s| {
            if cpu {
                s.stats.cpu
            } else {
                s.stats.memory_percent
            }
        })
        .collect();
    ui.label(format!(
        "{title} · {}",
        values
            .last()
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "Unavailable".into())
    ));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 64.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, crate::themes::tokens::RADIUS, pal.well);
    if values.len() < 2 {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Collecting history…",
            egui::FontId::proportional(12.0),
            pal.text_dim,
        );
        return;
    }
    let ceiling = values.iter().copied().fold(100.0, f64::max);
    let points = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            egui::pos2(
                rect.left() + rect.width() * i as f32 / (values.len() - 1) as f32,
                rect.bottom() - 4.0 - (rect.height() - 8.0) * (v / ceiling).clamp(0.0, 1.0) as f32,
            )
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, egui::Stroke::new(1.5, pal.steel)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, Rect, Vec2};

    fn fixture() -> Browser {
        let mut browser = Browser::default();
        browser.manager.open = true;
        browser.listing = Some(crate::containers::parse(
            r#"@docker
{"ID":"abcdef123456","Names":"shop-api","Image":"nginx:1.27","State":"running","Status":"Up 2 hours (healthy)","Labels":"com.docker.compose.project=shop,com.docker.compose.service=api"}
{"ID":"112233445566","Names":"batch-worker","Image":"alpine:3.21","State":"exited","Status":"Exited (0)"}
"#,
        ));
        browser
    }

    fn text_position(shape: &egui::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_position(s, label)),
            _ => None,
        }
    }
    fn text(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Text(t) => {
                out.push_str(t.galley.text());
                out.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text(shape, out);
                }
            }
            _ => {}
        }
    }
    fn render(
        ctx: &egui::Context,
        browser: &mut Browser,
        size: Vec2,
        events: Vec<Event>,
    ) -> (egui::FullOutput, Option<Command>) {
        let mut command = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| {
                command = show(
                    ui,
                    crate::themes::current(ctx),
                    browser,
                    Source::Ssh("test-host"),
                );
            },
        );
        output.textures_delta.clear();
        (output, command)
    }
    fn click(
        ctx: &egui::Context,
        browser: &mut Browser,
        size: Vec2,
        label: &str,
    ) -> Option<Command> {
        render(ctx, browser, size, vec![]);
        let (output, _) = render(ctx, browser, size, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, label))
            .unwrap_or_else(|| panic!("Missing visible control: {label}"));
        render(
            ctx,
            browser,
            size,
            vec![
                Event::PointerMoved(point),
                Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        render(
            ctx,
            browser,
            size,
            vec![Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        )
        .1
    }
    fn rendered_text(output: &egui::FullOutput) -> String {
        let mut out = String::new();
        for shape in &output.shapes {
            text(&shape.shape, &mut out);
        }
        out
    }

    #[test]
    fn search_selection_and_terminal_action_use_the_selected_container() {
        let mut browser = fixture();
        let ctx = egui::Context::default();
        let size = Vec2::new(1400.0, 900.0);
        click(
            &ctx,
            &mut browser,
            size,
            "Search containers, images or projects",
        );
        let (output, _) = render(
            &ctx,
            &mut browser,
            size,
            vec![Event::Text("no-such-container".into())],
        );
        assert_eq!(browser.filter, "no-such-container");
        assert!(rendered_text(&output).contains("No containers match your search."));
        click(&ctx, &mut browser, size, "Clear search");
        assert_eq!(browser.filter, "");
        click(&ctx, &mut browser, size, "shop-api");
        assert_eq!(
            browser.manager.selected.as_deref(),
            Some("Docker/abcdef123456/shop-api")
        );
        click(&ctx, &mut browser, size, "Inspect");
        assert_eq!(browser.manager.tab, DetailTab::Inspect);
        click(&ctx, &mut browser, size, "Terminal");
        let command = click(&ctx, &mut browser, size, "Open container terminal");
        let Some(Command::Terminal(item)) = command else {
            panic!("Expected a container terminal action");
        };
        assert_eq!(item.scope, "abcdef123456");
        let (output, _) = render(&ctx, &mut browser, size, vec![]);
        assert!(!rendered_text(&output).contains("use of widget ID"));
        click(&ctx, &mut browser, size, "Back to terminal");
        assert!(!browser.manager.open);
    }

    #[test]
    fn compact_layout_can_return_to_the_same_container_details() {
        let mut browser = fixture();
        let ctx = egui::Context::default();
        let size = Vec2::new(760.0, 380.0);
        click(&ctx, &mut browser, size, "shop-api");
        assert!(browser.manager.focus_details);
        click(&ctx, &mut browser, size, "All containers");
        assert!(!browser.manager.focus_details);
        click(&ctx, &mut browser, size, "shop-api");
        assert!(
            browser.manager.focus_details,
            "selecting the same container must reopen its details"
        );
        click(&ctx, &mut browser, size, "Terminal");
        assert!(matches!(
            click(&ctx, &mut browser, size, "Open container terminal"),
            Some(Command::Terminal(_))
        ));
    }

    #[test]
    fn ports_controls_capture_the_binding_and_selected_protocol() {
        let mut browser = fixture();
        browser
            .manager
            .select("Docker/abcdef123456/shop-api".into());
        browser.manager.tab = DetailTab::Ports;
        browser.manager.ports = Some(Ok(crate::containers::ports::parse(
            r#"{"80/tcp":[{"HostIp":"0.0.0.0","HostPort":"8080"}]}"#,
        )
        .unwrap()));
        let ctx = egui::Context::default();
        let size = Vec2::new(1400.0, 900.0);
        click(&ctx, &mut browser, size, "HTTPS");
        let Some(Command::OpenPort(item, port, mode)) =
            click(&ctx, &mut browser, size, "Open locally")
        else {
            panic!("Expected a published-port action");
        };
        assert_eq!(item.scope, "abcdef123456");
        assert_eq!(port.host_port, 8080);
        assert_eq!(port.container_port, 80);
        assert_eq!(mode, crate::containers::ports::OpenAs::Https);
        click(&ctx, &mut browser, size, "TCP only");
        assert!(matches!(
            click(&ctx, &mut browser, size, "Open locally"),
            Some(Command::OpenPort(
                _,
                _,
                crate::containers::ports::OpenAs::Tcp
            ))
        ));
        let (output, _) = render(&ctx, &mut browser, size, vec![]);
        assert!(!rendered_text(&output).contains("use of widget ID"));
    }

    #[test]
    fn sidebar_exposes_the_full_manager_entry_point() {
        let mut browser = fixture();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            super::super::container_view::show(
                ui,
                crate::themes::current(&ctx),
                &mut browser,
                Source::Ssh("test-host"),
            );
        });
        output.textures_delta.clear();
        assert!(rendered_text(&output).contains("Open manager"));
    }
    #[test]
    fn connection_form_hides_password_and_copies_only_on_explicit_action() {
        use crate::containers::services::{Connection, Service};
        let ctx = egui::Context::default();
        let address = "127.0.0.1:43210".parse().unwrap();
        let mut connection = Connection::new(Service::Postgres, 5432);
        connection.user = "reader".into();
        connection.password = "synthetic-secret".into();
        connection.database = "events".into();
        let render = |connection: &mut Connection, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(700.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    connection_view(ui, connection, address, false);
                },
            );
            output.textures_delta.clear();
            output
        };
        for (button, expected) in [
            ("Copy URI", "postgresql://reader@127.0.0.1:43210/events"),
            (
                "Copy with password",
                "postgresql://reader:synthetic-secret@127.0.0.1:43210/events",
            ),
        ] {
            render(&mut connection, vec![]);
            let output = render(&mut connection, vec![]);
            assert!(!rendered_text(&output).contains("synthetic-secret"));
            let point = output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, button))
                .unwrap();
            render(
                &mut connection,
                vec![
                    Event::PointerMoved(point),
                    Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
            );
            let output = render(
                &mut connection,
                vec![Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            let copied: Vec<_> = output
                .platform_output
                .commands
                .iter()
                .filter_map(|command| match command {
                    egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(copied, vec![expected]);
            assert!(!rendered_text(&output).contains("synthetic-secret"));
        }
    }
}
