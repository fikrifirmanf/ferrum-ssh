//! Folder comparison dialog. Workers own captured locations, never the live pane paths.
use super::forms;
use crate::sftp::sync::{self, Location, Mode, Plan};
use crate::themes::Palette;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
enum Phase {
    Ready,
    Comparing,
    Preview(Arc<Plan>),
    Applying { done: usize, total: usize },
    Finished(Result<usize, String>),
    Error(String),
}

pub struct SyncDialog {
    locations: [Location; 2],
    left_to_right: bool,
    mode: Mode,
    exclusions: String,
    phase: Arc<Mutex<Phase>>,
    cancel: Arc<AtomicBool>,
    confirm_delete: bool,
}

impl Drop for SyncDialog {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl SyncDialog {
    pub fn new(left: Location, right: Location) -> Self {
        Self {
            locations: [left, right],
            left_to_right: true,
            mode: Mode::Update,
            exclusions: ".git, node_modules".into(),
            phase: Arc::new(Mutex::new(Phase::Ready)),
            cancel: Arc::new(AtomicBool::new(false)),
            confirm_delete: false,
        }
    }

    pub(crate) fn compare(&mut self, rt: &tokio::runtime::Handle, ctx: &egui::Context) {
        self.cancel.store(false, Ordering::Relaxed);
        self.confirm_delete = false;
        let [left, right] = self.locations.clone();
        let (source, destination) = if self.left_to_right {
            (left, right)
        } else {
            (right, left)
        };
        let (mode, exclude) = (self.mode, self.exclusions.clone());
        let (phase, cancel, ctx) = (self.phase.clone(), self.cancel.clone(), ctx.clone());
        *phase.lock() = Phase::Comparing;
        rt.spawn(async move {
            let result = sync::compare(source, destination, mode, exclude, &cancel).await;
            *phase.lock() = match result {
                Ok(plan) => Phase::Preview(Arc::new(plan)),
                Err(error) => Phase::Error(error.to_string()),
            };
            ctx.request_repaint();
        });
    }

    fn apply(&mut self, plan: Arc<Plan>, rt: &tokio::runtime::Handle, ctx: &egui::Context) {
        self.cancel.store(false, Ordering::Relaxed);
        let (phase, cancel, ctx) = (self.phase.clone(), self.cancel.clone(), ctx.clone());
        *phase.lock() = Phase::Applying {
            done: 0,
            total: plan.changes.len(),
        };
        rt.spawn(async move {
            let result = sync::apply(&plan, &cancel, |done, total| {
                *phase.lock() = Phase::Applying { done, total };
                ctx.request_repaint();
            })
            .await
            .map_err(|error| error.to_string());
            *phase.lock() = Phase::Finished(result);
            ctx.request_repaint();
        });
    }

    /// Returns true when the user closes the dialog. Busy work must cancel before closing.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        tab: u64,
        pal: Palette,
        rt: &tokio::runtime::Handle,
    ) -> bool {
        let phase = self.phase.lock().clone();
        let busy = matches!(phase, Phase::Comparing | Phase::Applying { .. });
        let mut close = false;
        let modal_id = egui::Id::unique(("folder-sync", tab));
        egui::Modal::new(modal_id)
            .area(egui::Modal::default_area(modal_id).fade_in(false))
            .backdrop_color(pal.scrim)
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                super::glass::overlay_frame(ui, crate::themes::tokens::RADIUS_OVERLAY as f32, egui::Margin::same(24), |ui| {
                ui.set_width(620.0_f32.min((ctx.content_rect().width() - 64.0).max(240.0)));
                ui.set_max_height((ctx.content_rect().height() - 100.0).max(160.0));
                egui::ScrollArea::vertical().id_salt("sync-dialog-content")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .max_height((ctx.content_rect().height() - 100.0).max(160.0))
                    .show(ui, |ui| {
                forms::heading(ui, "Sync folders", 18.0);
                ui.label("Compare file contents, then review the changes before applying them.");
                ui.add_space(8.0);
                for (name, location) in ["Left", "Right"].iter().zip(&self.locations) {
                    ui.label(format!("{name}: {}", location.backend.label()));
                    ui.add(egui::Label::new(egui::RichText::new(&location.path).monospace()).wrap());
                }
                ui.add_space(8.0);
                let mut changed = false;
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        changed |= ui.selectable_value(&mut self.left_to_right, true, "Left → Right").changed();
                        changed |= ui.selectable_value(&mut self.left_to_right, false, "Right → Left").changed();
                    });
                    ui.horizontal_wrapped(|ui| {
                        changed |= ui.selectable_value(&mut self.mode, Mode::Update, "Copy / update").changed();
                        changed |= ui.selectable_value(&mut self.mode, Mode::Mirror, "Mirror files").changed();
                    });
                    let hint = if self.mode == Mode::Mirror {
                        "Mirror also deletes destination-only files. Empty destination folders are kept."
                    } else { "Copy new and changed files. Destination-only files are kept." };
                    ui.label(hint);
                    changed |= forms::Input::new("Exclude names or paths")
                        .hint(".git, node_modules, *.log, build/*")
                        .help("Comma-separated patterns. * matches any text; ? matches one character.")
                        .show(ui, &mut self.exclusions).changed();
                    ui.label("Sync includes hidden files. Exclude links and special files before comparing.");
                });
                if changed {
                    *self.phase.lock() = Phase::Ready;
                    self.confirm_delete = false;
                }
                ui.separator();
                // Do not show or apply a preview after its options have changed this frame.
                match if changed { Phase::Ready } else { phase.clone() } {
                    Phase::Ready => { ui.label("Compare the selected folders to see what will change."); }
                    Phase::Comparing => { ui.horizontal(|ui| { ui.spinner(); ui.label("Reading folders and comparing file contents…"); }); }
                    Phase::Preview(plan) => {
                        if plan.changes.is_empty() { ui.label("Folders match. No changes to apply."); }
                        else {
                            let bytes: u64 = plan.changes.iter().map(|c| match c { sync::Change::CopyFile { size, .. } => *size, _ => 0 }).sum();
                            ui.label(format!("{} changes · {} to copy · {} deletions", plan.changes.len(), crate::sftp::human_size(bytes), plan.deletions()));
                            egui::ScrollArea::vertical().id_salt("sync-changes")
                                .max_height((ctx.content_rect().height() * 0.25).min(240.0))
                                .show_rows(ui, 28.0, plan.changes.len(), |ui, range| {
                                    for index in range {
                                        let change = &plan.changes[index];
                                        ui.horizontal(|ui| {
                                            ui.label(change.label());
                                            ui.add(egui::Label::new(egui::RichText::new(change.path()).monospace()).truncate()).on_hover_text(change.path());
                                        });
                                    }
                                });
                            if plan.conflicts() { ui.colored_label(pal.danger, "Resolve file/folder conflicts or exclude those paths, then compare again."); }
                            if plan.deletions() > 0 {
                                // unslop-ignore NO_SQL_INTERPOLATION -- This formats a file-deletion confirmation label, not SQL.
                                ui.checkbox(&mut self.confirm_delete, format!("Delete {} destination-only files", plan.deletions()));
                            }
                            let allowed = !plan.conflicts() && (plan.deletions() == 0 || self.confirm_delete);
                            if ui.add_enabled(allowed, forms::Btn::primary("Apply changes")).clicked() { self.apply(plan, rt, ctx); }
                        }
                    }
                    Phase::Applying { done, total } => {
                        ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).text(format!("{done} / {total} changes")));
                        ui.label("Changes apply in order. Cancellation keeps completed changes.");
                    }
                    Phase::Finished(Ok(count)) => { ui.label(format!("Sync complete. Applied {count} changes.")); }
                    Phase::Finished(Err(error)) => {
                        ui.colored_label(pal.danger, error);
                        ui.label("Sync may have completed some changes. Compare again to review what remains.");
                    }
                    Phase::Error(error) => { ui.colored_label(pal.danger, error); }
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if busy {
                        if ui.add_enabled(!self.cancel.load(Ordering::Relaxed), forms::Btn::secondary("Cancel sync")).clicked() {
                            self.cancel.store(true, Ordering::Relaxed);
                        }
                        if self.cancel.load(Ordering::Relaxed) { ui.label("Cancelling…"); }
                    } else {
                        if ui.add(forms::Btn::secondary("Compare folders")).clicked() { self.compare(rt, ctx); }
                        if ui.add(forms::Btn::ghost("Close")).clicked() { close = true; }
                    }
                });
                });
            });
            });
        close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, Rect, Vec2};

    fn text_position(shape: &egui::Shape, label: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_position(shape, label)),
            _ => None,
        }
    }

    fn render(
        dialog: &mut SyncDialog,
        ctx: &egui::Context,
        events: Vec<Event>,
    ) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                dialog.show(
                    ctx,
                    1,
                    crate::themes::current(ctx),
                    &tokio::runtime::Handle::current(),
                );
                ui.label("Test workspace");
            },
        );
        output.textures_delta.clear();
        output
    }

    fn click(dialog: &mut SyncDialog, ctx: &egui::Context, label: &str) {
        render(dialog, ctx, vec![]);
        let output = render(dialog, ctx, vec![]);
        let point = output
            .shapes
            .iter()
            .find_map(|s| text_position(&s.shape, label))
            .unwrap_or_else(|| panic!("Missing visible control: {label}"));
        render(
            dialog,
            ctx,
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
            dialog,
            ctx,
            vec![Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
    }

    async fn wait_for(dialog: &SyncDialog, finished: bool) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let phase = dialog.phase.lock().clone();
                if matches!(phase, Phase::Error(_) | Phase::Finished(Err(_))) {
                    panic!("Sync worker failed");
                }
                if matches!(phase, Phase::Finished(Ok(_))) && finished {
                    break;
                }
                if matches!(phase, Phase::Preview(_)) && !finished {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Sync worker should finish");
    }

    #[tokio::test]
    async fn changing_direction_discards_the_old_preview() {
        let root = tempfile::tempdir().unwrap();
        let left = root.path().join("left");
        let right = root.path().join("right");
        tokio::fs::create_dir(&left).await.unwrap();
        tokio::fs::create_dir(&right).await.unwrap();
        tokio::fs::write(left.join("release.txt"), b"ready")
            .await
            .unwrap();
        let mut dialog = SyncDialog::new(
            Location {
                backend: crate::sftp::Backend::Local,
                path: left.to_str().unwrap().into(),
            },
            Location {
                backend: crate::sftp::Backend::Local,
                path: right.to_str().unwrap().into(),
            },
        );
        let ctx = egui::Context::default();
        click(&mut dialog, &ctx, "Compare folders");
        wait_for(&dialog, false).await;
        click(&mut dialog, &ctx, "Right → Left");
        assert!(!dialog.left_to_right);
        assert!(matches!(*dialog.phase.lock(), Phase::Ready));
        assert!(
            !tokio::fs::try_exists(right.join("release.txt"))
                .await
                .unwrap()
        );
        click(&mut dialog, &ctx, "Compare folders");
        wait_for(&dialog, false).await;
        let phase = dialog.phase.lock();
        let Phase::Preview(plan) = &*phase else {
            panic!("Expected a fresh preview");
        };
        assert!(
            plan.changes.is_empty(),
            "copy/update in reverse must keep destination-only files"
        );
    }

    #[tokio::test]
    async fn mirror_buttons_require_confirmation_then_apply_the_reviewed_changes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        tokio::fs::create_dir(&source).await.unwrap();
        tokio::fs::create_dir(&destination).await.unwrap();
        tokio::fs::write(source.join("release.txt"), b"ready")
            .await
            .unwrap();
        tokio::fs::write(destination.join("old.txt"), b"old")
            .await
            .unwrap();
        let mut dialog = SyncDialog::new(
            Location {
                backend: crate::sftp::Backend::Local,
                path: source.to_str().unwrap().into(),
            },
            Location {
                backend: crate::sftp::Backend::Local,
                path: destination.to_str().unwrap().into(),
            },
        );
        let ctx = egui::Context::default();
        // Render twice so modal placement and input hit testing have settled.
        render(&mut dialog, &ctx, vec![]);
        render(&mut dialog, &ctx, vec![]);
        click(&mut dialog, &ctx, "Mirror files");
        assert_eq!(dialog.mode, Mode::Mirror);
        click(&mut dialog, &ctx, "Compare folders");
        wait_for(&dialog, false).await;
        click(&mut dialog, &ctx, "Apply changes");
        assert!(
            matches!(*dialog.phase.lock(), Phase::Preview(_)),
            "deletions require confirmation"
        );
        assert_eq!(
            tokio::fs::read(destination.join("old.txt")).await.unwrap(),
            b"old"
        );
        click(&mut dialog, &ctx, "Delete 1 destination-only files");
        assert!(dialog.confirm_delete);
        click(&mut dialog, &ctx, "Apply changes");
        wait_for(&dialog, true).await;
        assert_eq!(
            tokio::fs::read(destination.join("release.txt"))
                .await
                .unwrap(),
            b"ready"
        );
        assert!(
            !tokio::fs::try_exists(destination.join("old.txt"))
                .await
                .unwrap()
        );
        assert!(matches!(*dialog.phase.lock(), Phase::Finished(Ok(2))));
    }
}
