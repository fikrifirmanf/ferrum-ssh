//! Side panel listing Docker/Podman containers and Kubernetes pods of the active SSH session.

use egui::{Color32, RichText};

use super::card_frame;
use crate::containers::{Action, Browser, Item, Runtime};
use crate::themes::Palette;

pub enum Source<'a> {
    Ssh(&'a str),
    Unsupported,
    Disconnected(&'a str),
}

/// What the user asked for; the caller owns the connection and runs it.
pub enum Command {
    Act(Item, Action),
    Logs(Item),
    /// Text to type into the terminal (not run).
    Shell(String),
    /// Retry Docker and Podman through `sudo -n`.
    UseSudo,
}

pub fn show(
    ui: &mut egui::Ui,
    pal: Palette,
    browser: &mut Browser,
    source: Source,
) -> Option<Command> {
    ui.label(
        RichText::new(format!("{} Containers", crate::icons::PACKAGE))
            .strong()
            .size(16.0),
    );
    let host = match source {
        Source::Ssh(host) => host,
        Source::Unsupported => {
            dim(ui, pal, "Containers are available for SSH connections.");
            return None;
        }
        Source::Disconnected(host) => {
            dim(ui, pal, &format!("{host} is not connected."));
            return None;
        }
    };
    if browser.logs.is_some() {
        return logs_view(ui, pal, browser);
    }
    dim(ui, pal, host);
    if let Some(error) = &browser.error {
        ui.colored_label(pal.warning, format!("Last update failed: {error}"));
    }
    match &browser.notice {
        Some(Ok(done)) => dim(ui, pal, done),
        Some(Err(error)) => {
            ui.colored_label(pal.danger, error);
        }
        None => {}
    }
    let Some(listing) = &browser.listing else {
        if browser.error.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                dim(ui, pal, "Looking for Docker, Podman and kubectl…");
            });
        }
        return None;
    };
    let mut command = None;
    for (runtime, message) in &listing.errors {
        ui.colored_label(pal.warning, format!("{}: {message}", runtime.label()));
        if crate::containers::is_permission_error(message) {
            if browser.use_sudo {
                dim(
                    ui,
                    pal,
                    "sudo did not work either. It must be allowed without a password (sudo -n).",
                );
            } else {
                dim(
                    ui,
                    pal,
                    "Your user can't use the Docker socket. Add it to the docker group \
                     (sudo usermod -aG docker $USER, then reconnect), or use sudo.",
                );
                if ui.button("Retry with sudo").clicked() {
                    command = Some(Command::UseSudo);
                }
            }
        }
    }
    if listing.items.is_empty() {
        if listing.errors.is_empty() {
            dim(
                ui,
                pal,
                "No containers or pods found. Docker, Podman or kubectl must be installed on the server.",
            );
        }
        return command;
    }
    ui.add(
        egui::TextEdit::singleline(&mut browser.filter)
            .hint_text("Filter")
            .desired_width(f32::INFINITY),
    );
    let query = browser.filter.to_lowercase();
    let busy = browser.busy.is_some();
    if let Some(label) = &browser.busy {
        ui.horizontal(|ui| {
            ui.spinner();
            dim(ui, pal, label);
        });
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for runtime in [Runtime::Docker, Runtime::Podman, Runtime::Kubernetes] {
                let items: Vec<&Item> = listing
                    .items
                    .iter()
                    .filter(|i| i.runtime == runtime)
                    .filter(|i| {
                        query.is_empty()
                            || i.name.to_lowercase().contains(&query)
                            || i.detail.to_lowercase().contains(&query)
                    })
                    .collect();
                if items.is_empty() {
                    continue;
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{} ({})", runtime.label(), items.len()))
                        .small()
                        .color(pal.text_dim),
                );
                for item in items {
                    if let Some(c) = item_card(ui, pal, item, busy) {
                        command = Some(c);
                    }
                }
            }
        });
    command
}

fn item_card(ui: &mut egui::Ui, pal: Palette, item: &Item, busy: bool) -> Option<Command> {
    let mut command = None;
    card_frame(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let color = if item.is_failing() {
                pal.danger
            } else if item.is_running() {
                Color32::from_rgb(0x3f, 0xb9, 0x50)
            } else {
                pal.text_dim
            };
            ui.label(RichText::new(crate::icons::CIRCLE).color(color).small());
            ui.label(RichText::new(&item.name).strong());
        });
        let kube = item.runtime == Runtime::Kubernetes;
        let summary = if kube {
            format!("{} · {} · {}", item.scope, item.detail, item.state)
        } else {
            item.detail.clone()
        };
        ui.label(RichText::new(summary).small().color(pal.text_dim));
        let status = if kube { &item.status } else { &item.state };
        let status = if kube {
            status.to_owned()
        } else {
            format!("{} · {}", item.status, item.ports)
                .trim_end_matches(" · ")
                .to_owned()
        };
        if !status.is_empty() {
            ui.label(RichText::new(status).small().color(pal.text_dim));
        }
        ui.horizontal_wrapped(|ui| {
            if !kube {
                let actions: &[Action] = if item.is_running() {
                    &[Action::Stop, Action::Restart]
                } else {
                    &[Action::Start]
                };
                for &action in actions {
                    if ui
                        .add_enabled(!busy, egui::Button::new(action.label()).small())
                        .clicked()
                    {
                        command = Some(Command::Act(item.clone(), action));
                    }
                }
            }
            if ui.add(egui::Button::new("Logs").small()).clicked() {
                command = Some(Command::Logs(item.clone()));
            }
            if item.is_running() {
                if let Some(text) = item.shell_command() {
                    if ui
                        .add(egui::Button::new("Shell").small())
                        .on_hover_text(format!("Types `{text}` into the terminal"))
                        .clicked()
                    {
                        command = Some(Command::Shell(text));
                    }
                }
            }
        });
    });
    ui.add_space(6.0);
    command
}

fn logs_view(ui: &mut egui::Ui, pal: Palette, browser: &mut Browser) -> Option<Command> {
    let mut command = None;
    let mut close = false;
    let view = browser.logs.as_ref()?;
    ui.horizontal(|ui| {
        if ui.button("‹ Back").clicked() {
            close = true;
        }
        ui.label(RichText::new(&view.item.name).strong());
        if ui.button("Refresh").clicked() {
            command = Some(Command::Logs(view.item.clone()));
        }
    });
    match &view.text {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                dim(ui, pal, "Loading logs…");
            });
        }
        Some(Err(error)) => {
            ui.colored_label(pal.danger, error);
        }
        Some(Ok(text)) if text.trim().is_empty() => dim(ui, pal, "No log output."),
        Some(Ok(text)) => {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).monospace().small()).selectable(true),
                    );
                });
        }
    }
    if close {
        browser.logs = None;
    }
    command
}

fn dim(ui: &mut egui::Ui, pal: Palette, text: &str) {
    ui.label(RichText::new(text).small().color(pal.text_dim));
}
