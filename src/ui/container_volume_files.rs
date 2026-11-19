//! File browser and text editor for a Docker volume on the current SSH host.
use egui::{RichText, Sense};
use egui_extras::{Column, TableBuilder};

use super::{container_inventory::Action, forms};
use crate::{
    containers::volume_files::{EDIT_LIMIT, Edit, EditRequest, Kind, View, display_name},
    themes::Palette,
};

pub fn show(ui: &mut egui::Ui, pal: Palette, volume: &str, view: &mut View) -> Option<Action> {
    if view.editor.is_some() {
        return editor(ui, pal, volume, view);
    }
    let mut export = false;
    let mut parent_depth = None;
    let mut reload = false;
    ui.horizontal_wrapped(|ui| {
        forms::heading(ui, volume, 16.0);
        if ui
            .add(forms::Btn::ghost("Volume details").height(44.0))
            .clicked()
        {
            view.close();
        }
        export = ui
            .add(forms::Btn::secondary("Export volume").height(44.0))
            .on_hover_text("Save the entire volume as a tar archive")
            .clicked();
    });
    if !view.open {
        return None;
    }
    ui.horizontal_wrapped(|ui| {
        if view.directory.is_empty() {
            ui.label(RichText::new("/").monospace().color(pal.text_dim));
        } else {
            if ui.add(forms::Btn::ghost("Root").height(44.0)).clicked() {
                parent_depth = Some(0);
            }
            for (depth, component) in view.directory.iter().enumerate() {
                ui.label(RichText::new("/").color(pal.text_dim));
                let name = display_name(component);
                if depth + 1 == view.directory.len() {
                    ui.label(RichText::new(name).monospace());
                } else if ui.add(forms::Btn::ghost(name).height(44.0)).clicked() {
                    parent_depth = Some(depth + 1);
                }
            }
        }
        reload = ui
            .add_enabled(
                !view.pending,
                forms::Btn::secondary("Refresh files").height(44.0),
            )
            .clicked();
    });
    if let Some(depth) = parent_depth {
        view.navigate(view.directory[..depth].to_vec());
    } else if reload {
        view.navigate(view.directory.clone());
    }
    let mut folder = None;
    let mut open = None;
    match &view.result {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading folder…");
            });
        }
        Some(Err(error)) => {
            ui.colored_label(pal.warning, error);
        }
        Some(Ok(entries)) if entries.is_empty() => {
            ui.label("This folder is empty.");
        }
        Some(Ok(entries)) => {
            let max_height = (ui.available_height() - 32.0).max(48.0);
            ui.visuals_mut().selection.bg_fill = super::glass::selection_fill(ui);
            ui.visuals_mut().widgets.hovered.weak_bg_fill = super::glass::hover_wash(ui);
            TableBuilder::new(ui)
                .id_salt(("volume-file-list", volume, &view.directory))
                .sense(Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .max_scroll_height(max_height)
                .column(Column::remainder().at_least(100.0).clip(true))
                .column(Column::exact(100.0))
                .column(Column::exact(100.0))
                .column(Column::exact(90.0))
                .header(28.0, |mut header| {
                    for label in ["Name", "Type", "Size"] {
                        header.col(|ui| {
                            ui.label(RichText::new(label).color(pal.text_dim));
                        });
                    }
                    header.col(|_| {});
                })
                .body(|body| {
                    body.rows(44.0, entries.len(), |mut row| {
                        let entry = &entries[row.index()];
                        row.col(|ui| {
                            let name = display_name(&entry.name);
                            if entry.kind == Kind::Directory {
                                ui.push_id(&entry.name, |ui| {
                                    if super::container_manager::row_label(
                                        ui, &name, None, false, pal,
                                    )
                                    .clicked()
                                    {
                                        folder = Some(entry.name.clone());
                                    }
                                });
                            } else {
                                ui.add(egui::Label::new(&name).selectable(false).truncate())
                                    .on_hover_text(name);
                            }
                        });
                        let (kind, size) = match entry.kind {
                            Kind::Directory => ("Folder", None),
                            Kind::File(size) => ("File", Some(size)),
                            Kind::Link => ("Symbolic link", None),
                            Kind::Other => ("Other", None),
                        };
                        row.col(|ui| {
                            ui.label(kind);
                        });
                        row.col(|ui| {
                            if let Some(size) = size {
                                ui.label(format!("{size} B"));
                            }
                        });
                        row.col(|ui| {
                            if let Kind::File(size) = entry.kind
                                && size <= EDIT_LIMIT
                                && ui
                                    .add(forms::Btn::secondary("Edit text").height(44.0))
                                    .clicked()
                            {
                                open = Some(entry.name.clone());
                            }
                        });
                        if entry.kind == Kind::Directory && row.response().clicked() {
                            folder = Some(entry.name.clone());
                        }
                    })
                });
        }
    }
    if let Some(folder) = folder {
        let mut directory = view.directory.clone();
        directory.push(folder);
        view.navigate(directory);
    }
    if let Some(name) = open {
        view.open_editor(name);
        return editor(ui, pal, volume, view);
    }
    if export {
        Some(Action::ExportVolume(volume.to_owned()))
    } else if view.result.is_none() && !view.pending {
        Some(Action::BrowseVolume {
            name: volume.to_owned(),
            directory: view.directory.clone(),
            revision: view.revision,
        })
    } else {
        None
    }
}

fn editor(ui: &mut egui::Ui, pal: Palette, volume: &str, view: &mut View) -> Option<Action> {
    let editor = view.editor.as_mut()?;
    let mut close = false;
    let mut save = false;
    let path = view
        .directory
        .iter()
        .chain(std::iter::once(&editor.name))
        .map(|component| display_name(component))
        .collect::<Vec<_>>()
        .join("/");
    let read_only = editor
        .original
        .as_ref()
        .is_some_and(|version| version.read_only());
    ui.horizontal_wrapped(|ui| {
        forms::heading(ui, volume, 16.0);
        ui.label(RichText::new(format!("/{path}")).monospace());
        let back = if editor.dirty {
            "Discard edits"
        } else {
            "Back to files"
        };
        close = ui
            .add_enabled(!editor.pending, forms::Btn::secondary(back).small())
            .clicked();
        save = ui
            .add_enabled(
                !editor.pending && !read_only && editor.original.is_some() && editor.dirty,
                forms::Btn::primary("Save file").height(44.0),
            )
            .clicked();
        if editor.pending {
            ui.spinner();
        }
    });
    if close {
        view.close_editor();
        return None;
    }
    if let Some(error) = &editor.error {
        ui.colored_label(pal.warning, error);
        if editor.original.is_none()
            && ui
                .add_enabled(!editor.pending, forms::Btn::secondary("Retry").small())
                .clicked()
        {
            editor.error = None;
        }
    } else if editor.saved {
        ui.label("Saved on the host.");
    }
    if editor.original.is_some() {
        if read_only {
            ui.label("Read-only · This file has multiple hard links.");
        } else if editor.dirty {
            ui.label("Unsaved changes");
        }
        egui::ScrollArea::vertical()
            .id_salt(("volume-text-editor", view.revision))
            .max_height(ui.available_height().max(40.0))
            .show(ui, |ui| {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut editor.text)
                        .id_salt(("volume-file-text", view.revision))
                        .hint_text("File contents")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(14)
                        .interactive(!editor.pending && !read_only),
                );
                if response.changed() {
                    editor.dirty = true;
                    editor.saved = false;
                }
            });
    } else if editor.error.is_none() {
        ui.label("Opening file…");
    }
    let edit = if save {
        Edit::Save {
            original: editor.original.clone()?,
            text: editor.text.clone(),
        }
    } else if !editor.pending && editor.original.is_none() && editor.error.is_none() {
        Edit::Open
    } else {
        return None;
    };
    Some(Action::EditVolume(EditRequest {
        volume: volume.to_owned(),
        directory: view.directory.clone(),
        name: editor.name.clone(),
        revision: view.revision,
        edit,
    }))
}
