//! Combined log view for a selected set of workloads and Buildx records.
use std::fmt::Write as _;

use egui::{RichText, Sense};
use egui_extras::{Column, TableBuilder};

use super::forms;
use crate::{
    containers::{
        Listing,
        inventory::{Entry, Page},
        unified_logs::{MAX_SOURCES, PresetEdit, Row, Source, View},
    },
    model::LogFilterPreset,
    themes::Palette,
};

pub enum Action {
    LoadBuilds,
    Load { revision: u64, sources: Vec<Source> },
    EditPreset(PresetEdit),
    Export(Vec<Row>),
}

pub fn show(
    ui: &mut egui::Ui,
    pal: Palette,
    state: &mut View,
    containers: Option<&Listing>,
    builds: &Page,
    presets: Option<&[LogFilterPreset]>,
) -> Option<Action> {
    let mut sources: Vec<_> = containers
        .into_iter()
        .flat_map(|listing| &listing.items)
        .cloned()
        .map(Source::Container)
        .collect();
    let mut action = None;
    match &builds.listing {
        Some(Ok(entries)) => sources.extend(entries.iter().filter_map(|entry| {
            if let Entry::Build(build) = entry {
                Some(Source::Build {
                    reference: build.reference.clone(),
                    name: build.name.clone(),
                })
            } else {
                None
            }
        })),
        None if !builds.pending => action = Some(Action::LoadBuilds),
        _ => {}
    }
    let mut sources: Vec<_> = sources
        .into_iter()
        .map(|source| (source.key(), source.label(), source))
        .collect();
    sources.sort_by_key(|(key, _, _)| !state.selected.contains(key));
    let compact = ui.available_height() < 400.0;
    let source_label = format!("Sources ({} selected)", state.selected.len());
    let mut pick_sources = |ui: &mut egui::Ui| {
        ui.horizontal_wrapped(|ui| {
            forms::Input::bare("Find log sources")
                .hint("Find log sources")
                .width(215.0)
                .show(ui, &mut state.source_query);
            if ui
                .add_enabled(
                    !builds.pending,
                    forms::Btn::secondary("Refresh sources").small(),
                )
                .clicked()
            {
                action = Some(Action::LoadBuilds);
            }
            if !state.selected.is_empty() && ui.add(forms::Btn::ghost("Clear sources")).clicked() {
                state.clear_sources();
            }
            if state.selected.len() == MAX_SOURCES {
                ui.label(format!("{} sources maximum", MAX_SOURCES));
            }
        });
        if let Some(Err(error)) = &builds.listing {
            ui.colored_label(pal.warning, format!("Build sources unavailable. {error}"));
        }
        let query = state.source_query.trim().to_lowercase();
        let visible: Vec<_> = sources
            .iter()
            .filter(|(key, label, _)| {
                query.is_empty()
                    || label.to_lowercase().contains(&query)
                    || key.to_lowercase().contains(&query)
            })
            .collect();
        if visible.is_empty() {
            ui.label(if builds.pending || containers.is_none() {
                "Looking for sources…"
            } else {
                "No matching sources."
            });
        }
        egui::ScrollArea::vertical()
            .id_salt("unified-log-source-list")
            .max_height((ui.available_height() * 0.32).clamp(45.0, 145.0))
            .show_rows(ui, 20.0, visible.len(), |ui, range| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for (key, label, _) in &visible[range] {
                    let mut selected = state.selected.contains(key);
                    if ui
                        .add_enabled(
                            selected || state.selected.len() < MAX_SOURCES,
                            egui::Checkbox::new(&mut selected, label),
                        )
                        .on_hover_text(label)
                        .changed()
                    {
                        state.change_source(key.clone(), selected);
                    }
                }
            });
    };
    if compact {
        let config = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        egui::containers::menu::MenuButton::new(source_label)
            .config(config)
            .ui(ui, |ui| {
                forms::menu_padding(ui);
                ui.set_min_width(300.0);
                pick_sources(ui);
            });
    } else {
        egui::CollapsingHeader::new(source_label)
            .id_salt("unified-log-sources")
            .default_open(true)
            .show(ui, pick_sources);
    }

    ui.horizontal_wrapped(|ui| {
        forms::Input::bare("Search logs")
            .hint("Search logs")
            .icon(crate::icons::MAGNIFYING_GLASS)
            .width(215.0)
            .show(ui, &mut state.search.query);
        ui.menu_button("Search options", |ui| {
            forms::menu_padding(ui);
            ui.checkbox(&mut state.search.regex, "Regular expression");
            ui.checkbox(&mut state.search.case_sensitive, "Match case");
        });
        state.update_search();
        if let Some(edit) = preset_menu(ui, state, presets) {
            action = Some(Action::EditPreset(edit));
        }
    });
    if let Some(error) = state.matcher().error() {
        ui.colored_label(pal.warning, format!("Invalid search pattern. {error}"));
    }
    let available_count = sources
        .iter()
        .filter(|(key, _, _)| state.selected.contains(key))
        .count();
    if available_count != state.selected.len() {
        ui.label("Some selected sources are unavailable.");
    }
    let rows: Vec<_> = state
        .snapshot
        .iter()
        .flat_map(|snapshot| snapshot.rows.iter().enumerate())
        .filter(|(_, row)| state.matcher().matches(row))
        .collect();
    let mut refresh = ui
        .horizontal_wrapped(|ui| {
            let refresh = ui
                .add_enabled(
                    available_count > 0 && !state.pending,
                    forms::Btn::secondary("Refresh logs").small(),
                )
                .clicked();
            ui.checkbox(&mut state.auto_refresh, "Auto-refresh");
            if ui
                .add_enabled(
                    !rows.is_empty(),
                    forms::Btn::secondary("Copy matches").small(),
                )
                .clicked()
            {
                ui.ctx().copy_text(copy_rows(&rows));
            }
            ui.menu_button("Export logs", |ui| {
                forms::menu_padding(ui);
                let all = state
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.rows.as_slice())
                    .unwrap_or_default();
                if ui
                    .add_enabled(!all.is_empty(), egui::Button::new("Export all rows"))
                    .clicked()
                {
                    action = Some(Action::Export(all.to_vec()));
                    ui.close();
                }
                if ui
                    .add_enabled(!rows.is_empty(), egui::Button::new("Export matches"))
                    .clicked()
                {
                    action = Some(Action::Export(
                        rows.iter().map(|(_, row)| (*row).clone()).collect(),
                    ));
                    ui.close();
                }
            });
            if state.pending {
                ui.spinner();
            }
            refresh
        })
        .inner;
    if state.refresh_requested && available_count > 0 && !state.pending && action.is_none() {
        refresh = true;
    }
    if state.auto_refresh && available_count > 0 && !state.pending {
        let now = ui.input(|input| input.time);
        let wait = state.last_request.map_or(0.0, |last| 5.0 - (now - last));
        if wait > 0.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(wait));
        } else if action.is_none() {
            refresh = true;
        }
    }
    if refresh {
        let selected = sources
            .iter()
            .filter(|(key, _, _)| state.selected.contains(key))
            .map(|(_, _, source)| source.clone())
            .collect();
        action = Some(Action::Load {
            revision: state.revision,
            sources: selected,
        });
    }
    let Some(snapshot) = &state.snapshot else {
        ui.label(if state.pending {
            "Reading log sources…"
        } else {
            "Select sources and refresh to read their logs."
        });
        return action;
    };
    for (source, error) in &snapshot.errors {
        ui.colored_label(pal.warning, format!("{source} · {error}"));
    }
    if state.matcher().error().is_some() {
        return action;
    }
    if rows.is_empty() {
        ui.label(if snapshot.rows.is_empty() {
            "No log lines returned."
        } else {
            "No log lines match your search."
        });
        return action;
    }
    show_rows(ui, pal, &rows, &mut state.selected_row);
    action
}

fn preset_menu(
    ui: &mut egui::Ui,
    state: &mut View,
    presets: Option<&[LogFilterPreset]>,
) -> Option<PresetEdit> {
    ui.menu_button("Saved filters", |ui| {
        forms::menu_padding(ui);
        let Some(presets) = presets else {
            ui.label("Save this SSH host to keep log filters.");
            return None;
        };
        if presets.is_empty() {
            ui.label("No saved filters for this host.");
        }
        egui::ScrollArea::vertical()
            .max_height(145.0)
            .show(ui, |ui| {
                for preset in presets {
                    let mut edit = None;
                    ui.push_id(&preset.name, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button(&preset.name).clicked() {
                                state.apply_preset(preset);
                                ui.close();
                            }
                            if ui
                                .small_button("Remove")
                                .on_hover_text(format!("Remove filter {}", preset.name))
                                .clicked()
                            {
                                edit = Some(PresetEdit::Remove(preset.name.clone()));
                                ui.close();
                            }
                        });
                    });
                    if edit.is_some() {
                        return edit;
                    }
                }
                None
            })
            .inner
            .or_else(|| {
                forms::Input::bare("Filter name")
                    .hint("Filter name")
                    .width(200.0)
                    .show(ui, &mut state.preset_name);
                let name = state.preset_name.trim();
                let label = if presets.iter().any(|preset| preset.name == name) {
                    "Update filter"
                } else {
                    "Save filter"
                };
                if ui
                    .add_enabled(
                        !name.is_empty() && state.matcher().error().is_none(),
                        forms::Btn::secondary(label).small(),
                    )
                    .clicked()
                {
                    ui.close();
                    return Some(PresetEdit::Save(LogFilterPreset {
                        name: name.to_owned(),
                        search: state.search.clone(),
                        sources: state.selected.clone(),
                    }));
                }
                None
            })
    })
    .inner
    .flatten()
}

fn copy_rows(rows: &[(usize, &Row)]) -> String {
    let mut text = String::new();
    for (_, row) in rows {
        let time = row.time.map(|time| time.to_rfc3339()).unwrap_or_default();
        let _ = writeln!(text, "{time}\t{}\t{}", row.source, row.message);
    }
    text
}

fn show_rows(
    ui: &mut egui::Ui,
    pal: Palette,
    rows: &[(usize, &Row)],
    selected: &mut Option<usize>,
) {
    let detail_height = if selected.is_some() { 85.0 } else { 0.0 };
    let height = (ui.available_height() - detail_height - 32.0).max(42.0);
    ui.visuals_mut().selection.bg_fill = super::glass::selection_fill(ui);
    ui.visuals_mut().widgets.hovered.weak_bg_fill = super::glass::hover_wash(ui);
    TableBuilder::new(ui)
        .id_salt("unified-log-table")
        .sense(Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .max_scroll_height(height)
        .column(Column::exact(150.0))
        .column(Column::initial(205.0).at_least(105.0).clip(true))
        .column(Column::remainder().at_least(90.0).clip(true))
        .header(28.0, |mut header| {
            for label in ["Time (UTC)", "Source", "Message"] {
                header.col(|ui| {
                    ui.label(RichText::new(label).color(pal.text_dim));
                });
            }
        })
        .body(|body| {
            body.rows(24.0, rows.len(), |mut row| {
                let (index, entry) = rows[row.index()];
                row.set_selected(*selected == Some(index));
                let time = entry
                    .time
                    .map(|time| time.format("%m-%d %H:%M:%S%.3f").to_string());
                for value in [
                    time.as_deref().unwrap_or("—"),
                    &entry.source,
                    &entry.message,
                ] {
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(RichText::new(value).monospace().color(pal.text))
                                .selectable(false)
                                .truncate(),
                        )
                        .on_hover_text(value);
                    });
                }
                if row.response().clicked() {
                    *selected = (*selected != Some(index)).then_some(index);
                }
            });
        });
    if let Some(index) = *selected
        && let Some((_, row)) = rows.iter().find(|(row_index, _)| *row_index == index)
    {
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("unified-log-expanded")
            .max_height(ui.available_height().max(28.0))
            .show(ui, |ui| {
                let mut text = row.message.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
            });
    }
}
