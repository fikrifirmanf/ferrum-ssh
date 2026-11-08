//! Native macOS menus and a matching menu bar on Windows/Linux.
use crate::app::{App, Dialog, Editor, Page};
use crate::model::Host;

pub const HELP_URL: &str = "https://github.com/fikrifirmanf/ferrum-ssh#readme";

pub fn clipboard_event(action: &str, paste: Option<String>) -> Option<egui::Event> {
    match action {
        "copy" => Some(egui::Event::Copy),
        "cut" => Some(egui::Event::Cut),
        "paste" => paste.map(egui::Event::Paste),
        "select-all" => Some(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }),
        _ => None,
    }
}

impl App {
    pub fn menu_action(&mut self, action: &str, ctx: &egui::Context) {
        if matches!(action, "copy" | "cut" | "paste" | "select-all") {
            self.clipboard_actions.push(action.to_owned());
            ctx.request_repaint();
            return;
        }
        if self.locked && !matches!(action, "help" | "about" | "updates" | "quit") {
            return;
        }
        match action {
            "new-host" => {
                self.active_tab = None;
                self.page = Page::Hosts;
                self.editor = Some(Editor::Host(Box::new(Host::default()), true));
            }
            "local" => self.open_local(),
            "sftp" => self.open_sftp(None),
            "close" => {
                if let Some(id) = self.active_tab {
                    self.close_tab(id);
                }
            }
            "settings" => {
                self.active_tab = None;
                self.page = Page::Settings;
            }
            "hosts" => {
                self.active_tab = None;
                self.page = Page::Hosts;
            }
            "palette" => {
                self.palette.open = true;
            }
            "sidebar" => {
                self.vault.settings.sidebar_collapsed = !self.vault.settings.sidebar_collapsed;
                self.mark_dirty();
            }
            "help" => {
                if let Err(e) = open::that(HELP_URL) {
                    self.toast(format!("Could not open help: {e}"), true);
                }
            }
            "about" => self.dialog = Some(Dialog::About),
            "updates" => {
                self.dialog = Some(Dialog::Updates);
                self.updater.check(self.rt.handle(), ctx);
            }
            "quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => {}
        }
    }

    pub fn application_menus(&mut self, ui: &mut egui::Ui) {
        for action in std::mem::take(&mut self.menu_actions) {
            self.menu_action(&action, ui.ctx());
        }
        #[cfg(not(target_os = "macos"))]
        egui::Panel::top("application-menus")
            .show_separator_line(false)
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    for (title, items) in [
                        (
                            "File",
                            vec![
                                ("New host", "new-host"),
                                ("Local terminal", "local"),
                                ("SFTP browser", "sftp"),
                                ("Close tab", "close"),
                                ("Settings", "settings"),
                                ("Quit", "quit"),
                            ],
                        ),
                        (
                            "Edit",
                            vec![
                                ("Cut", "cut"),
                                ("Copy", "copy"),
                                ("Paste", "paste"),
                                ("Select all", "select-all"),
                            ],
                        ),
                        (
                            "View",
                            vec![
                                ("Hosts", "hosts"),
                                ("Toggle sidebar", "sidebar"),
                                ("Command palette", "palette"),
                            ],
                        ),
                        (
                            "Help",
                            vec![
                                ("Ferrum help", "help"),
                                ("Check for updates…", "updates"),
                                ("About Ferrum", "about"),
                            ],
                        ),
                    ] {
                        ui.menu_button(title, |ui| {
                            crate::ui::forms::menu_padding(ui);
                            for (label, action) in items {
                                if ui.button(label).clicked() {
                                    self.menu_action(action, ui.ctx());
                                    ui.close();
                                }
                            }
                        });
                    }
                });
            });
    }
}

#[cfg(target_os = "macos")]
pub fn native_menu() -> muda::Result<muda::Menu> {
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuItem, PredefinedMenuItem as Standard, Submenu};
    let item = |id: &str, label: &str, key: Option<Code>| {
        MenuItem::with_id(
            id,
            label,
            true,
            key.map(|key| Accelerator::new(Modifiers::META, key)),
        )
    };
    let app = Submenu::with_items(
        "Ferrum",
        true,
        &[
            &item("about", "About Ferrum", None),
            &item("updates", "Check for updates…", None),
            &Standard::separator(),
            &item("settings", "Settings…", Some(Code::Comma)),
            &Standard::separator(),
            &Standard::hide(None),
            &Standard::hide_others(None),
            &Standard::show_all(None),
            &Standard::separator(),
            &item("quit", "Quit Ferrum", Some(Code::KeyQ)),
        ],
    )?;
    let file = Submenu::with_items(
        "File",
        true,
        &[
            &item("new-host", "New host", Some(Code::KeyN)),
            &item("local", "Local terminal", Some(Code::KeyT)),
            &item("sftp", "SFTP browser", None),
            &Standard::separator(),
            &item("close", "Close tab", Some(Code::KeyW)),
        ],
    )?;
    // Explicit events work with egui's custom text widgets, unlike Cocoa's
    // responder-chain edit actions, which assume an NSTextView.
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &item("cut", "Cut", Some(Code::KeyX)),
            &item("copy", "Copy", Some(Code::KeyC)),
            &item("paste", "Paste", Some(Code::KeyV)),
            &item("select-all", "Select all", Some(Code::KeyA)),
        ],
    )?;
    let view = Submenu::with_items(
        "View",
        true,
        &[
            &item("hosts", "Hosts", None),
            &item("sidebar", "Toggle sidebar", None),
            &item("palette", "Command palette", Some(Code::KeyK)),
        ],
    )?;
    let help = Submenu::with_items(
        "Help",
        true,
        &[
            &item("help", "Ferrum help", None),
            &item("updates", "Check for updates…", None),
            &item("about", "About Ferrum", None),
        ],
    )?;
    let menu = Menu::with_items(&[&app, &file, &edit, &view, &help])?;
    menu.init_for_nsapp();
    help.set_as_help_menu_for_nsapp();
    Ok(menu)
}
