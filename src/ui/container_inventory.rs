//! Docker images, volumes and build history inside the SSH container manager.
use egui::{RichText, Sense};
use egui_extras::{Column, TableBuilder};

use super::forms;
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
) -> Option<Action> {
    if kind == Kind::Volumes
        && page.files.open
        && let Some(name) = page.selected.as_deref()
    {
        return super::container_volume_files::show(ui, pal, name, &mut page.files);
    }
    let (search, loading, empty, mutation) = match kind {
        Kind::Images => (
            "Search images",
            "Loading images…",
            "No images on this host.",
            Some(("Image reference", "e.g. redis:7", "Pull image")),
        ),
        Kind::Volumes => (
            "Search volumes",
            "Loading volumes…",
            "No volumes on this host.",
            Some(("Volume name", "e.g. app-data", "Create volume")),
        ),
        Kind::Builds => (
            "Search builds",
            "Loading builds…",
            "No builds on the configured builder.",
            None,
        ),
    };
    if ui.available_height() < 425.0 && page.selected.is_some() {
        let back = match kind {
            Kind::Images => "All images",
            Kind::Volumes => "All volumes",
            Kind::Builds => "All builds",
        };
        if ui.add(forms::Btn::secondary(back).small()).clicked() {
            page.clear_selection();
            return None;
        }
        return details(ui, pal, kind, page).map(Action::Inventory);
    }
    let mut request = None;
    let mut import = None;
    ui.horizontal_wrapped(|ui| {
        forms::Input::bare(search)
            .hint(search)
            .icon(crate::icons::MAGNIFYING_GLASS)
            .width(240.0)
            .show(ui, &mut page.filter);
        if ui
            .add_enabled(!page.pending, forms::Btn::secondary("Refresh").small())
            .clicked()
        {
            request = Some(Request::List(kind));
        }
        if page.pending {
            ui.spinner();
        }
    });
    if let Some((input, hint, save)) = mutation {
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(!page.pending, |ui| {
                forms::Input::bare(input)
                    .hint(hint)
                    .width(240.0)
                    .show(ui, &mut page.input);
                if ui
                    .add_enabled(
                        !page.input.trim().is_empty(),
                        forms::Btn::secondary(save).small(),
                    )
                    .clicked()
                {
                    let value = page.input.trim().to_owned();
                    request = Some(if kind == Kind::Images {
                        Request::Pull(value)
                    } else {
                        Request::CreateVolume(value)
                    });
                }
                if kind == Kind::Volumes
                    && ui
                        .add_enabled(
                            !page.input.trim().is_empty(),
                            forms::Btn::secondary("Import archive").small(),
                        )
                        .on_hover_text("Restore an archive into a new volume with this name")
                        .clicked()
                {
                    import = Some(Action::ImportVolume(page.input.trim().to_owned()));
                }
            });
        });
    }
    if import.is_some() {
        return import;
    }
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
                let height = ui.available_height();
                let max_height = if page.selected.is_some() {
                    height * 0.45
                } else {
                    height - 32.0
                };
                let mut selected = None;
                ui.visuals_mut().selection.bg_fill = super::glass::selection_fill(ui);
                ui.visuals_mut().widgets.hovered.weak_bg_fill = super::glass::hover_wash(ui);
                let (secondary, trailing) = if kind == Kind::Builds {
                    (Column::exact(100.0), Column::exact(146.0))
                } else {
                    (
                        Column::remainder().at_least(70.0),
                        Column::auto().at_least(60.0),
                    )
                };
                TableBuilder::new(ui)
                    .id_salt(kind)
                    .sense(Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .max_scroll_height(max_height.max(64.0))
                    .column(Column::remainder().at_least(90.0).clip(true))
                    .column(secondary.clip(true))
                    .column(trailing.clip(true))
                    .header(28.0, |mut header| {
                        let columns = match kind {
                            Kind::Images => ["Image", "ID", "Size"],
                            Kind::Volumes => ["Volume", "Driver", "Scope"],
                            Kind::Builds => ["Build", "Status", "Started (UTC)"],
                        };
                        for label in columns {
                            header.col(|ui| {
                                ui.label(RichText::new(label).color(pal.text_dim));
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(28.0, rows.len(), |mut row| {
                            let entry = rows[row.index()];
                            row.set_selected(page.selected.as_deref() == Some(entry.id()));
                            let (name, secondary, trailing) = match entry {
                                Entry::Image(image) => (
                                    image.reference().unwrap_or_else(|| "<untagged>".into()),
                                    image.id.clone(),
                                    image.size.clone(),
                                ),
                                Entry::Volume(volume) => (
                                    volume.name.clone(),
                                    volume.driver.clone(),
                                    volume.scope.clone(),
                                ),
                                Entry::Build(build) => (
                                    build_name(build).to_owned(),
                                    build.status.clone(),
                                    build.created_at.format("%Y-%m-%d %H:%M").to_string(),
                                ),
                            };
                            for (column, label) in [name.as_str(), &secondary, &trailing]
                                .into_iter()
                                .enumerate()
                            {
                                row.col(|ui| {
                                    let color = if column == 1
                                        && kind == Kind::Builds
                                        && matches!(label, "Error" | "Failed")
                                    {
                                        pal.danger
                                    } else {
                                        pal.text
                                    };
                                    ui.add(
                                        egui::Label::new(RichText::new(label).color(color))
                                            .truncate()
                                            .selectable(false),
                                    )
                                    .on_hover_text(label);
                                });
                            }
                            if row.response().clicked() {
                                selected = Some(entry.id().to_owned());
                            }
                        })
                    });
                if let Some(id) = selected {
                    page.select(&id);
                }
            }
        }
    }
    if page.selected.is_some() {
        ui.separator();
        let detail_request = details(ui, pal, kind, page);
        request = request.or(detail_request);
    }
    request.map(Action::Inventory)
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
    let id = page.selected.as_deref()?;
    let mut request = None;
    ui.horizontal_wrapped(|ui| {
        forms::heading(
            ui,
            if kind == Kind::Builds {
                "Build log"
            } else {
                "Metadata"
            },
            16.0,
        );
        let copy = match kind {
            Kind::Images => "Copy image ID",
            Kind::Volumes => "Copy volume name",
            Kind::Builds => "Copy build reference",
        };
        if ui.add(forms::Btn::secondary(copy).small()).clicked() {
            ui.ctx().copy_text(id.to_owned());
        }
        if kind == Kind::Volumes && ui.add(forms::Btn::secondary("Browse files").small()).clicked() {
            page.files.open = true;
        }
    });
    if kind == Kind::Builds
        && let Some(Ok(entries)) = &page.listing
        && let Some(Entry::Build(build)) = entries.iter().find(|entry| entry.id() == id)
    {
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
