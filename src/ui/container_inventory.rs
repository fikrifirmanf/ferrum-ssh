//! Docker images, volumes and build history inside the SSH container manager.
use egui::{RichText, Sense};

use super::{container_manager, forms, library_list};
use crate::{
    containers::{
        inventory::{Build, Detail, Entry, Kind, Page, Request},
        is_permission_error,
    },
    themes::Palette,
};

pub enum Action {
    Inventory(Request),
    BrowseVolume {
        name: String,
        directory: Vec<Vec<u8>>,
        revision: u64,
    },
    ExportVolume(String),
    EditVolume(crate::containers::volume_files::EditRequest),
    ImportVolume(String),
}

pub fn show(
    ui: &mut egui::Ui,
    pal: Palette,
    kind: Kind,
    page: &mut Page,
    sudo: &mut bool,
    host: &str,
) -> Option<Action> {
    if kind == Kind::Volumes
        && page.files.open
        && let Some(name) = page.selected.as_deref()
    {
        return super::container_volume_files::show(ui, pal, name, &mut page.files);
    }
    let compact = container_manager::compact_workspace(ui);
    let focus_id = ui.scope_id().with(("inventory-focus", kind));
    if compact && page.focus_details && page.selected.is_some() {
        let back = match kind {
            Kind::Images => "All images",
            Kind::Volumes => "All volumes",
            Kind::Builds => "All builds",
        };
        let button = ui.add(forms::Btn::ghost(back).height(44.0));
        if ui
            .data_mut(|data| data.remove_temp::<bool>(focus_id.with("details")))
            .unwrap_or(false)
        {
            button.request_focus();
        }
        if button.clicked() {
            page.focus_details = false;
            ui.data_mut(|data| {
                data.insert_temp(focus_id, page.selected.clone().unwrap_or_default())
            });
            return None;
        }
        return details(ui, pal, kind, page).map(Action::Inventory);
    }
    if compact || !matches!(&page.listing, Some(Ok(entries)) if !entries.is_empty()) {
        let result = list(ui, pal, kind, page, sudo, host, focus_id);
        if result.is_none()
            && page.focus_details
            && page.selected.is_some()
            && page.detail.is_none()
            && !page.pending
        {
            return page
                .selected
                .clone()
                .map(|id| Action::Inventory(Request::Inspect(kind, id)));
        }
        result
    } else {
        let (mut list_ui, mut inspector) = container_manager::split_workspace(ui, pal, 0.38, 340.0);
        let action = list(&mut list_ui, pal, kind, page, sudo, host, focus_id);
        let detail = details(&mut inspector, pal, kind, page).map(Action::Inventory);
        action.or(detail)
    }
}

#[derive(Clone, Copy)]
enum Mutation {
    Pull,
    Create,
    Import,
}

impl Mutation {
    fn label(self) -> &'static str {
        match self {
            Self::Pull => "Pull image",
            Self::Create => "Create volume",
            Self::Import => "Import archive",
        }
    }
}

fn mutation_form(
    ui: &mut egui::Ui,
    pal: Palette,
    page: &mut Page,
    host: &str,
    id: egui::Id,
) -> Option<Action> {
    let intent = ui.data(|data| data.get_temp::<Mutation>(id))?;
    let modal = egui::Modal::new(id)
        .backdrop_color(pal.scrim)
        .show(ui.ctx(), |ui| {
            ui.set_width(360.0);
            forms::heading(ui, intent.label(), 16.0);
            ui.label(RichText::new(host).monospace().color(pal.text_dim));
            let (name, hint) = match intent {
                Mutation::Pull => ("Image reference", "e.g. redis:7"),
                _ => ("Volume name", "e.g. app-data"),
            };
            if matches!(intent, Mutation::Import) {
                ui.label("Restore a tar archive into a new volume.");
            }
            forms::Input::new(name).hint(hint).show(ui, &mut page.input);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let submit_label = if matches!(intent, Mutation::Import) {
                    "Choose archive"
                } else {
                    intent.label()
                };
                let cancel = ui.add(forms::Btn::ghost("Cancel").height(44.0)).clicked();
                let submit = ui
                    .add_enabled(
                        !page.pending && !page.input.trim().is_empty(),
                        forms::Btn::primary(submit_label).height(44.0),
                    )
                    .clicked();
                (cancel, submit)
            })
            .inner
        });
    let (cancel, submit) = modal.inner;
    if cancel || submit || modal.should_close() {
        ui.data_mut(|data| data.remove::<Mutation>(id));
    }
    submit.then(|| {
        let value = page.input.trim().to_owned();
        match intent {
            Mutation::Pull => Action::Inventory(Request::Pull(value)),
            Mutation::Create => Action::Inventory(Request::CreateVolume(value)),
            Mutation::Import => Action::ImportVolume(value),
        }
    })
}

fn list(
    ui: &mut egui::Ui,
    pal: Palette,
    kind: Kind,
    page: &mut Page,
    sudo: &mut bool,
    host: &str,
    focus_id: egui::Id,
) -> Option<Action> {
    let (search, loading, empty, mutation) = match kind {
        Kind::Images => (
            "Search images",
            "Loading images…",
            "No images on this host.",
            Some(Mutation::Pull),
        ),
        Kind::Volumes => (
            "Search volumes",
            "Loading volumes…",
            "No volumes on this host.",
            Some(Mutation::Create),
        ),
        Kind::Builds => (
            "Search builds",
            "Loading builds…",
            "No builds on the configured builder.",
            None,
        ),
    };
    let modal_id = focus_id.with("mutation");
    if let Some(action) = mutation_form(ui, pal, page, host, modal_id) {
        return Some(action);
    }
    let mut request = None;
    let has_entries = matches!(&page.listing, Some(Ok(entries)) if !entries.is_empty());
    ui.horizontal_wrapped(|ui| {
        if has_entries {
            forms::Input::bare(search)
                .height(44.0)
                .hint(search)
                .icon(crate::icons::MAGNIFYING_GLASS)
                .width((ui.available_width() * 0.45).clamp(140.0, 240.0))
                .show(ui, &mut page.filter);
        }
        if mutation.is_none() || !matches!(&page.listing, Some(Ok(entries)) if entries.is_empty()) {
            if ui
                .add_enabled(!page.pending, forms::Btn::secondary("Refresh").height(44.0))
                .clicked()
            {
                request = Some(Request::List(kind));
            }
        }
        if let Some(mutation) = mutation {
            if ui
                .add_enabled(
                    !page.pending,
                    forms::Btn::primary(mutation.label()).height(44.0),
                )
                .clicked()
            {
                ui.data_mut(|data| data.insert_temp(modal_id, mutation));
            }
            if kind == Kind::Volumes
                && ui
                    .add_enabled(
                        !page.pending,
                        forms::Btn::secondary("Import archive").height(44.0),
                    )
                    .clicked()
            {
                ui.data_mut(|data| data.insert_temp(modal_id, Mutation::Import));
            }
        }
        if page.pending {
            ui.spinner();
        }
    });
    if let Some(notice) = &page.notice {
        match notice {
            Ok(message) => {
                ui.label(message);
            }
            Err(error) => {
                ui.colored_label(pal.danger, error);
            }
        }
    }
    match &page.listing {
        None => {
            ui.label(loading);
            if !page.pending && request.is_none() {
                request = Some(Request::List(kind));
            }
        }
        Some(Err(error)) => {
            ui.colored_label(pal.warning, error);
            if !*sudo
                && is_permission_error(error)
                && ui
                    .add_enabled(
                        !page.pending,
                        forms::Btn::secondary("Retry with sudo").small(),
                    )
                    .clicked()
            {
                *sudo = true;
                request = Some(Request::List(kind));
            }
        }
        Some(Ok(entries)) if entries.is_empty() => {
            ui.label(empty);
        }
        Some(Ok(entries)) => {
            let query = page.filter.trim().to_ascii_lowercase();
            let rows: Vec<_> = entries
                .iter()
                .filter(|entry| {
                    let matches = |value: &str| value.to_ascii_lowercase().contains(&query);
                    match entry {
                        Entry::Image(image) => {
                            matches(&image.id)
                                || matches(
                                    &image.reference().unwrap_or_else(|| "<untagged>".into()),
                                )
                        }
                        Entry::Volume(volume) => matches(&volume.name) || matches(&volume.driver),
                        Entry::Build(build) => {
                            matches(build_name(build))
                                || matches(&build.status)
                                || matches(&build.reference)
                        }
                    }
                })
                .collect();
            if rows.is_empty() {
                ui.label("No resources match your search.");
                if ui.add(forms::Btn::ghost("Clear search")).clicked() {
                    page.filter.clear();
                }
            } else {
                let restore = ui.data_mut(|data| data.remove_temp::<String>(focus_id));
                let mut selected = None;
                let row_height = if ui.available_height() < 430.0 {
                    crate::themes::tokens::COMPACT_ROW
                } else {
                    crate::themes::tokens::ROW
                };
                let labels = match kind {
                    Kind::Images => ["Image / ID", "", "Size"],
                    Kind::Volumes => ["Volume / driver", "", "Scope"],
                    Kind::Builds => ["Build / reference", "Status", "Started (UTC)"],
                };
                let (header, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
                let [identity, state, trailing] = inventory_columns(header, kind);
                let identity = egui::Rect::from_min_max(
                    identity.min + egui::vec2(if row_height < 50.0 { 52.0 } else { 56.0 }, 0.0),
                    identity.max,
                );
                for (index, (cell, label)) in [identity, state, trailing]
                    .into_iter()
                    .zip(labels)
                    .enumerate()
                {
                    if !label.is_empty() {
                        library_list::text(
                            ui,
                            cell,
                            label,
                            egui::FontId::new(12.0, crate::fonts::medium()),
                            pal.text_dim,
                            index == 2,
                        );
                    }
                }
                ui.painter().hline(
                    (header.left() + 12.0)..=(header.right() - 12.0),
                    header.bottom(),
                    egui::Stroke::new(1.0, pal.border),
                );
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt(("inventory", kind))
                    .auto_shrink([false, false]);
                if let Some(index) = restore
                    .as_deref()
                    .and_then(|id| rows.iter().position(|entry| entry.id() == id))
                {
                    scroll = scroll.vertical_scroll_offset(
                        index as f32 * (row_height + ui.spacing().item_spacing.y),
                    );
                }
                scroll.show_rows(ui, row_height, rows.len(), |ui, range| {
                    let scope = ui.scope_id();
                    for entry in &rows[range] {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(header.width(), row_height),
                            Sense::hover(),
                        );
                        let icon = match kind {
                            Kind::Images => crate::icons::PACKAGE,
                            Kind::Volumes => crate::icons::HARD_DRIVES,
                            Kind::Builds => crate::icons::TERMINAL_WINDOW,
                        };
                        let (name, subtitle, status, value) = match entry {
                            Entry::Image(image) => (
                                image.reference().unwrap_or_else(|| "<untagged>".into()),
                                image.id.as_str(),
                                "",
                                image.size.clone(),
                            ),
                            Entry::Volume(volume) => (
                                volume.name.clone(),
                                volume.driver.as_str(),
                                "",
                                volume.scope.clone(),
                            ),
                            Entry::Build(build) => (
                                build_name(build).to_owned(),
                                build.reference.as_str(),
                                build.status.as_str(),
                                build.created_at.format("%Y-%m-%d %H:%M").to_string(),
                            ),
                        };
                        let response = library_list::resource_row(
                            ui,
                            rect,
                            scope.with(entry.id()),
                            icon,
                            (&name, subtitle),
                            page.selected.as_deref() == Some(entry.id()),
                            pal,
                        );
                        if restore.as_deref() == Some(entry.id()) {
                            response.request_focus();
                        }
                        if response.clicked() {
                            selected = Some(entry.id().to_owned());
                        }
                        let [identity, state, trailing] = inventory_columns(rect, kind);
                        library_list::identity(ui, identity, &name, subtitle, pal);
                        if !status.is_empty() {
                            let color = match status {
                                "Error" | "Failed" => pal.danger,
                                "Completed" | "Succeeded" => pal.success,
                                _ => pal.text_dim,
                            };
                            library_list::text(
                                ui,
                                state,
                                status,
                                egui::FontId::proportional(13.0),
                                color,
                                false,
                            );
                        }
                        library_list::text(
                            ui,
                            trailing,
                            &value,
                            egui::FontId::monospace(12.0),
                            pal.text_dim,
                            true,
                        );
                    }
                });
                if let Some(id) = selected {
                    page.select(&id);
                    ui.data_mut(|data| data.insert_temp(focus_id.with("details"), true));
                }
            }
        }
    }
    request.map(Action::Inventory)
}

fn inventory_columns(rect: egui::Rect, kind: Kind) -> [egui::Rect; 3] {
    let right = rect.right() - 12.0;
    let end = if kind == Kind::Builds {
        right - 136.0
    } else {
        right - 92.0
    };
    let state = if kind == Kind::Builds {
        end - 92.0
    } else {
        end
    };
    let cell = |left, right| {
        egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(right, rect.bottom()),
        )
    };
    [
        cell(rect.left(), state - 12.0),
        cell(state, end - 8.0),
        cell(end, right),
    ]
}

fn build_name(build: &Build) -> &str {
    if build.name.trim().is_empty() {
        build
            .reference
            .rsplit('/')
            .next()
            .unwrap_or(&build.reference)
    } else {
        &build.name
    }
}

fn details(ui: &mut egui::Ui, pal: Palette, kind: Kind, page: &mut Page) -> Option<Request> {
    ui.add_space(8.0);
    let Some(id) = page.selected.as_deref() else {
        let resource = match kind {
            Kind::Images => "an image",
            Kind::Volumes => "a volume",
            Kind::Builds => "a build",
        };
        forms::heading(ui, &format!("Select {resource}"), 16.0);
        ui.label(RichText::new("Choose a row to see its details.").color(pal.text_dim));
        return None;
    };
    let entry = match &page.listing {
        Some(Ok(entries)) => entries.iter().find(|entry| entry.id() == id),
        _ => None,
    };
    let name = match entry {
        Some(Entry::Image(image)) => image.reference().unwrap_or_else(|| "<untagged>".into()),
        Some(Entry::Build(build)) => build_name(build).to_owned(),
        _ => id.to_owned(),
    };
    forms::heading(ui, &name, 16.0);
    let mut request = None;
    ui.horizontal_wrapped(|ui| {
        let copy = match kind {
            Kind::Images => "Copy image ID",
            Kind::Volumes => "Copy volume name",
            Kind::Builds => "Copy build reference",
        };
        if ui.add(forms::Btn::secondary(copy).small()).clicked() {
            ui.ctx().copy_text(id.to_owned());
        }
        if kind == Kind::Volumes
            && ui
                .add(forms::Btn::primary("Browse files").height(44.0))
                .clicked()
        {
            page.files.open = true;
        }
    });
    if let Some(Entry::Build(build)) = entry {
        forms::segmented_tabs(
            ui,
            "build-detail",
            &mut page.build_metadata,
            &[(false, "Build log"), (true, "Inspect")],
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(build_name(build));
            ui.label(&build.status);
            if let (Some(completed), Some(total)) = (build.completed_steps, build.total_steps) {
                ui.label(format!("{completed} of {total} steps"));
            }
            if let Some(cached) = build.cached_steps {
                ui.label(format!("{cached} cached"));
            }
            if let Some(finished) = build.completed_at {
                let elapsed = (finished - build.created_at).num_milliseconds();
                if elapsed >= 0 {
                    ui.label(format!("{:.2}s", elapsed as f64 / 1000.0));
                }
            }
        });
    }
    if kind == Kind::Builds && page.build_metadata {
        ui.label(RichText::new(id).monospace().size(12.0).color(pal.text_dim))
            .on_hover_text(id);
        return None;
    }
    match &page.detail {
        Some(Ok(Detail::Metadata(value))) => {
            let metadata = serde_json::to_string_pretty(value).unwrap_or_default();
            output(ui, kind, &metadata);
        }
        Some(Ok(Detail::BuildLog(log))) if log.trim().is_empty() => {
            ui.label("This build has no log output.");
        }
        Some(Ok(Detail::BuildLog(log))) => {
            ui.horizontal_wrapped(|ui| {
                forms::Input::bare("Search build log")
                    .height(44.0)
                    .hint("Search build log")
                    .width(240.0)
                    .show(ui, &mut page.log_filter);
                if ui
                    .add(forms::Btn::secondary("Copy all logs").small())
                    .clicked()
                {
                    ui.ctx().copy_text(log.clone());
                }
            });
            let query = page.log_filter.trim().to_ascii_lowercase();
            let lines: Vec<_> = log
                .lines()
                .filter(|line| query.is_empty() || line.to_ascii_lowercase().contains(&query))
                .collect();
            if lines.is_empty() {
                ui.label("No log lines match your search.");
            } else {
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                egui::ScrollArea::both()
                    .id_salt(("build-log", id))
                    .max_height(ui.available_height().max(row_height))
                    .show_rows(ui, row_height, lines.len(), |ui, range| {
                        for line in &lines[range] {
                            ui.add(egui::Label::new(RichText::new(*line).monospace()).extend());
                        }
                    });
            }
        }
        Some(Err(error)) => {
            ui.colored_label(pal.warning, error);
            if ui
                .add_enabled(
                    !page.pending,
                    forms::Btn::secondary(if kind == Kind::Builds {
                        "Retry log"
                    } else {
                        "Retry inspection"
                    })
                    .small(),
                )
                .clicked()
            {
                request = Some(Request::Inspect(kind, id.to_owned()));
            }
        }
        None if page.listing.as_ref().is_some_and(Result::is_ok) => {
            ui.label(if kind == Kind::Builds {
                "Loading build log…"
            } else {
                "Loading metadata…"
            });
            if !page.pending {
                request = Some(Request::Inspect(kind, id.to_owned()));
            }
        }
        None => {}
    }
    request
}

fn output(ui: &mut egui::Ui, kind: Kind, mut text: &str) {
    egui::ScrollArea::vertical()
        .id_salt(("inventory-detail", kind))
        .max_height(ui.available_height().max(24.0))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
        });
}
