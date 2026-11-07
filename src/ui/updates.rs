use super::{password_input, primary_button};
use crate::app::{App, Tab};
use crate::updates::{RELEASES_URL, State};

impl App {
    pub fn update_dialog(&mut self, ui: &mut egui::Ui) -> bool {
        let mut close = false;
        let pal = self.pal();
        ui.label(
            egui::RichText::new("Software updates")
                .size(20.0)
                .family(crate::fonts::semibold()),
        );
        ui.label(format!("Installed version: {}", env!("CARGO_PKG_VERSION")));
        ui.add_space(12.0);
        ui.add_enabled_ui(!self.updater.busy(), |ui| {
            if ui.checkbox(&mut self.updater.previews, "Include preview releases").changed() {
                self.updater.state = State::Idle;
            }
            egui::CollapsingHeader::new("Private repository access").show(ui, |ui| {
                ui.label("Uses your GitHub CLI sign-in, or a token entered below. The token stays in memory for this session.");
                ui.label("GitHub token");
                password_input(ui, &mut self.updater.token, "Optional when signed in with gh", f32::INFINITY);
            });
        });
        ui.add_space(12.0);
        let mut download = false;
        let mut restart = false;
        match &self.updater.state {
            State::Idle => {
                ui.label("Check GitHub for a newer release.");
            }
            State::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking GitHub releases…");
                });
            }
            State::Current(message) => {
                ui.label(message);
            }
            State::Failed(message) => {
                ui.colored_label(pal.danger, "Update could not complete");
                ui.label(message);
            }
            State::Available(release) => {
                ui.label(
                    egui::RichText::new(format!("Ferrum {} is available", release.tag_name))
                        .family(crate::fonts::semibold()),
                );
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        ui.label(
                            release
                                .body
                                .as_deref()
                                .unwrap_or("No release notes provided."),
                        );
                    });
                ui.add_space(8.0);
                download = ui.add(primary_button("Download update", pal)).clicked();
                ui.label(egui::RichText::new("The download is checked against GitHub’s SHA-256 digest before installation.").small().color(pal.text_dim));
            }
            State::Downloading(done, total) => {
                let fraction = if *total == 0 {
                    0.0
                } else {
                    *done as f32 / *total as f32
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .show_percentage()
                        .animate(true),
                );
                ui.label(format!(
                    "Downloaded {:.1} of {:.1} MB",
                    *done as f64 / 1_000_000.0,
                    *total as f64 / 1_000_000.0
                ));
            }
            State::Ready(prepared) => {
                ui.label(
                    egui::RichText::new(format!("Ferrum {} is ready to install", prepared.version))
                        .color(pal.success),
                );
                ui.label("The package passed its integrity checks. Ferrum will save your vault, install the update, and reopen.");
                let sessions = self
                    .tabs
                    .iter()
                    .filter_map(|tab| match tab {
                        Tab::Terminal(t) => Some(t),
                        _ => None,
                    })
                    .flat_map(|t| &t.panes)
                    .filter(|p| p.session.is_live())
                    .count();
                if sessions > 0 {
                    ui.colored_label(
                        pal.warning,
                        format!("Restarting will disconnect {sessions} active terminal sessions."),
                    );
                }
                if self.editor.is_some() {
                    ui.colored_label(
                        pal.warning,
                        "Save any changes in the open form before restarting.",
                    );
                }
                restart = ui
                    .add_enabled(
                        self.qa.is_none(),
                        primary_button("Install and restart", pal),
                    )
                    .clicked();
            }
        }
        if download {
            self.updater.download(self.rt.handle(), ui.ctx());
        }
        if restart {
            let saved = !self.loaded
                || self
                    .store
                    .save(&self.vault)
                    .map_err(|e| self.toast(format!("Cannot save the vault: {e}"), true))
                    .is_ok();
            if saved {
                match self.updater.install() {
                    Ok(()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(e) => self.toast(format!("Cannot install update: {e:#}"), true),
                }
            }
        }
        ui.add_space(12.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.updater.busy(), egui::Button::new("Check again"))
                .clicked()
            {
                self.updater.check(self.rt.handle(), ui.ctx());
            }
            ui.hyperlink_to("GitHub releases", RELEASES_URL);
            if ui.add(super::forms::Btn::ghost("Close")).clicked() {
                close = true;
            }
        });
        close
    }
}

impl App {
    /// An optional notice: opening it never starts a download or interrupts a form.
    pub fn draw_update_notice(&mut self, ctx: &egui::Context) {
        if self.locked
            || self.dialog.is_some()
            || self.editor.is_some()
            || !self.prompts.is_empty()
            || self.palette.open
            || !self.vault.settings.check_updates_on_startup
        {
            return;
        }
        let Some(release) = self.updater.notice() else {
            return;
        };
        let tag = release.tag_name.clone();
        let pal = self.pal();
        let mut open = true;
        let mut review = false;
        let mut later = false;
        egui::Window::new("Update available")
            .id(egui::Id::unique("update-notice"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(336.0)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-24.0, 76.0))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(20))
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!("Ferrum {tag} is ready to download"))
                        .family(crate::fonts::semibold()),
                );
                ui.label(
                    "Review the release when it suits you. Your current session can keep running.",
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    review = ui.add(primary_button("View update", pal)).clicked();
                    later = ui.add(super::forms::Btn::ghost("Later")).clicked();
                });
            });
        if !open || later || review || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.updater.dismiss_notice();
        }
        if review {
            self.dialog = Some(crate::app::Dialog::Updates);
        }
    }
}
