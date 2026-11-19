//! Full-width container workspace, sharing the terminal sidebar's connection and state.
use std::collections::BTreeMap;

use egui::{Color32, RichText, Sense, Ui};

use super::{container_view::Source, forms, library_list};
use crate::containers::{
    Action, Browser, Item, Runtime, StateFilter,
    inventory::{Kind, Request},
    manager::{DetailTab, Sample, Section},
};
use crate::themes::Palette;

pub enum Command {
    Inventory(Request),
    UnifiedLogs {
        revision: u64,
        sources: Vec<crate::containers::unified_logs::Source>,
    },
    EditLogPreset(crate::containers::unified_logs::PresetEdit),
    ExportLogs(Vec<crate::containers::unified_logs::Row>),
    BrowseVolume {
        name: String,
        directory: Vec<Vec<u8>>,
        revision: u64,
    },
    ExportVolume(String),
    EditVolume(crate::containers::volume_files::EditRequest),
    ImportVolume(String),
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
    log_presets: Option<&[crate::model::LogFilterPreset]>,
) -> Option<Command> {
    let confirmation_id = ui.make_persistent_id(("container-action", browser.session));
    let host = match source {
        Source::Ssh(host) => host,
        Source::Connecting(host) => {
            forms::heading(ui, "Container Manager", crate::themes::tokens::TITLE);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Connecting to {host}…"));
            });
            if ui
                .add(forms::Btn::ghost("Back to terminal").height(44.0))
                .clicked()
            {
                browser.manager.open = false;
            }
            return None;
        }
        Source::Disconnected(host) => {
            ui.data_mut(|data| data.remove::<(String, Action)>(confirmation_id));
            forms::heading(ui, "Container Manager", crate::themes::tokens::TITLE);
            ui.label(format!(
                "{host} is disconnected. Reconnect the SSH session to manage containers."
            ));
            if ui.add(forms::Btn::secondary("Back to terminal")).clicked() {
                browser.manager.open = false;
            }
            if let Some(listing) = &browser.listing {
                ui.label(
                    RichText::new(format!(
                        "Last known inventory · {} workloads",
                        listing.items.len()
                    ))
                    .color(pal.text_dim),
                );
            }
            return None;
        }
        Source::Unsupported => {
            ui.data_mut(|data| data.remove::<(String, Action)>(confirmation_id));
            ui.label("Open an SSH session to manage remote containers.");
            if ui.add(forms::Btn::secondary("Back to terminal")).clicked() {
                browser.manager.open = false;
            }
            return None;
        }
    };
    ui.horizontal_wrapped(|ui| {
        forms::heading(ui, "Container Manager", crate::themes::tokens::TITLE);
        ui.label(RichText::new("SSH connected").small().color(pal.success));
        ui.label(RichText::new(host).small().monospace().color(pal.text_dim));
        if ui
            .add(forms::Btn::secondary("Back to terminal").small())
            .clicked()
        {
            browser.manager.open = false;
            ui.data_mut(|data| data.remove::<(String, Action)>(confirmation_id));
        }
    });
    forms::segmented_tabs(
        ui,
        "container-manager-sections",
        &mut browser.manager.section,
        &[
            (Section::Containers, "Containers"),
            (Section::Inventory(Kind::Images), "Images"),
            (Section::Inventory(Kind::Volumes), "Volumes"),
            (Section::Inventory(Kind::Builds), "Builds"),
            (Section::Logs, "Unified logs"),
        ],
    );
    ui.separator();
    if let Section::Inventory(kind) = browser.manager.section {
        return ui
            .push_id(("container-inventory", browser.session), |ui| {
                super::container_inventory::show(
                    ui,
                    pal,
                    kind,
                    browser.manager.inventory.page(kind),
                    &mut browser.use_sudo,
                    host,
                )
            })
            .inner
            .map(|action| match action {
                super::container_inventory::Action::Inventory(request) => {
                    Command::Inventory(request)
                }
                super::container_inventory::Action::BrowseVolume {
                    name,
                    directory,
                    revision,
                } => Command::BrowseVolume {
                    name,
                    directory,
                    revision,
                },
                super::container_inventory::Action::ExportVolume(name) => {
                    Command::ExportVolume(name)
                }
                super::container_inventory::Action::EditVolume(request) => {
                    Command::EditVolume(request)
                }
                super::container_inventory::Action::ImportVolume(name) => {
                    Command::ImportVolume(name)
                }
            });
    }
    if browser.manager.section == Section::Logs {
        return super::container_logs::show(
            ui,
            pal,
            &mut browser.manager.unified_logs,
            browser.listing.as_ref(),
            browser.manager.inventory.page(Kind::Builds),
            log_presets,
        )
        .map(|action| match action {
            super::container_logs::Action::LoadBuilds => {
                browser.last_poll = f64::NEG_INFINITY;
                Command::Inventory(Request::List(Kind::Builds))
            }
            super::container_logs::Action::Load { revision, sources } => {
                Command::UnifiedLogs { revision, sources }
            }
            super::container_logs::Action::EditPreset(edit) => Command::EditLogPreset(edit),
            super::container_logs::Action::Export(rows) => Command::ExportLogs(rows),
        });
    }
    if let Some(command) = confirm_action(ui, pal, browser, host, confirmation_id) {
        return Some(command);
    }
    feedback(ui, pal, browser);
    let compact = compact_workspace(ui);
    if compact && browser.manager.focus_details && browser.manager.selected.is_some() {
        return details(ui, pal, browser, confirmation_id, true);
    }
    if compact
        || browser
            .listing
            .as_ref()
            .is_none_or(|listing| listing.items.is_empty())
    {
        workload_list(ui, pal, browser);
        None
    } else {
        let (mut list, mut inspector) = split_workspace(ui, pal, 0.38, 340.0);
        if workload_list(&mut list, pal, browser) {
            details(&mut inspector, pal, browser, confirmation_id, false)
        } else {
            select_workload(&mut inspector, pal);
            None
        }
    }
}

pub(super) fn compact_workspace(ui: &Ui) -> bool {
    ui.available_width() < 820.0 || ui.available_height() < 430.0
}

/// Reserve the workspace once, so the table and detail output can scroll independently.
pub(super) fn split_workspace(ui: &mut Ui, pal: Palette, share: f32, min_right: f32) -> (Ui, Ui) {
    let (area, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let boundary = area.right() - (area.width() * share).max(min_right);
    let left = egui::Rect::from_min_max(area.min, egui::pos2(boundary - 12.0, area.bottom()));
    let right = egui::Rect::from_min_max(
        egui::pos2(boundary + 12.0, area.top()),
        area.max - egui::vec2(8.0, 0.0),
    );
    let surface = egui::Rect::from_min_max(egui::pos2(boundary, area.top()), area.max);
    super::glass::pane(ui, surface, 8.0, super::glass::Tier::Surface(pal.panel));
    ui.painter()
        .vline(boundary, area.y_range(), egui::Stroke::new(1.0, pal.border));
    let mut pane = |id, rect: egui::Rect| {
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
        child.set_clip_rect(ui.clip_rect().intersect(rect));
        child
    };
    (pane("workspace-left", left), pane("workspace-right", right))
}

fn request_row_focus(ui: &Ui, key: Option<&str>) {
    if let Some(key) = key {
        ui.data_mut(|data| {
            data.insert_temp(ui.scope_id().with("manager-row-focus"), key.to_owned())
        });
    }
}

pub(super) fn row_label(
    ui: &mut Ui,
    name: &str,
    secondary: Option<&str>,
    selected: bool,
    pal: Palette,
) -> egui::Response {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.scope_id().with("manager-row"), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, name)
    });
    if selected || response.has_focus() {
        let marker = egui::Rect::from_min_size(rect.left_top(), egui::vec2(4.0, rect.height()));
        ui.painter().rect_filled(marker, 0.0, pal.accent);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            4.0,
            egui::Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
        if ui.input(|input| input.key_pressed(egui::Key::Tab)) {
            response.scroll_to_me(None);
        }
    }
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(name)
                        .family(crate::fonts::semibold())
                        .color(pal.text),
                )
                .truncate()
                .selectable(false),
            );
            if let Some(secondary) = secondary {
                ui.add(
                    egui::Label::new(RichText::new(secondary).size(12.0).color(pal.text_dim))
                        .truncate()
                        .selectable(false),
                );
            }
        });
    });
    response.on_hover_text(match secondary {
        Some(value) => format!("{name}\n{value}"),
        None => name.to_owned(),
    })
}

fn refresh_workloads(browser: &mut Browser) {
    browser.retry_runtime = None;
    browser.last_poll = f64::NEG_INFINITY;
    browser.manager.last_stats_poll = f64::NEG_INFINITY;
}

fn workload_list(ui: &mut Ui, pal: Palette, browser: &mut Browser) -> bool {
    if browser
        .listing
        .as_ref()
        .is_none_or(|listing| listing.items.is_empty())
    {
        ui.add_space(12.0);
        if browser
            .listing
            .as_ref()
            .is_some_and(|listing| listing.errors.is_empty())
        {
            forms::heading(ui, "No containers or pods", 16.0);
        } else if browser.in_flight || browser.error.is_none() && browser.listing.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading Docker, Podman and Kubernetes containers…");
            });
        }
        runtime_errors(ui, pal, browser);
        if !browser.in_flight
            && ui
                .add(forms::Btn::secondary("Refresh containers").height(44.0))
                .clicked()
        {
            refresh_workloads(browser);
        }
        return false;
    }
    ui.horizontal_wrapped(|ui| {
        const FILTERS: [(StateFilter, &str); 4] = [
            (StateFilter::All, "All states"),
            (StateFilter::Running, "Running"),
            (StateFilter::Paused, "Paused"),
            (StateFilter::Stopped, "Stopped"),
        ];
        forms::Input::bare("Search containers")
            .height(44.0)
            .hint("Search containers, images or projects")
            .icon(crate::icons::MAGNIFYING_GLASS)
            .width((ui.available_width() - 248.0).clamp(150.0, 320.0))
            .show(ui, &mut browser.filter);
        let current = FILTERS
            .iter()
            .find(|(filter, _)| *filter == browser.state_filter)
            .map_or("All states", |(_, text)| *text);
        forms::select(ui, "container-state-filter", 116.0, current, false, |ui| {
            for (filter, text) in FILTERS {
                forms::select_value(ui, &mut browser.state_filter, filter, text);
            }
        })
        .on_hover_text("Filter by state");
        if ui
            .add_enabled(
                !browser.in_flight,
                forms::Btn::secondary("Refresh").height(44.0),
            )
            .on_hover_text("Refresh containers and resource samples")
            .clicked()
        {
            refresh_workloads(browser);
        }
        if browser.in_flight {
            ui.spinner();
        }
    });
    runtime_errors(ui, pal, browser);
    let Some(listing) = &browser.listing else {
        return false;
    };
    if let Some(error) = &browser.manager.stats_error {
        ui.colored_label(pal.warning, format!("Resources unavailable: {error}"));
    }
    let query = browser.filter.trim().to_lowercase();
    let mut groups = BTreeMap::<(_, _, _), Vec<Item>>::new();
    for item in &listing.items {
        if !browser.state_filter.matches(item) {
            continue;
        }
        let standalone = item.project.is_none() && item.runtime != Runtime::Kubernetes;
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
            .entry((item.runtime.label(), standalone, project))
            .or_default()
            .push(item.clone());
    }
    let selected_key = browser.manager.selected.as_deref();
    let mut selection_matches = groups
        .values()
        .flatten()
        .any(|item| selected_key.is_some_and(|key| item.key() == key));
    let count = groups.values().map(Vec::len).sum::<usize>();
    let running = groups
        .values()
        .flatten()
        .filter(|item| item.is_running())
        .count();
    let collapsed = &mut browser.manager.collapsed_groups;
    let mut rows = Vec::new();
    for ((runtime, _, project), mut items) in groups {
        let name = format!("{runtime} / {project}");
        let closed = collapsed.contains(&name);
        items.sort_by(|a, b| a.name.cmp(&b.name));
        rows.push(Row::Group(name, items.len(), closed));
        if !closed {
            rows.extend(items.into_iter().map(Row::Container));
        }
    }
    if rows.is_empty() {
        ui.label(if browser.state_filter == StateFilter::All {
            "No containers match your search."
        } else {
            "No containers match these filters."
        });
        let clear_label = if browser.state_filter == StateFilter::All {
            "Clear search"
        } else {
            "Clear filters"
        };
        if ui
            .add(forms::Btn::ghost(clear_label).height(44.0))
            .clicked()
        {
            browser.filter.clear();
            browser.state_filter = StateFilter::All;
        }
        return false;
    }
    let available = ui.available_rect_before_wrap();
    let footer_height = 28.0;
    let table_height = (available.height() - footer_height).max(0.0);
    let table_rect =
        egui::Rect::from_min_size(available.min, egui::vec2(available.width(), table_height));
    let mut table_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("workload-table-area")
            .max_rect(table_rect),
    );
    table_ui.set_clip_rect(ui.clip_rect().intersect(table_rect));
    let focus_id = ui.scope_id().with("manager-row-focus");
    let focus = ui.data_mut(|data| data.remove_temp::<String>(focus_id));
    let mut selection = None;
    let mut group_change = None;
    let row_height = if ui.available_height() < 430.0 {
        crate::themes::tokens::COMPACT_ROW
    } else {
        crate::themes::tokens::ROW
    };
    table_ui.spacing_mut().item_spacing.y = 0.0;
    let (header, _) =
        table_ui.allocate_exact_size(egui::vec2(table_ui.available_width(), 28.0), Sense::hover());
    let header_cells = workload_columns(header, row_height);
    let font = egui::FontId::new(12.0, crate::fonts::medium());
    for (index, label) in ["Name / image", "State / health", "CPU", "Memory"]
        .into_iter()
        .enumerate()
    {
        let cell = header_cells[index];
        if cell.width() > 0.0 {
            library_list::text(
                &table_ui,
                cell,
                label,
                font.clone(),
                pal.text_dim,
                index >= 2,
            );
        }
    }
    table_ui.painter().hline(
        (header.left() + 12.0)..=(header.right() - 12.0),
        header.bottom(),
        egui::Stroke::new(1.0, pal.border),
    );
    table_ui.add_space(4.0);

    let mut positions = Vec::with_capacity(rows.len() + 1);
    positions.push(0.0);
    for row in &rows {
        let height = if matches!(row, Row::Group(..)) {
            32.0
        } else {
            row_height + 2.0
        };
        positions.push(positions.last().copied().unwrap_or_default() + height);
    }
    let restore_index = focus.as_ref().and_then(|key| {
        rows.iter()
            .position(|row| matches!(row, Row::Container(item) if item.key() == *key))
    });
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("container-table")
        .auto_shrink([false, false]);
    if let Some(index) = restore_index {
        scroll = scroll.vertical_scroll_offset(positions[index]);
    }
    scroll.show_viewport(&mut table_ui, |ui, viewport| {
        let scope = ui.scope_id();
        let start = positions
            .partition_point(|y| *y <= viewport.top())
            .saturating_sub(2);
        let end = positions
            .partition_point(|y| *y < viewport.bottom())
            .saturating_add(1)
            .min(rows.len());
        ui.add_space(positions[start]);
        for (index, row) in rows.iter().enumerate().take(end).skip(start) {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(header.width(), positions[index + 1] - positions[index]),
                Sense::hover(),
            );
            match row {
                Row::Group(name, count, closed) => {
                    let response = library_list::group_header(
                        ui,
                        rect,
                        scope.with(("group", name)),
                        name,
                        *count,
                        *closed,
                        pal,
                    );
                    if response.clicked() {
                        group_change = Some(name.clone());
                    }
                }
                Row::Container(item) => {
                    let rect = egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(0.0, 2.0));
                    let key = item.key();
                    let is_selected = browser.manager.selected.as_ref() == Some(&key);
                    let subtitle = if item.runtime == Runtime::Kubernetes {
                        format!("Ready {}", item.detail)
                    } else {
                        item.detail.clone()
                    };
                    let response = library_list::resource_row(
                        ui,
                        rect,
                        scope.with(&key),
                        crate::icons::PACKAGE,
                        (&item.name, &subtitle),
                        is_selected,
                        pal,
                    );
                    if restore_index == Some(index) {
                        response.request_focus();
                    }
                    if response.clicked() {
                        selection = Some(key.clone());
                    }
                    let [identity, state, cpu, memory] = workload_columns(rect, row_height);
                    library_list::identity(
                        ui,
                        egui::Rect::from_min_max(rect.min, identity.max),
                        &item.name,
                        &subtitle,
                        pal,
                    );
                    workload_state(ui, state, item, pal);
                    let sample = browser
                        .manager
                        .stats
                        .get(&key)
                        .and_then(|history| history.back());
                    let cpu_value = sample
                        .and_then(|s| s.stats.cpu)
                        .map(|value| format!("{value:.1}%"));
                    metric_cell(ui, cpu, cpu_value.as_deref(), pal);
                    if memory.width() > 0.0 {
                        metric_cell(ui, memory, sample.map(|s| s.stats.memory.as_str()), pal);
                    }
                }
            }
        }
        ui.set_min_height(*positions.last().unwrap_or(&0.0));
    });
    let footer_rect = egui::Rect::from_min_max(
        egui::pos2(available.left(), table_rect.bottom()),
        available.max,
    );
    ui.painter().hline(
        footer_rect.x_range(),
        footer_rect.top(),
        egui::Stroke::new(1.0, pal.border),
    );
    let mut footer = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("workload-footer")
            .max_rect(footer_rect.shrink2(egui::vec2(0.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    footer.set_clip_rect(ui.clip_rect().intersect(footer_rect));
    footer.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("{count} workloads · {running} running"))
                .size(12.0)
                .color(pal.text_dim),
        );
        if let Some(updated) = browser.manager.stats_updated {
            let age = updated.elapsed().as_secs();
            ui.label(
                RichText::new(format!(
                    "Resources updated {age}s ago{}",
                    if age > 15 { " · Stale" } else { "" }
                ))
                .size(12.0)
                .color(if age > 15 { pal.warning } else { pal.text_dim }),
            );
        }
    });
    if let Some(name) = group_change {
        if !collapsed.remove(&name) {
            collapsed.insert(name);
        }
    }
    if let Some(key) = selection {
        ui.data_mut(|data| data.insert_temp(ui.scope_id().with("manager-detail-focus"), true));
        browser.manager.select(key);
        selection_matches = true;
    }
    selection_matches
}

fn runtime_errors(ui: &mut Ui, pal: Palette, browser: &mut Browser) {
    let mut retry = None;
    let mut sudo = false;
    if let Some(listing) = &browser.listing {
        for (runtime, error) in &listing.errors {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    pal.warning,
                    format!("{} unavailable · {error}", runtime.label()),
                );
                if *runtime != Runtime::Kubernetes
                    && crate::containers::is_permission_error(error)
                    && !browser.use_sudo
                {
                    sudo |= ui
                        .add_enabled(!browser.in_flight, forms::Btn::ghost("Retry with sudo"))
                        .clicked();
                } else {
                    if ui
                        .add_enabled(
                            !browser.in_flight,
                            forms::Btn::ghost(format!("Retry {}", runtime.label())),
                        )
                        .clicked()
                    {
                        retry = Some(*runtime);
                    }
                }
            });
        }
    }
    if sudo {
        browser.use_sudo = true;
        browser.manager.invalidate_detail();
        refresh_workloads(browser);
    } else if let Some(runtime) = retry {
        browser.retry_runtime = Some(runtime);
        browser.last_poll = f64::NEG_INFINITY;
    }
}

fn workload_columns(rect: egui::Rect, row_height: f32) -> [egui::Rect; 4] {
    let columns = rect.width() >= 420.0;
    let right = rect.right() - 12.0;
    let cpu_right = right - if columns { 132.0 } else { 0.0 };
    let state_right = cpu_right - 70.0;
    let state_left = state_right - if columns { 112.0 } else { 96.0 };
    let identity_left = rect.left() + if row_height < 50.0 { 52.0 } else { 56.0 };
    let cell = |left, right| {
        egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom()),
        )
    };
    [
        cell(identity_left, state_left - 12.0),
        cell(state_left, state_right - 8.0),
        cell(state_right, cpu_right),
        cell(if columns { cpu_right + 12.0 } else { right }, right),
    ]
}

fn workload_state(ui: &Ui, cell: egui::Rect, item: &Item, pal: Palette) {
    let health = health(item, pal);
    let (top, bottom) = library_list::lines(cell);
    let state_cell = if health.is_some() { top } else { cell };
    let color = state_color(item, pal);
    ui.painter()
        .circle_filled(state_cell.left_center() + egui::vec2(3.0, 0.0), 3.0, color);
    let text_rect =
        egui::Rect::from_min_max(state_cell.min + egui::vec2(12.0, 0.0), state_cell.max);
    library_list::text(
        ui,
        text_rect,
        display_state(item),
        egui::FontId::proportional(13.0),
        color,
        false,
    );
    if let Some((label, ink)) = health {
        let text_rect = egui::Rect::from_min_max(bottom.min + egui::vec2(12.0, 0.0), bottom.max);
        library_list::text(
            ui,
            text_rect,
            label,
            egui::FontId::proportional(12.0),
            ink,
            false,
        );
    }
}

fn metric_cell(ui: &Ui, cell: egui::Rect, value: Option<&str>, pal: Palette) {
    library_list::text(
        ui,
        cell,
        value.unwrap_or("—"),
        egui::FontId::monospace(12.0),
        if value.is_some() {
            pal.steel
        } else {
            pal.text_dim
        },
        true,
    );
    if let Some(value) = value {
        ui.interact(
            cell,
            ui.scope_id()
                .with(("metric", cell.min.x.to_bits(), cell.min.y.to_bits())),
            Sense::hover(),
        )
        .on_hover_text(value);
    }
}

enum Row {
    Group(String, usize, bool),
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

fn display_state(item: &Item) -> &str {
    if item.is_running() {
        "Running"
    } else {
        match item.state.as_str() {
            "paused" => "Paused",
            "exited" | "stopped" => "Stopped",
            "created" | "configured" | "initialized" => "Created",
            "restarting" => "Restarting",
            "dead" => "Dead",
            state => state,
        }
    }
}

fn state_color(item: &Item, pal: Palette) -> Color32 {
    if item.is_failing() {
        pal.danger
    } else if item.is_running() {
        pal.success
    } else if item.state == "paused" {
        pal.warning
    } else {
        pal.text_dim
    }
}

fn health(item: &Item, pal: Palette) -> Option<(&'static str, Color32)> {
    if item.status.contains("unhealthy") {
        Some(("Unhealthy", pal.danger))
    } else if item.status.contains("(healthy)") {
        Some(("Healthy", pal.text_dim))
    } else if item.status.contains("health: starting") {
        Some(("Health starting", pal.warning))
    } else {
        None
    }
}

fn confirm_action(
    ui: &Ui,
    pal: Palette,
    browser: &Browser,
    host: &str,
    id: egui::Id,
) -> Option<Command> {
    let (key, action) = ui.data(|data| data.get_temp::<(String, Action)>(id))?;
    let item = browser
        .listing
        .as_ref()
        .and_then(|listing| listing.items.iter().find(|item| item.key() == key));
    let Some(item) = item.filter(|item| item.actions().contains(&action)) else {
        ui.data_mut(|data| data.remove::<(String, Action)>(id));
        return None;
    };
    let modal = egui::Modal::new(id)
        .backdrop_color(pal.scrim)
        .show(ui.ctx(), |ui| {
            ui.set_max_width(400.0);
            forms::heading(ui, &format!("{} {}?", action.label(), item.name), 16.0);
            ui.label(format!(
                "{} on {host} may interrupt active connections.",
                item.name
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let cancel = ui
                    .add(forms::Btn::secondary("Cancel").height(44.0))
                    .clicked();
                let label = format!("{} container", action.label());
                let confirm = ui
                    .add_enabled(
                        browser.can_act(),
                        if action == Action::Stop {
                            forms::Btn::danger(label).height(44.0)
                        } else {
                            forms::Btn::primary(label).height(44.0)
                        },
                    )
                    .clicked();
                (cancel, confirm)
            })
            .inner
        });
    let (cancel, confirm) = modal.inner;
    if cancel || confirm || modal.should_close() {
        ui.data_mut(|data| data.remove::<(String, Action)>(id));
    }
    confirm.then(|| Command::Action(item.clone(), action))
}

fn select_workload(ui: &mut Ui, pal: Palette) {
    ui.add_space(12.0);
    forms::heading(ui, "Select a workload", 16.0);
    ui.label(
        RichText::new("Choose a row to see its logs, resources and configuration.")
            .color(pal.text_dim),
    );
}

fn details(
    ui: &mut Ui,
    pal: Palette,
    browser: &mut Browser,
    confirmation_id: egui::Id,
    compact: bool,
) -> Option<Command> {
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
        select_workload(ui, pal);
        return None;
    };
    let mut command = None;
    let focus_id = ui.scope_id().with("manager-detail-focus");
    let mut back_to_list = false;
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        if compact {
            let back = ui.add(forms::Btn::ghost("All containers").height(44.0));
            if ui
                .data_mut(|data| data.remove_temp::<bool>(focus_id))
                .unwrap_or(false)
            {
                back.request_focus();
            }
            if back.clicked() {
                back_to_list = true;
                return;
            }
        }
        let actions_width = if item.actions().contains(&Action::Stop) {
            168.0
        } else if item.actions().is_empty() {
            0.0
        } else {
            96.0
        };
        let name_width = (ui.available_width() - actions_width).max(96.0);
        ui.allocate_ui_with_layout(
            egui::vec2(name_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_max_width(name_width);
                ui.add(
                    egui::Label::new(
                        RichText::new(&item.name)
                            .family(crate::fonts::semibold())
                            .size(16.0),
                    )
                    .truncate(),
                )
                .on_hover_text(&item.name);
                ui.add(
                    egui::Label::new(
                        RichText::new(&item.detail)
                            .monospace()
                            .size(12.0)
                            .color(pal.text_dim),
                    )
                    .truncate(),
                )
                .on_hover_text(&item.detail);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(display_state(&item)).color(state_color(&item, pal)));
                    if let Some((label, color)) = health(&item, pal) {
                        ui.label(RichText::new(label).size(12.0).color(color));
                    }
                });
            },
        );
        ui.add_enabled_ui(browser.can_act(), |ui| {
            let mut requested = None;
            for &action in item.actions() {
                if matches!(action, Action::Stop | Action::Pause) {
                    continue;
                }
                if ui
                    .add(forms::Btn::ghost(action.label()).height(44.0))
                    .clicked()
                {
                    requested = Some(action);
                }
            }
            if item.actions().contains(&Action::Stop) {
                egui::Popup::menu(&ui.add(forms::Btn::secondary("More").height(44.0))).show(|ui| {
                    forms::menu_padding(ui);
                    for &action in item
                        .actions()
                        .iter()
                        .filter(|action| matches!(action, Action::Stop | Action::Pause))
                    {
                        let button = if action == Action::Stop {
                            forms::Btn::danger(action.label())
                        } else {
                            forms::Btn::ghost(action.label())
                        };
                        if ui.add(button.height(44.0)).clicked() {
                            requested = Some(action);
                            ui.close();
                        }
                    }
                });
            }
            if let Some(action) = requested {
                if matches!(action, Action::Stop | Action::Restart) {
                    ui.data_mut(|data| data.insert_temp(confirmation_id, (item.key(), action)));
                } else {
                    command = Some(Command::Action(item.clone(), action));
                }
            }
        });
    });
    if back_to_list {
        browser.manager.focus_details = false;
        request_row_focus(ui, browser.manager.selected.as_deref());
        return None;
    }
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
    ui.separator();
    let detail = if compact && ui.available_height() < 240.0 {
        egui::ScrollArea::vertical()
            .id_salt((
                "compact-detail-output",
                item.key(),
                browser.manager.tab as u8,
            ))
            .show(ui, |ui| detail_body(ui, pal, browser, item))
            .inner
    } else {
        detail_body(ui, pal, browser, item)
    };
    command.or(detail)
}

fn detail_body(ui: &mut Ui, pal: Palette, browser: &mut Browser, item: Item) -> Option<Command> {
    let mut command = None;
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
                let (text, color) = if browser.manager.pause_logs {
                    ("Paused", pal.warning)
                } else {
                    ("Live · every 2s", pal.success)
                };
                ui.label(RichText::new(text).size(12.0).color(color));
                if let Some(at) = browser.manager.logs_updated {
                    ui.label(format!("Updated {}s ago", at.elapsed().as_secs()));
                }
            });
            forms::Input::bare("Search log lines")
                .height(44.0)
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
                                .max_height((ui.available_height() - 44.0).max(40.0))
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
                    ui.label(
                        RichText::new("Latest 300 lines")
                            .size(12.0)
                            .color(pal.text_dim),
                    );
                }
                Some(Err(error)) => {
                    ui.colored_label(pal.danger, error);
                    if ui
                        .add_enabled(
                            browser.manager.detail_pending.is_none(),
                            forms::Btn::ghost("Retry logs").height(44.0),
                        )
                        .clicked()
                    {
                        browser.manager.pause_logs = false;
                        browser.manager.invalidate_detail();
                    }
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
                        if ui
                            .add(forms::Btn::ghost("Copy JSON").height(44.0))
                            .clicked()
                        {
                            ui.ctx().copy_text(text.clone());
                        }
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
                let age = latest.at.elapsed().as_secs();
                ui.label(
                    RichText::new(format!(
                        "Sampled {age}s ago{}",
                        if age > 15 { " · Stale" } else { "" }
                    ))
                    .color(if age > 15 { pal.warning } else { pal.text_dim }),
                )
                .on_hover_text("Up to 60 samples. CPU can exceed 100% on multiple cores.");
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
    forms::heading(ui, "Published ports", 16.0);
    ui.horizontal_wrapped(|ui| {
        ui.label("Open as");
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
    ui.label(
        RichText::new("Forward through SSH to a free port on this computer.")
            .size(12.0)
            .color(pal.text_dim),
    );
    egui::ScrollArea::vertical().id_salt("container-ports").auto_shrink([false, false]).show(ui, |ui| {
        match &browser.manager.ports {
            None => { ui.spinner(); ui.label("Loading published ports…"); }
            Some(Err(error)) => { ui.colored_label(pal.danger, error); }
            Some(Ok(ports)) if ports.is_empty() => { ui.label("No published ports. Publish a TCP port on the SSH server to open it locally."); }
            Some(Ok(ports)) => {
                for port in ports {
                    let key = port.key(&item.key());
                    ui.push_id(&key, |ui| {
                        super::glass::well_frame(ui.ctx(), 8).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.label(RichText::new("Remote binding → Container port").size(12.0).color(pal.text_dim));
                            ui.label(RichText::new(format!("{} → {}/{}", std::net::SocketAddr::new(port.host, port.host_port), port.container_port, port.protocol)).monospace());
                            let detected = crate::containers::services::detect(&item.detail, port.container_port);
                            ui.label(RichText::new(detected.label()).size(12.0).color(pal.text_dim));
                            let status = browser.manager.tunnels.get(&key).map(|t| t.status());
                            let active = matches!(status, Some(TunnelStatus::Starting | TunnelStatus::Ready(_)));
                            if ui.add_enabled(port.protocol == "tcp" && item.is_running() && !active, forms::Btn::primary("Open locally").height(44.0).painted())
                                .on_disabled_hover_text(if port.protocol != "tcp" { "SSH forwards TCP ports only." } else if active { "Use the active tunnel below." } else { "Start the container first." }).clicked() {
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
                        ui.label(RichText::new("Tunnel ready · Local address").color(pal.success));
                        ui.label(RichText::new(address.to_string()).monospace());
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
        render_with_presets(ctx, browser, size, events, None)
    }

    fn render_with_presets(
        ctx: &egui::Context,
        browser: &mut Browser,
        size: Vec2,
        events: Vec<Event>,
        presets: Option<&[crate::model::LogFilterPreset]>,
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
                    presets,
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
        click_with_presets(ctx, browser, size, label, None)
    }

    fn click_with_presets(
        ctx: &egui::Context,
        browser: &mut Browser,
        size: Vec2,
        label: &str,
        presets: Option<&[crate::model::LogFilterPreset]>,
    ) -> Option<Command> {
        render_with_presets(ctx, browser, size, vec![], presets);
        let (output, _) = render_with_presets(ctx, browser, size, vec![], presets);
        let point = output
            .shapes
            .iter()
            .rev()
            .find_map(|s| text_position(&s.shape, label).filter(|pos| s.clip_rect.contains(*pos)))
            .unwrap_or_else(|| panic!("Missing visible control: {label}"));
        render_with_presets(
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
            presets,
        );
        render_with_presets(
            ctx,
            browser,
            size,
            vec![Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            presets,
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
    fn volume_tab_filters_rows_and_inspects_on_selection() {
        use crate::containers::inventory::{Entry, Reply, Volume};

        let mut browser = fixture();
        let ctx = egui::Context::default();
        let size = Vec2::new(760.0, 380.0);
        assert!(matches!(
            click(&ctx, &mut browser, size, "Volumes"),
            Some(Command::Inventory(Request::List(Kind::Volumes)))
        ));
        let page = browser.manager.inventory.page(Kind::Volumes);
        let volumes = ["app-data", "build-cache"]
            .map(|name| {
                Entry::Volume(Volume {
                    name: name.into(),
                    driver: "local".into(),
                    scope: "local".into(),
                })
            })
            .into();
        page.apply(Request::List(Kind::Volumes), Ok(Reply::Listing(volumes)));

        click(&ctx, &mut browser, size, "Search volumes");
        let (output, _) = render(&ctx, &mut browser, size, vec![Event::Text("cache".into())]);
        assert_eq!(browser.manager.inventory.page(Kind::Volumes).filter, "cache");
        let text = rendered_text(&output);
        assert!(text.contains("build-cache"));
        assert!(!text.lines().any(|line| line == "app-data"));
        let command = click(&ctx, &mut browser, size, "build-cache");
        let Some(Command::Inventory(Request::Inspect(Kind::Volumes, volume))) = command else {
            panic!("Expected volume inspection");
        };
        assert_eq!(volume, "build-cache");
        click(&ctx, &mut browser, size, "All volumes");
        let page = browser.manager.inventory.page(Kind::Volumes);
        assert_eq!(page.filter, "cache");
        assert_eq!(page.selected.as_deref(), Some("build-cache"));
        assert!(!page.focus_details);
    }

    #[test]
    fn volume_file_browser_opens_folders_and_exports_the_whole_volume() {
        use crate::containers::{
            inventory::{Entry, Reply, Volume},
            volume_files::{Entry as File, Kind as FileKind},
        };

        let mut browser = fixture();
        browser.manager.section = Section::Inventory(Kind::Volumes);
        let page = browser.manager.inventory.page(Kind::Volumes);
        page.apply(
            Request::List(Kind::Volumes),
            Ok(Reply::Listing(vec![Entry::Volume(Volume {
                name: "app-data".into(),
                driver: "local".into(),
                scope: "local".into(),
            })])),
        );
        let ctx = egui::Context::default();
        let size = Vec2::new(850.0, 440.0);
        click(&ctx, &mut browser, size, "app-data");
        click(&ctx, &mut browser, size, "Browse files");
        let Some(Command::BrowseVolume { name, directory, revision }) =
            render(&ctx, &mut browser, size, vec![]).1
        else {
            panic!("Expected the volume's root folder");
        };
        assert_eq!(name, "app-data");
        assert!(directory.is_empty());
        browser.manager.inventory.page(Kind::Volumes).files.apply(
            revision,
            Ok(vec![File {
                name: b"nested".to_vec(),
                kind: FileKind::Directory,
            }]),
        );
        let Some(Command::BrowseVolume { directory, .. }) =
            click(&ctx, &mut browser, size, "nested")
        else {
            panic!("Expected a request for the selected folder");
        };
        assert_eq!(directory, [b"nested".to_vec()]);
        assert!(matches!(
            click(&ctx, &mut browser, size, "Export volume"),
            Some(Command::ExportVolume(name)) if name == "app-data"
        ));
    }

    #[test]
    fn volume_text_editor_opens_and_keeps_the_draft_on_save_conflict() {
        use crate::containers::{
            inventory::{Entry, Reply, Volume},
            volume_files::{Edit, EditResult, Entry as File, Kind as FileKind, version},
        };

        let mut browser = fixture();
        browser.manager.section = Section::Inventory(Kind::Volumes);
        let page = browser.manager.inventory.page(Kind::Volumes);
        page.apply(
            Request::List(Kind::Volumes),
            Ok(Reply::Listing(vec![Entry::Volume(Volume {
                name: "app-data".into(),
                driver: "local".into(),
                scope: "local".into(),
            })])),
        );
        page.select("app-data");
        page.files.open = true;
        page.files.apply(page.files.revision, Ok(vec![File {
            name: b"settings.ini".to_vec(),
            kind: FileKind::File(8),
        }]));
        let ctx = egui::Context::default();
        let size = Vec2::new(850.0, 440.0);
        let Some(Command::EditVolume(request)) = click(&ctx, &mut browser, size, "Edit text") else {
            panic!("Expected a volume text editor request");
        };
        assert!(matches!(request.edit, Edit::Open));
        assert_eq!(request.name, b"settings.ini");
        let text = "port=80\n";
        let original = version("20:123:8:time:time:1", text).unwrap();
        let files = &mut browser.manager.inventory.page(Kind::Volumes).files;
        files.apply_edit(request.revision, Ok(EditResult::Opened {
            version: original.clone(),
            text: text.into(),
        }));
        let editor = files.editor.as_mut().unwrap();
        editor.text = "port=8080\n".into();
        editor.dirty = true;
        let Some(Command::EditVolume(request)) = click(&ctx, &mut browser, size, "Save file") else {
            panic!("Expected a save request");
        };
        let Edit::Save { original: expected, text } = request.edit else {
            panic!("Expected the draft to be uploaded");
        };
        assert_eq!(expected, original);
        assert_eq!(text, "port=8080\n");
        let files = &mut browser.manager.inventory.page(Kind::Volumes).files;
        files.apply_edit(request.revision, Err("The file changed on the host.".into()));
        let (output, _) = render(&ctx, &mut browser, size, vec![]);
        let rendered = rendered_text(&output);
        assert!(rendered.contains("The file changed on the host."));
        assert!(rendered.contains("port=8080"));
        assert!(rendered.contains("Discard edits"));
    }

    #[test]
    fn volume_import_uses_the_new_volume_name_from_the_form() {
        use crate::containers::inventory::Reply;

        let mut browser = fixture();
        browser.manager.section = Section::Inventory(Kind::Volumes);
        browser.manager.inventory.page(Kind::Volumes).apply(
            Request::List(Kind::Volumes),
            Ok(Reply::Listing(Vec::new())),
        );
        let ctx = egui::Context::default();
        let size = Vec2::new(850.0, 440.0);
        assert!(click(&ctx, &mut browser, size, "Import archive").is_none());
        click(&ctx, &mut browser, size, "e.g. app-data");
        render(
            &ctx,
            &mut browser,
            size,
            vec![Event::Text("restored-data".into())],
        );
        assert!(matches!(
            click(&ctx, &mut browser, size, "Choose archive"),
            Some(Command::ImportVolume(name)) if name == "restored-data"
        ));
    }

    #[test]
    fn builds_tab_opens_the_selected_record_and_filters_its_log() {
        use crate::containers::inventory::{Detail, Entry, Reply};

        let mut browser = fixture();
        let ctx = egui::Context::default();
        let size = Vec2::new(1200.0, 800.0);
        assert!(matches!(
            click(&ctx, &mut browser, size, "Builds"),
            Some(Command::Inventory(Request::List(Kind::Builds)))
        ));

        let build_ref = "desktop-linux/desktop-linux/build123";
        let build = serde_json::from_value(serde_json::json!({
            "ref": build_ref,
            "name": "web image",
            "status": "Error",
            "created_at": "2026-11-18T17:56:33Z",
            "completed_steps": 2,
            "total_steps": 3,
        }))
        .unwrap();
        browser.manager.inventory.page(Kind::Builds).apply(
            Request::List(Kind::Builds),
            Ok(Reply::Listing(vec![Entry::Build(build)])),
        );
        let Some(Command::Inventory(Request::Inspect(Kind::Builds, selected))) =
            click(&ctx, &mut browser, size, "web image")
        else {
            panic!("Expected the selected build's log request");
        };
        assert_eq!(selected, build_ref);

        browser.manager.inventory.page(Kind::Builds).apply(
            Request::Inspect(Kind::Builds, selected),
            Ok(Reply::Detail(Detail::BuildLog(
                "loaded build context\nCOPY missing-file failed\n".into(),
            ))),
        );
        click(&ctx, &mut browser, size, "Search build log");
        let (output, _) = render(
            &ctx,
            &mut browser,
            size,
            vec![Event::Text("MISSING-FILE".into())],
        );
        let text = rendered_text(&output);
        assert!(text.contains("COPY missing-file failed"));
        assert!(!text.contains("loaded build context"));
        assert!(text.contains("2 of 3 steps"));
    }

    #[test]
    fn unified_logs_loads_only_selected_sources_and_searches_messages() {
        use crate::containers::{inventory::Reply, unified_logs::{Row, Snapshot}};

        let mut browser = fixture();
        browser.manager.inventory.page(Kind::Builds).apply(
            Request::List(Kind::Builds),
            Ok(Reply::Listing(Vec::new())),
        );
        let ctx = egui::Context::default();
        let size = Vec2::new(1200.0, 800.0);
        click(&ctx, &mut browser, size, "Unified logs");
        click(&ctx, &mut browser, size, "Docker / shop-api");
        let Some(Command::UnifiedLogs { revision, sources }) =
            click(&ctx, &mut browser, size, "Refresh logs")
        else {
            panic!("Expected logs for the selected workload");
        };
        assert_eq!(sources.iter().map(|source| source.key()).collect::<Vec<_>>(), ["Docker/abcdef123456/shop-api"]);
        browser.manager.unified_logs.apply(revision, Snapshot {
            rows: ["ready for requests", "disk quota exceeded"]
                .map(|message| Row {
                    time: None,
                    source: "Docker / shop-api".into(),
                    message: message.into(),
                })
                .into(),
            errors: Vec::new(),
        });
        click(&ctx, &mut browser, size, "Search logs");
        let (output, _) = render(&ctx, &mut browser, size, vec![Event::Text("QUOTA".into())]);
        let text = rendered_text(&output);
        assert!(text.contains("disk quota exceeded"));
        assert!(!text.contains("ready for requests"));

        click(&ctx, &mut browser, size, "Search options");
        click(&ctx, &mut browser, size, "Regular expression");
        assert!(browser.manager.unified_logs.search.regex);
        click(&ctx, &mut browser, size, "Search options");
        click(&ctx, &mut browser, size, "Export logs");
        let Some(Command::ExportLogs(exported)) =
            click(&ctx, &mut browser, size, "Export matches")
        else {
            panic!("Expected an export for matching rows");
        };
        assert_eq!(
            exported
                .iter()
                .map(|row| row.message.as_str())
                .collect::<Vec<_>>(),
            ["disk quota exceeded"]
        );

        assert!(matches!(
            click(&ctx, &mut browser, size, "Auto-refresh"),
            Some(Command::UnifiedLogs { .. })
        ));
        let now = ctx.input(|input| input.time);
        browser.manager.unified_logs.last_request = Some(now);
        assert!(render(&ctx, &mut browser, size, vec![]).1.is_none());
        let mut next = None;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(now + 5.1),
                ..Default::default()
            },
            |ui| {
                next = show(
                    ui,
                    crate::themes::current(&ctx),
                    &mut browser,
                    Source::Ssh("test-host"),
                    None,
                );
            },
        );
        assert!(matches!(next, Some(Command::UnifiedLogs { .. })));
    }

    #[test]
    fn selecting_a_saved_log_filter_reloads_its_sources() {
        use crate::containers::inventory::Reply;
        use crate::model::{LogFilterPreset, LogSearch};

        let mut browser = fixture();
        browser.manager.section = Section::Logs;
        browser.manager.inventory.page(Kind::Builds).apply(
            Request::List(Kind::Builds),
            Ok(Reply::Listing(Vec::new())),
        );
        let preset = LogFilterPreset {
            name: "Workload errors".into(),
            search: LogSearch {
                query: "^ERROR".into(),
                regex: true,
                case_sensitive: true,
            },
            sources: ["Docker/abcdef123456/shop-api".to_owned()].into(),
        };
        let presets = Some(std::slice::from_ref(&preset));
        let ctx = egui::Context::default();
        let size = Vec2::new(1200.0, 800.0);
        click_with_presets(&ctx, &mut browser, size, "Saved filters", presets);
        let Some(Command::UnifiedLogs { sources, .. }) =
            click_with_presets(&ctx, &mut browser, size, "Workload errors", presets)
        else {
            panic!("Expected the preset to request its saved sources");
        };
        assert_eq!(
            sources.iter().map(|source| source.key()).collect::<Vec<_>>(),
            ["Docker/abcdef123456/shop-api"],
        );
        assert_eq!(browser.manager.unified_logs.search, preset.search);
    }

    #[test]
    fn compact_log_source_picker_opens_without_permanently_using_table_space() {
        let mut browser = fixture();
        browser.manager.section = Section::Logs;
        let ctx = egui::Context::default();
        let size = Vec2::new(760.0, 380.0);
        let (output, _) = render(&ctx, &mut browser, size, vec![]);
        assert!(!rendered_text(&output).contains("Find log sources"));

        click(&ctx, &mut browser, size, "Sources (0 selected)");
        click(&ctx, &mut browser, size, "Docker / shop-api");
        assert!(
            browser
                .manager
                .unified_logs
                .selected
                .contains("Docker/abcdef123456/shop-api")
        );
    }

    #[test]
    fn library_group_header_collapses_only_its_workloads() {
        let mut browser = fixture();
        let ctx = egui::Context::default();
        let size = Vec2::new(1400.0, 900.0);
        click(&ctx, &mut browser, size, "Docker / shop");
        let (closed, _) = render(&ctx, &mut browser, size, vec![]);
        let text = rendered_text(&closed);
        assert!(!text.lines().any(|line| line == "shop-api"));
        assert!(text.lines().any(|line| line == "batch-worker"));
        click(&ctx, &mut browser, size, "Docker / shop");
        let (opened, _) = render(&ctx, &mut browser, size, vec![]);
        assert!(rendered_text(&opened).lines().any(|line| line == "shop-api"));
    }

    #[test]
    fn paused_filter_keeps_only_paused_rows_and_offers_resume() {
        let mut browser = fixture();
        browser.listing.as_mut().unwrap().items.extend(
            crate::containers::parse(
                r#"@docker
{"ID":"123456abcdef","Names":"paused-worker","Image":"alpine:3.21","State":"paused","Status":"Up 2 hours (Paused)"}
"#,
            )
            .items,
        );
        let ctx = egui::Context::default();
        let size = Vec2::new(1400.0, 900.0);
        click(&ctx, &mut browser, size, "All states");
        click(&ctx, &mut browser, size, "Paused");
        let (output, _) = render(&ctx, &mut browser, size, vec![]);
        let text = rendered_text(&output);
        assert!(text.contains("paused-worker"));
        assert!(!text.contains("shop-api"));
        assert!(!text.contains("batch-worker"));

        click(&ctx, &mut browser, size, "paused-worker");
        let command = click(&ctx, &mut browser, size, "Resume");
        let Some(Command::Action(item, Action::Resume)) = command else {
            panic!("Expected a resume command");
        };
        assert_eq!(item.scope, "123456abcdef");
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
    fn stop_requires_confirmation_for_the_named_workload() {
        let mut browser = fixture();
        let ctx = egui::Context::default();
        let size = Vec2::new(1200.0, 800.0);
        click(&ctx, &mut browser, size, "shop-api");
        click(&ctx, &mut browser, size, "More");
        assert!(click(&ctx, &mut browser, size, "Stop").is_none());
        render(&ctx, &mut browser, size, vec![]);
        let (output, _) = render(&ctx, &mut browser, size, vec![]);
        let text = rendered_text(&output);
        assert!(text.contains("Stop shop-api?"), "{text}");
        assert!(click(&ctx, &mut browser, size, "Cancel").is_none());
        click(&ctx, &mut browser, size, "More");
        assert!(click(&ctx, &mut browser, size, "Stop").is_none());
        let Some(Command::Action(item, Action::Stop)) =
            click(&ctx, &mut browser, size, "Stop container")
        else {
            panic!("Expected a confirmed stop command");
        };
        assert_eq!(item.scope, "abcdef123456");
    }

    #[test]
    fn compact_ports_scroll_to_the_forwarding_action() {
        let mut browser = fixture();
        browser.manager.select("Docker/abcdef123456/shop-api".into());
        browser.manager.choose_tab(DetailTab::Ports);
        browser.manager.ports = Some(crate::containers::ports::parse(
            r#"{"80/tcp":[{"HostIp":"127.0.0.1","HostPort":"8080"}]}"#,
        ).map_err(|error| error.to_string()));
        let ctx = egui::Context::default();
        let size = Vec2::new(760.0, 320.0);
        let (before, _) = render(&ctx, &mut browser, size, vec![]);
        let visible = |output: &egui::FullOutput, label| {
            output.shapes.iter().any(|shape| {
                text_position(&shape.shape, label).is_some_and(|pos| shape.clip_rect.contains(pos))
            })
        };
        assert!(!visible(&before, "Open locally"));
        render(&ctx, &mut browser, size, vec![
            Event::PointerMoved(Pos2::new(128.0, 280.0)),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Page,
                delta: Vec2::new(0.0, -1.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (after, _) = render(&ctx, &mut browser, size, vec![]);
        assert!(visible(&after, "All containers"));
        let Some(Command::OpenPort(item, port, _)) =
            click(&ctx, &mut browser, size, "Open locally")
        else {
            panic!("Expected to reach the forward action after scrolling");
        };
        assert_eq!(item.name, "shop-api");
        assert_eq!(port.host_port, 8080);
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
