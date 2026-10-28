//! Dual-pane SFTP browser (local ↔ remote, remote ↔ remote) and the transfer queue drawer.

use std::sync::atomic::Ordering;

use egui::{Align2, FontId, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use uuid::Uuid;

use super::{danger_button, ghost_button, primary_button};
use crate::app::{App, FsPane, SortKey, Tab};
use crate::conn::AppEvent;
use crate::model::Protocol;
use crate::sftp::{Backend, FileEntry, SftpEvent, TransferStatus, human_size, perms_string};
use crate::themes::Palette;

/// Replay the exact original locations, even if a pane has since navigated elsewhere.
#[derive(Clone)]
pub struct TransferRetry {
    pub(crate) source: Backend,
    pub(crate) entry: FileEntry,
    pub(crate) destination: Backend,
    pub(crate) directory: String,
    pub(crate) pane: u64,
}

enum FsAction {
    Navigate(u64, String),
    Back(u64),
    Forward(u64),
    Connect(u64, Uuid),
    UseLocal(u64),
    Disconnect(u64),
    Copy {
        from: u64,
        entries: Vec<FileEntry>,
        to: u64,
    },
    Mkdir(u64, String),
    Touch(u64, String),
    Rename(u64, String, String),
    Chmod(u64, String, u32),
    Delete(u64, Vec<FileEntry>),
    Edit(u64, String),
    OpenLocal(String),
}

const ROW_H: f32 = 36.0;

impl App {
    pub fn sftp_tab(&mut self, ui: &mut egui::Ui, idx: usize) {
        let pal = self.pal();
        let show_hidden = self.vault.settings.sftp_show_hidden;
        let hosts: Vec<(Uuid, String, String)> = self
            .vault
            .hosts
            .iter()
            .filter(|h| h.protocol == Protocol::Ssh)
            .map(|h| (h.id, h.display_name(), h.address.clone()))
            .collect();
        let mut actions: Vec<FsAction> = vec![];
        let mut toggle_hidden = false;

        let Some(Tab::Sftp(tab)) = self.tabs.get_mut(idx) else {
            return;
        };
        let (left_id, right_id) = (tab.left.id, tab.right.id);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.bg)
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(ui, |ui| {
                ui.label(
                    RichText::new("File transfer")
                        .size(crate::themes::tokens::TITLE)
                        .strong(),
                );
                ui.label(
                    RichText::new("Select files, then choose their destination.")
                        .small()
                        .color(pal.text_dim),
                );
                ui.add_space(16.0);
                let narrow = ui.ctx().content_rect().width() < 900.0;
                let key = egui::Id::unique(("sftp-visible-pane", tab.id));
                let mut right_visible = ui.data(|d| d.get_temp::<bool>(key).unwrap_or(false));
                if narrow {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(!right_visible, "Left location")
                            .clicked()
                        {
                            right_visible = false;
                        }
                        if ui
                            .selectable_label(right_visible, "Right location")
                            .clicked()
                        {
                            right_visible = true;
                        }
                    });
                    ui.add_space(8.0);
                    ui.data_mut(|d| d.insert_temp(key, right_visible));
                }
                let full = ui.available_rect_before_wrap();
                let width = if narrow {
                    full.width()
                } else {
                    full.width() / 2.0
                };
                let left_rect = Rect::from_min_size(full.min, Vec2::new(width, full.height()));
                let right_rect = if narrow {
                    left_rect
                } else {
                    Rect::from_min_size(
                        Pos2::new(full.left() + width, full.top()),
                        Vec2::new(width, full.height()),
                    )
                };
                let left_selected = selected_entries(&tab.left);
                let right_selected = selected_entries(&tab.right);
                let left_backend = tab.left.backend.clone();
                let right_backend = tab.right.backend.clone();
                if !narrow || !right_visible {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                        fs_pane(
                            ui,
                            &mut tab.left,
                            right_id,
                            &right_selected,
                            right_backend.as_ref(),
                            &hosts,
                            show_hidden,
                            pal,
                            &mut actions,
                            &mut toggle_hidden,
                        )
                    });
                }
                if !narrow || right_visible {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
                        fs_pane(
                            ui,
                            &mut tab.right,
                            left_id,
                            &left_selected,
                            left_backend.as_ref(),
                            &hosts,
                            show_hidden,
                            pal,
                            &mut actions,
                            &mut toggle_hidden,
                        )
                    });
                }
            });

        if toggle_hidden {
            self.vault.settings.sftp_show_hidden = !show_hidden;
            self.mark_dirty();
        }
        for a in actions {
            self.run_fs_action(a);
        }
    }

    fn run_fs_action(&mut self, a: FsAction) {
        match a {
            FsAction::Navigate(pane, path) => {
                if let Some(p) = self.fs_pane_mut(pane) {
                    p.forward.clear();
                }
                self.fs_navigate(pane, path);
            }
            FsAction::Back(pane) => {
                let target = self.fs_pane_mut(pane).and_then(|p| {
                    let t = p.back.pop()?;
                    p.forward.push(std::mem::take(&mut p.path));
                    Some(t)
                });
                if let Some(t) = target {
                    self.fs_navigate(pane, t);
                }
            }
            FsAction::Forward(pane) => {
                let target = self.fs_pane_mut(pane).and_then(|p| p.forward.pop());
                if let Some(t) = target {
                    self.fs_navigate(pane, t);
                }
            }
            FsAction::Connect(pane, host) => self.fs_connect(pane, host),
            FsAction::UseLocal(pane) => {
                if let Some(p) = self.fs_pane_mut(pane) {
                    *p = FsPane {
                        id: pane,
                        ..FsPane::new()
                    };
                    p.backend = Some(Backend::Local);
                }
                let home = directories::UserDirs::new()
                    .map(|u| u.home_dir().to_string_lossy().to_string())
                    .unwrap_or("/".into());
                self.fs_navigate(pane, home);
            }
            FsAction::Disconnect(pane) => {
                let old = self.fs_pane_mut(pane).map(|p| {
                    let b = p.backend.take();
                    *p = FsPane {
                        id: pane,
                        host_picker: true,
                        ..FsPane::new()
                    };
                    b
                });
                if let Some(Some(Backend::Remote { client, .. })) = old {
                    self.rt.spawn(async move { client.disconnect().await });
                }
            }
            FsAction::Copy { from, entries, to } => {
                let src = self.fs_pane_mut(from).and_then(|p| p.backend.clone());
                let dst = self
                    .fs_pane_mut(to)
                    .and_then(|p| Some((p.backend.clone()?, p.path.clone())));
                let (Some(src), Some((dst, dir))) = (src, dst) else {
                    self.toast("Open a location on the other side first", true);
                    return;
                };
                for entry in entries {
                    self.queue_file_transfer(TransferRetry {
                        source: src.clone(),
                        entry,
                        destination: dst.clone(),
                        directory: dir.clone(),
                        pane: to,
                    });
                }
                self.show_transfers = true;
            }
            FsAction::Mkdir(pane, path) => self.fs_op(pane, "Created folder", move |b| {
                Box::pin(async move { crate::sftp::mkdir(&b, &path).await })
            }),
            FsAction::Touch(pane, path) => self.fs_op(pane, "Created file", move |b| {
                Box::pin(async move { crate::sftp::touch(&b, &path).await })
            }),
            FsAction::Rename(pane, from, to) => self.fs_op(pane, "Renamed", move |b| {
                Box::pin(async move { crate::sftp::rename(&b, &from, &to).await })
            }),
            FsAction::Chmod(pane, path, mode) => {
                self.fs_op(pane, "Permissions updated", move |b| {
                    Box::pin(async move { crate::sftp::chmod(&b, &path, mode).await })
                })
            }
            FsAction::Delete(pane, entries) => self.fs_op(pane, "Deleted", move |b| {
                Box::pin(async move {
                    for e in &entries {
                        crate::sftp::remove(&b, &e.path, e.is_dir && !e.is_link).await?;
                    }
                    Ok(())
                })
            }),
            FsAction::Edit(pane, remote) => {
                let Some(b) = self.fs_pane_mut(pane).and_then(|p| p.backend.clone()) else {
                    return;
                };
                let bridge = self.bridge.clone();
                self.rt.spawn(async move {
                    match crate::sftp::download_for_edit(&b, &remote).await {
                        Ok(local) => bridge.send(AppEvent::Sftp(SftpEvent::EditReady {
                            local,
                            remote,
                            pane,
                        })),
                        Err(e) => bridge.toast(format!("Could not open {remote}: {e:#}"), true),
                    }
                });
            }
            FsAction::OpenLocal(path) => {
                if let Err(e) = open::that_detached(&path) {
                    self.toast(format!("Could not open {path}: {e}"), true);
                }
            }
        }
    }

    /// Run one backend operation and refresh the pane when it finishes.
    fn fs_op<F>(&mut self, pane: u64, ok_msg: &'static str, op: F)
    where
        F: FnOnce(
                Backend,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>,
            > + Send
            + 'static,
    {
        let Some(b) = self.fs_pane_mut(pane).and_then(|p| p.backend.clone()) else {
            return;
        };
        let bridge = self.bridge.clone();
        self.rt.spawn(async move {
            let result = op(b)
                .await
                .map(|_| ok_msg.to_string())
                .map_err(|e| format!("{e:#}"));
            bridge.send(AppEvent::Sftp(SftpEvent::OpDone { pane, result }));
        });
    }

    pub(crate) fn queue_file_transfer(&mut self, request: TransferRetry) {
        let bridge = self.bridge.clone();
        let name = request.entry.name.clone();
        let pane = request.pane;
        let id = crate::sftp::start_transfer(
            self.rt.handle(),
            &self.transfers,
            request.source.clone(),
            request.entry.clone(),
            request.destination.clone(),
            request.directory.clone(),
            self.bridge.ctx.clone(),
            move |result| {
                bridge.send(AppEvent::Sftp(SftpEvent::OpDone {
                    pane,
                    result: result.map(|_| format!("Copied {name}")),
                }));
            },
        );
        self.transfer_retries.insert(id, request);
        self.show_transfers = true;
    }

    pub(crate) fn retry_file_transfer(&mut self, id: u64) {
        if let Some(request) = self.transfer_retries.remove(&id) {
            self.queue_file_transfer(request);
        }
    }

    pub fn transfers_drawer(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let list: Vec<_> = self.transfers.lock().iter().rev().cloned().collect();
        let mut clear = false;
        let mut hide = false;
        let mut retry = None;
        // Successful jobs no longer need to retain their source/destination handles.
        let retryable: std::collections::HashSet<_> = list
            .iter()
            .filter(|t| *t.status.lock() != TransferStatus::Done)
            .map(|t| t.id)
            .collect();
        self.transfer_retries.retain(|id, _| retryable.contains(id));
        egui::Panel::bottom("transfers")
            .resizable(true)
            .default_size((ui.ctx().content_rect().height()*0.28).clamp(90.0,180.0))
            .size_range(90.0..=(ui.ctx().content_rect().height()*0.4).clamp(90.0,420.0))
            .frame(egui::Frame::new().fill(pal.panel).stroke(Stroke::new(1.0, pal.border)).inner_margin(egui::Margin::symmetric(14, 8)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Transfer queue").strong());
                    ui.label(RichText::new(format!("{}", list.len())).small().color(pal.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(ghost_button("×")).on_hover_text("Hide").clicked() {
                            hide = true;
                        }
                        if ui.add(ghost_button("Clear finished")).clicked() {
                            clear = true;
                        }
                    });
                });
                ui.separator();
                if list.is_empty() {
                    ui.label(RichText::new("No transfers yet. Select files in an SFTP tab and copy them to the other side.").color(pal.text_dim));
                    return;
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for t in &list {
                        let status=*t.status.lock();
                        let total=t.total.load(Ordering::Relaxed);let done=t.done.load(Ordering::Relaxed);
                        let fraction=if total>0{(done as f32/total as f32).clamp(0.0,1.0)}else if status==TransferStatus::Done{1.0}else{0.0};
                        let (label,color)=match status{
                            TransferStatus::Queued=>("Queued",pal.text_dim),TransferStatus::Running=>("Transferring",pal.accent),TransferStatus::Done=>("Completed",pal.success),TransferStatus::Failed=>("Failed",pal.danger),TransferStatus::Cancelled=>("Cancelled",pal.warning)
                        };
                        ui.horizontal(|ui|{
                            ui.label(RichText::new(label).small().color(color));
                            ui.add(egui::Label::new(RichText::new(&t.name).strong()).truncate()).on_hover_text(&t.name);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                                if matches!(status,TransferStatus::Running|TransferStatus::Queued)&&ui.small_button("Cancel transfer").clicked(){t.cancel.store(true,Ordering::Relaxed);}
                                if matches!(status,TransferStatus::Failed|TransferStatus::Cancelled)&&self.transfer_retries.contains_key(&t.id)&&ui.small_button("Retry transfer").clicked(){retry=Some(t.id);}
                            });
                        });
                        let path=format!("{} → {}",t.from,t.to);
                        ui.add(egui::Label::new(RichText::new(&path).small().color(pal.text_dim)).truncate()).on_hover_text(path);
                        ui.add(egui::ProgressBar::new(fraction).show_percentage().desired_width(ui.available_width()).fill(color));
                        let detail=match status{
                            TransferStatus::Failed=>t.error.lock().clone().unwrap_or_else(||"Transfer failed. Retry or check both locations.".into()),
                            TransferStatus::Running=>format!("{} / {} · {}/s",human_size(done),human_size(total),human_size((done as f64/t.started.elapsed().as_secs_f64().max(0.001))as u64)),
                            _=>format!("{} / {}",human_size(done),human_size(total.max(done))),
                        };
                        ui.add(egui::Label::new(RichText::new(detail).small().color(if status==TransferStatus::Failed{pal.danger}else{pal.text_dim})).wrap());
                        ui.add_space(8.0);ui.separator();
                    }
                });
            });
        if let Some(id) = retry {
            self.retry_file_transfer(id);
        }
        if clear {
            self.transfers.lock().retain(|t| {
                matches!(
                    *t.status.lock(),
                    TransferStatus::Running | TransferStatus::Queued
                )
            });
        }
        if clear {
            let retained: std::collections::HashSet<_> =
                self.transfers.lock().iter().map(|t| t.id).collect();
            self.transfer_retries.retain(|id, _| retained.contains(id));
        }
        if hide {
            self.show_transfers = false;
        }
    }
}

fn selected_entries(p: &FsPane) -> Vec<FileEntry> {
    p.entries
        .iter()
        .filter(|e| p.selected.contains(&e.path))
        .cloned()
        .collect()
}

fn visible_entries(p: &FsPane, show_hidden: bool) -> Vec<FileEntry> {
    let q = p.filter.to_lowercase();
    let mut v: Vec<FileEntry> = p
        .entries
        .iter()
        .filter(|e| show_hidden || !e.name.starts_with('.'))
        .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q))
        .cloned()
        .collect();
    v.sort_by(|a, b| {
        let ord = match p.sort {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Modified => a.mtime.cmp(&b.mtime),
        };
        let ord = if p.sort_asc { ord } else { ord.reverse() };
        // Folders always first.
        b.is_dir.cmp(&a.is_dir).then(ord)
    });
    v
}

fn fmt_time(ts: Option<i64>) -> String {
    ts.and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn fs_pane(
    ui: &mut egui::Ui,
    p: &mut FsPane,
    other_id: u64,
    other_selected: &[FileEntry],
    other_backend: Option<&Backend>,
    hosts: &[(Uuid, String, String)],
    show_hidden: bool,
    pal: Palette,
    actions: &mut Vec<FsAction>,
    toggle_hidden: &mut bool,
) {
    let id = p.id;
    egui::Frame::new()
        .fill(pal.panel)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(0)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());

            // ----- Location header -----
            ui.horizontal(|ui| {
                let label = match &p.backend {
                    Some(b) => b.label(),
                    None if p.connecting => "Connecting…".into(),
                    None => "Choose a location".into(),
                };
                let icon = if p.backend.as_ref().is_some_and(|b| b.is_local()) {
                    "\u{E586}"
                } else {
                    "\u{E2A0}"
                };
                ui.label(RichText::new(icon).size(16.0));
                ui.menu_button(RichText::new(format!("{label}  \u{E136}")).strong(), |ui| {
                    ui.set_min_width(240.0);
                    if ui.button("\u{E586}  Local computer").clicked() {
                        actions.push(FsAction::UseLocal(id));
                        ui.close();
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for (hid, name, addr) in hosts {
                                if ui
                                    .button(format!("\u{E2A0}  {name}"))
                                    .on_hover_text(addr)
                                    .clicked()
                                {
                                    actions.push(FsAction::Connect(id, *hid));
                                    ui.close();
                                }
                            }
                        });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if matches!(p.backend, Some(Backend::Remote { .. }))
                        && ui
                            .add(ghost_button("\u{E212}"))
                            .on_hover_text("Disconnect")
                            .clicked()
                    {
                        actions.push(FsAction::Disconnect(id));
                    }
                    let eye = if show_hidden { "\u{E220}" } else { "\u{E224}" };
                    if ui
                        .add(ghost_button(eye))
                        .on_hover_text("Show hidden files")
                        .clicked()
                    {
                        *toggle_hidden = true;
                    }
                });
            });

            if p.backend.is_none() {
                ui.add_space(12.0);
                if p.connecting {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Connecting…");
                    });
                } else {
                    host_picker(ui, id, hosts, pal, actions);
                }
                if let Some(e) = &p.error {
                    ui.add_space(8.0);
                    ui.colored_label(pal.danger, e);
                }
                return;
            }
            let backend = p.backend.clone().unwrap_or(Backend::Local);

            // ----- Navigation bar -----
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!p.back.is_empty(), egui::Button::new("←"))
                    .on_hover_text("Back")
                    .clicked()
                {
                    actions.push(FsAction::Back(id));
                }
                if ui
                    .add_enabled(!p.forward.is_empty(), egui::Button::new("→"))
                    .on_hover_text("Forward")
                    .clicked()
                {
                    actions.push(FsAction::Forward(id));
                }
                let parent = backend.parent(&p.path);
                if ui
                    .add_enabled(parent.is_some(), egui::Button::new("↑"))
                    .on_hover_text("Parent folder")
                    .clicked()
                {
                    if let Some(par) = parent {
                        actions.push(FsAction::Navigate(id, par));
                    }
                }
                if ui.button("\u{E094}").on_hover_text("Refresh").clicked() {
                    p.forward.clear();
                    actions.push(FsAction::Navigate(id, p.path.clone()));
                    p.path.clear();
                }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut p.path_edit)
                        .font(egui::TextStyle::Monospace)
                        .desired_width((ui.available_width() - 160.0).max(80.0)),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    actions.push(FsAction::Navigate(id, p.path_edit.trim().to_string()));
                }
                ui.add(
                    egui::TextEdit::singleline(&mut p.filter)
                        .hint_text("\u{E30C} Filter")
                        .desired_width(140.0),
                );
            });

            // ----- Directional actions -----
            let selected = selected_entries(p);
            ui.horizontal_wrapped(|ui| {
                let direction = match (backend.is_local(), other_backend.map(Backend::is_local)) {
                    (true, Some(false)) => "Upload selected",
                    (false, Some(true)) => "Download selected",
                    _ => "Copy selected",
                };
                let label = format!("{direction} ({})", selected.len());
                if ui
                    .add_enabled(
                        !selected.is_empty() && other_backend.is_some(),
                        primary_button(&label, pal),
                    )
                    .on_disabled_hover_text(if other_backend.is_none() {
                        "Choose a destination in the other location first."
                    } else {
                        "Select one or more files first."
                    })
                    .clicked()
                {
                    actions.push(FsAction::Copy {
                        from: id,
                        entries: selected.clone(),
                        to: other_id,
                    });
                }
                if !other_selected.is_empty()
                    && ui
                        .add(ghost_button(&format!(
                            "\u{E058} Copy {} here",
                            other_selected.len()
                        )))
                        .clicked()
                {
                    actions.push(FsAction::Copy {
                        from: other_id,
                        entries: other_selected.to_vec(),
                        to: id,
                    });
                }
                ui.menu_button("+ New", |ui| {
                    if ui.button("\u{E25A}  Folder").clicked() {
                        p.new_folder = Some(String::new());
                        ui.close();
                    }
                    if ui.button("\u{E230}  File").clicked() {
                        p.new_file = Some(String::new());
                        ui.close();
                    }
                });
                let one = (selected.len() == 1).then(|| selected[0].clone());
                if ui
                    .add_enabled(one.is_some(), egui::Button::new("Rename"))
                    .clicked()
                {
                    if let Some(e) = &one {
                        p.rename = Some((e.path.clone(), e.name.clone()));
                    }
                }
                if !backend.is_local()
                    && ui
                        .add_enabled(one.is_some(), egui::Button::new("Permissions"))
                        .clicked()
                {
                    if let Some(e) = &one {
                        p.chmod = Some((e.path.clone(), e.perms.unwrap_or(0o644) & 0o7777));
                    }
                }
                if ui
                    .add_enabled(!selected.is_empty(), danger_button("Delete", pal))
                    .clicked()
                {
                    p.confirm_delete = Some(selected.clone());
                }
            });

            if let Some(e) = &p.error {
                ui.colored_label(pal.danger, e);
            }

            // ----- Inline editors (new folder / file, rename, chmod, delete) -----
            inline_forms(ui, p, &backend, pal, actions);

            // ----- File list -----
            ui.add_space(4.0);
            let entries = visible_entries(p, show_hidden);
            header_row(ui, p, pal);
            if p.loading && p.entries.is_empty() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading files…");
                });
            } else if entries.is_empty() {
                ui.add_space(16.0);
                ui.label(
                    RichText::new(if p.filter.is_empty() {
                        "This folder is empty."
                    } else {
                        "No files match the filter."
                    })
                    .color(pal.text_dim),
                );
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .id_salt(("fs-list", id))
                .show_rows(ui, ROW_H, entries.len(), |ui, range| {
                    for e in &entries[range] {
                        let resp = file_row(ui, e, p.selected.contains(&e.path), pal);
                        if resp.clicked() {
                            let multi = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                            if multi {
                                if !p.selected.remove(&e.path) {
                                    p.selected.insert(e.path.clone());
                                }
                            } else {
                                p.selected.clear();
                                p.selected.insert(e.path.clone());
                            }
                        }
                        if resp.double_clicked() {
                            if e.is_dir {
                                actions.push(FsAction::Navigate(id, e.path.clone()));
                            } else if backend.is_local() {
                                actions.push(FsAction::OpenLocal(e.path.clone()));
                            } else {
                                actions.push(FsAction::Edit(id, e.path.clone()));
                            }
                        }
                        resp.context_menu(|ui| {
                            if !p.selected.contains(&e.path) {
                                p.selected.clear();
                                p.selected.insert(e.path.clone());
                            }
                            if e.is_dir && ui.button("Open").clicked() {
                                actions.push(FsAction::Navigate(id, e.path.clone()));
                                ui.close();
                            }
                            if !e.is_dir
                                && ui
                                    .button(if backend.is_local() {
                                        "Open"
                                    } else {
                                        "Edit in external editor"
                                    })
                                    .clicked()
                            {
                                actions.push(if backend.is_local() {
                                    FsAction::OpenLocal(e.path.clone())
                                } else {
                                    FsAction::Edit(id, e.path.clone())
                                });
                                ui.close();
                            }
                            if ui.button("Copy to other side").clicked() {
                                actions.push(FsAction::Copy {
                                    from: id,
                                    entries: selected_entries(p),
                                    to: other_id,
                                });
                                ui.close();
                            }
                            if ui.button("Copy path").clicked() {
                                ui.ctx().copy_text(e.path.clone());
                                ui.close();
                            }
                            if ui.button("Rename").clicked() {
                                p.rename = Some((e.path.clone(), e.name.clone()));
                                ui.close();
                            }
                            if !backend.is_local() && ui.button("Permissions").clicked() {
                                p.chmod = Some((e.path.clone(), e.perms.unwrap_or(0o644) & 0o7777));
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .button(RichText::new("Delete").color(pal.danger))
                                .clicked()
                            {
                                p.confirm_delete = Some(selected_entries(p));
                                ui.close();
                            }
                        });
                    }
                });
            ui.label(
                RichText::new(format!(
                    "{} items · {} selected",
                    entries.len(),
                    p.selected.len()
                ))
                .small()
                .color(pal.text_dim),
            );
        });
}

fn host_picker(
    ui: &mut egui::Ui,
    id: u64,
    hosts: &[(Uuid, String, String)],
    pal: Palette,
    actions: &mut Vec<FsAction>,
) {
    ui.label(RichText::new("Open a location").size(16.0).strong());
    ui.label(
        RichText::new("Pick a saved SSH host to browse over SFTP, or your local files.")
            .color(pal.text_dim),
    );
    ui.add_space(8.0);
    if ui.add(ghost_button("\u{E586}  Local computer")).clicked() {
        actions.push(FsAction::UseLocal(id));
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .id_salt(("picker", id))
        .show(ui, |ui| {
            for (hid, name, addr) in hosts {
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
                let fill = if resp.hovered() {
                    pal.card_hover
                } else {
                    pal.card
                };
                ui.painter().rect_filled(rect, 8.0, fill);
                let c = super::hash_color(name);
                ui.painter()
                    .circle_filled(Pos2::new(rect.left() + 20.0, rect.center().y), 11.0, c);
                ui.painter().text(
                    Pos2::new(rect.left() + 20.0, rect.center().y),
                    Align2::CENTER_CENTER,
                    name.chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string(),
                    FontId::proportional(12.0),
                    crate::themes::readable_on(c),
                );
                ui.painter().text(
                    Pos2::new(rect.left() + 40.0, rect.top() + 12.0),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(14.0),
                    pal.text,
                );
                ui.painter().text(
                    Pos2::new(rect.left() + 40.0, rect.top() + 28.0),
                    Align2::LEFT_CENTER,
                    addr,
                    FontId::proportional(11.0),
                    pal.text_dim,
                );
                if resp.clicked() {
                    actions.push(FsAction::Connect(id, *hid));
                }
                ui.add_space(4.0);
            }
            if hosts.is_empty() {
                ui.label(
                    RichText::new("No SSH hosts saved yet — add one in the Vault.")
                        .color(pal.text_dim),
                );
            }
        });
}

fn inline_forms(
    ui: &mut egui::Ui,
    p: &mut FsPane,
    backend: &Backend,
    pal: Palette,
    actions: &mut Vec<FsAction>,
) {
    let id = p.id;
    let dir = p.path.clone();
    let form = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
        egui::Frame::new()
            .fill(pal.card)
            .corner_radius(8)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| add(ui));
            });
    };
    let enter = |ui: &egui::Ui, r: &egui::Response| {
        r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
    };

    if let Some(name) = p.new_folder.as_mut() {
        let mut done = None;
        form(ui, &mut |ui| {
            ui.label("New folder");
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(200.0));
            r.request_focus();
            if (ui.add(primary_button("Create", pal)).clicked() || enter(ui, &r))
                && !name.trim().is_empty()
            {
                done = Some(Some(backend.join(&dir, name.trim())));
            }
            if ui.add(ghost_button("Cancel")).clicked() {
                done = Some(None);
            }
        });
        if let Some(d) = done {
            if let Some(path) = d {
                actions.push(FsAction::Mkdir(id, path));
            }
            p.new_folder = None;
        }
    }
    if let Some(name) = p.new_file.as_mut() {
        let mut done = None;
        form(ui, &mut |ui| {
            ui.label("New file");
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(200.0));
            r.request_focus();
            if (ui.add(primary_button("Create", pal)).clicked() || enter(ui, &r))
                && !name.trim().is_empty()
            {
                done = Some(Some(backend.join(&dir, name.trim())));
            }
            if ui.add(ghost_button("Cancel")).clicked() {
                done = Some(None);
            }
        });
        if let Some(d) = done {
            if let Some(path) = d {
                actions.push(FsAction::Touch(id, path));
            }
            p.new_file = None;
        }
    }
    if let Some((from, name)) = p.rename.as_mut() {
        let mut done = None;
        form(ui, &mut |ui| {
            ui.label("Rename to");
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(220.0));
            if (ui.add(primary_button("Rename", pal)).clicked() || enter(ui, &r))
                && !name.trim().is_empty()
            {
                done = Some(Some((from.clone(), backend.join(&dir, name.trim()))));
            }
            if ui.add(ghost_button("Cancel")).clicked() {
                done = Some(None);
            }
        });
        if let Some(d) = done {
            if let Some((a, b)) = d {
                actions.push(FsAction::Rename(id, a, b));
            }
            p.rename = None;
        }
    }
    if let Some((path, mode)) = p.chmod.as_mut() {
        let mut done = None;
        form(ui, &mut |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Permissions · {}",
                        path.rsplit('/').next().unwrap_or(path)
                    ))
                    .strong(),
                );
                egui::Grid::new(("chmod", id))
                    .spacing([10.0, 2.0])
                    .show(ui, |ui| {
                        ui.label("");
                        for h in ["Read", "Write", "Exec"] {
                            ui.label(RichText::new(h).small().color(pal.text_dim));
                        }
                        ui.end_row();
                        for (who, shift) in [("Owner", 6u32), ("Group", 3), ("Others", 0)] {
                            ui.label(who);
                            for bit in [4u32, 2, 1] {
                                let mask = bit << shift;
                                let mut on = *mode & mask != 0;
                                if ui.checkbox(&mut on, "").changed() {
                                    if on { *mode |= mask } else { *mode &= !mask }
                                }
                            }
                            ui.end_row();
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:o}  {}", mode, perms_string(*mode, false)))
                            .monospace()
                            .color(pal.text_dim),
                    );
                    if ui.add(primary_button("Apply", pal)).clicked() {
                        done = Some(Some((path.clone(), *mode)));
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        done = Some(None);
                    }
                });
            });
        });
        if let Some(d) = done {
            if let Some((path, mode)) = d {
                actions.push(FsAction::Chmod(id, path, mode));
            }
            p.chmod = None;
        }
    }
    if let Some(list) = p.confirm_delete.as_ref() {
        let mut done = None;
        form(ui, &mut |ui| {
            let what = if list.len() == 1 {
                format!("“{}”", list[0].name)
            } else {
                format!("{} items", list.len())
            };
            ui.colored_label(
                pal.danger,
                // unslop-ignore NO_SQL_INTERPOLATION -- UI confirmation text for deleting selected files; no SQL is used.
                format!("Delete {what}? Folders are removed with everything inside."),
            );
            if ui.add(danger_button("Delete", pal)).clicked() {
                done = Some(true);
            }
            if ui.add(ghost_button("Cancel")).clicked() {
                done = Some(false);
            }
        });
        if let Some(yes) = done {
            if yes {
                actions.push(FsAction::Delete(id, list.clone()));
            }
            p.confirm_delete = None;
        }
    }
}

fn columns(rect: Rect) -> [f32; 4] {
    let w = rect.width();
    // name | size | modified | permissions
    let perms_w = if w > 520.0 { 96.0 } else { 0.0 };
    let date_w = if w > 380.0 { 128.0 } else { 0.0 };
    let size_w = 72.0;
    let right = rect.right() - 8.0;
    [
        rect.left() + 34.0,
        right - perms_w - date_w - size_w,
        right - perms_w - date_w,
        right - perms_w,
    ]
}

fn header_row(ui: &mut egui::Ui, p: &mut FsPane, pal: Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    let cols = columns(rect);
    let pane_id = p.id;
    let mut click = |x0: f32, x1: f32, key: SortKey, label: &str, align_right: bool| {
        let r = Rect::from_x_y_ranges(x0..=x1.max(x0 + 1.0), rect.y_range());
        let resp = ui.interact(
            r,
            ui.scope_id().with(("hdr", pane_id, label)),
            Sense::click(),
        );
        let arrow = if p.sort == key {
            if p.sort_asc { " ▲" } else { " ▼" }
        } else {
            ""
        };
        let color = if resp.hovered() {
            pal.text
        } else {
            pal.text_dim
        };
        let (pos, align) = if align_right {
            (Pos2::new(x1 - 4.0, r.center().y), Align2::RIGHT_CENTER)
        } else {
            (Pos2::new(x0, r.center().y), Align2::LEFT_CENTER)
        };
        ui.painter().text(
            pos,
            align,
            format!("{label}{arrow}"),
            FontId::proportional(11.0),
            color,
        );
        if resp.clicked() {
            if p.sort == key {
                p.sort_asc = !p.sort_asc;
            } else {
                p.sort = key;
                p.sort_asc = true;
            }
        }
    };
    click(cols[0], cols[1], SortKey::Name, "NAME", false);
    click(cols[1], cols[2], SortKey::Size, "SIZE", true);
    if cols[3] > cols[2] {
        click(cols[2] + 8.0, cols[3], SortKey::Modified, "MODIFIED", false);
    }
    if rect.right() - 8.0 > cols[3] {
        ui.painter().text(
            Pos2::new(cols[3] + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            "PERMS",
            FontId::proportional(11.0),
            pal.text_dim,
        );
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, pal.border));
}

fn file_row(ui: &mut egui::Ui, e: &FileEntry, selected: bool, pal: Palette) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 6.0, pal.accent_soft);
    } else if resp.hovered() {
        painter.rect_filled(rect, 6.0, pal.card_hover);
    }
    let cols = columns(rect);
    let cy = rect.center().y;
    let (icon, icolor) = if e.is_dir {
        ("\u{E25A}", pal.accent)
    } else if e.is_link {
        ("\u{E2E6}", pal.text_dim)
    } else {
        (file_icon(&e.name), pal.text_dim)
    };
    painter.text(
        Pos2::new(rect.left() + 16.0, cy),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(13.0),
        icolor,
    );
    let name_w = (cols[1] - cols[0] - 8.0).max(20.0);
    let galley = painter.layout(
        e.name.clone(),
        FontId::proportional(13.0),
        pal.text,
        f32::INFINITY,
    );
    let clip = Rect::from_x_y_ranges(cols[0]..=cols[0] + name_w, rect.y_range());
    painter.with_clip_rect(clip).galley(
        Pos2::new(cols[0], cy - galley.size().y / 2.0),
        galley,
        pal.text,
    );
    if !e.is_dir {
        painter.text(
            Pos2::new(cols[2] - 4.0, cy),
            Align2::RIGHT_CENTER,
            human_size(e.size),
            FontId::proportional(12.0),
            pal.text_dim,
        );
    }
    if cols[3] > cols[2] {
        painter.text(
            Pos2::new(cols[2] + 8.0, cy),
            Align2::LEFT_CENTER,
            fmt_time(e.mtime),
            FontId::proportional(12.0),
            pal.text_dim,
        );
    }
    if rect.right() - 8.0 > cols[3] {
        if let Some(m) = e.perms {
            painter.text(
                Pos2::new(cols[3] + 8.0, cy),
                Align2::LEFT_CENTER,
                perms_string(m, e.is_dir),
                FontId::monospace(11.0),
                pal.text_dim,
            );
        }
    }
    let tip = if e.owner.is_empty() {
        e.path.clone()
    } else {
        format!("{}\nowner: {}", e.path, e.owner)
    };
    resp.on_hover_text_at_pointer(tip)
}

fn file_icon(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" => "\u{E2CA}",
        "zip" | "gz" | "tgz" | "xz" | "bz2" | "7z" | "rar" | "tar" | "zst" => "\u{E390}",
        "sh" | "bash" | "zsh" | "py" | "rb" | "js" | "ts" | "rs" | "go" | "c" | "h" | "cpp"
        | "java" | "php" => "\u{E914}",
        "log" | "txt" | "md" | "conf" | "cfg" | "ini" | "yml" | "yaml" | "toml" | "json"
        | "xml" => "\u{E230}",
        "pem" | "key" | "pub" | "crt" => "\u{E2D6}",
        _ => "\u{E230}",
    }
}
