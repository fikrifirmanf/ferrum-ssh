//! Settings page.

use egui::{Color32, CornerRadius, RichText, Stroke, Vec2};

use super::forms;
use crate::app::{App, Dialog};
use crate::model::*;

const SECTIONS: [&str; 9] = [
    "Appearance",
    "Terminal",
    "Connections",
    "SFTP",
    "Security",
    "Sync and backup",
    "Integrations",
    "AI assistant",
    "Updates",
];

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui) {
        let before = self.vault.settings.clone();
        forms::heading(ui, "Settings", crate::themes::tokens::TITLE);
        ui.add_space(12.0);
        if ui.available_width() < 700.0 {
            let width = ui.available_width();
            let shown = SECTIONS[self.settings_section.min(8) as usize];
            let section = &mut self.settings_section;
            forms::select(ui, "settings-section", width, shown, false, |ui| {
                for (index, label) in SECTIONS.iter().enumerate() {
                    forms::select_value(ui, section, index as u8, label);
                }
            });
            ui.add_space(16.0);
            self.settings_content(ui);
        } else {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(176.0, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (index, label) in SECTIONS.iter().enumerate() {
                            let active = self.settings_section == index as u8;
                            if section_item(ui, label, active).clicked() {
                                self.settings_section = index as u8;
                            }
                        }
                    },
                );
                ui.add_space(24.0);
                ui.vertical(|ui| self.settings_content(ui));
            });
        }
        if before != self.vault.settings {
            crate::store::touch_settings(&mut self.vault);
            self.mark_dirty();
        }
    }

    fn settings_content(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let enter = super::motion::since_change(
            ui.ctx(),
            egui::Id::unique("settings-section-enter"),
            self.settings_section,
            super::motion::PANEL,
        );
        ui.set_opacity(enter);
        egui::ScrollArea::vertical()
            .id_salt("settings-content")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(720.0));
                ui.spacing_mut().item_spacing.y = 12.0;
                match self.settings_section {
                    0 => self.settings_appearance(ui),
                    1 => self.settings_terminal(ui),
                    2 => self.settings_connections(ui),
                    3 => self.settings_sftp(ui),
                    4 => self.settings_security(ui),
                    5 => self.settings_sync(ui),
                    6 => self.settings_integrations(ui),
                    7 => self.settings_ai(ui),
                    _ => self.settings_updates(ui),
                }
                ui.add_space(12.0);
                forms::help(ui, &format!("Ferrum {}", env!("CARGO_PKG_VERSION")), pal);
                forms::help(ui, &self.store.path.display().to_string(), pal);
            });
    }

    fn settings_updates(&mut self, ui: &mut egui::Ui) {
        forms::card(ui, |ui| {
            forms::heading(ui, "Software updates", 14.0);
            if forms::switch_row(
                ui,
                "Check for updates on startup",
                "A new release shows a notice you can close. Downloads and installation always wait for you.",
                &mut self.vault.settings.check_updates_on_startup,
            )
            .changed()
                && !self.vault.settings.check_updates_on_startup
            {
                self.updater.dismiss_notice();
            }
            if ui.add(forms::Btn::secondary("Check for updates")).clicked() {
                self.dialog = Some(Dialog::Updates);
                self.updater.check(self.rt.handle(), ui.ctx());
            }
        });
    }

    fn settings_appearance(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        forms::card(ui, |ui| {
            forms::heading(ui, "Interface theme", 14.0);
            forms::help(
                ui,
                "Terminal colors stay independent of the interface theme.",
                pal,
            );
            ui.horizontal_wrapped(|ui| {
                let width = ((ui.available_width() - 24.0) / 3.0).clamp(100.0, 200.0);
                for (theme, label) in [
                    (UiTheme::Dark, "Smoke"),
                    (UiTheme::Midnight, "Midnight"),
                    (UiTheme::Light, "Frost"),
                ] {
                    let colors = crate::themes::palette(theme, s.accent);
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(width, 100.0), egui::Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::Role::RadioButton,
                            true,
                            s.ui_theme == theme,
                            label,
                        )
                    });
                    let picked = super::motion::ease(
                        ui.ctx(),
                        response.id.with("picked"),
                        s.ui_theme == theme,
                        super::motion::CONTROL,
                    );
                    ui.painter().rect(
                        rect,
                        8,
                        pal.bg,
                        Stroke::new(
                            if s.ui_theme == theme || response.has_focus() {
                                2.0
                            } else {
                                1.0
                            },
                            pal.control_border.lerp_to_gamma(pal.accent, picked),
                        ),
                        egui::StrokeKind::Inside,
                    );
                    let preview = rect.shrink2(Vec2::new(8.0, 8.0));
                    let preview = egui::Rect::from_min_max(
                        preview.min,
                        egui::pos2(preview.right(), preview.bottom() - 24.0),
                    );
                    ui.painter().rect_filled(preview, 4, colors.bg);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(
                            preview.min,
                            Vec2::new(preview.width() * 0.24, preview.height()),
                        ),
                        4,
                        colors.sidebar,
                    );
                    ui.painter().hline(
                        (preview.left() + preview.width() * 0.32)..=(preview.right() - 8.0),
                        preview.top() + 16.0,
                        Stroke::new(2.0, colors.text_dim),
                    );
                    ui.painter().hline(
                        (preview.left() + preview.width() * 0.32)..=(preview.right() - 16.0),
                        preview.top() + 28.0,
                        Stroke::new(2.0, colors.accent),
                    );
                    ui.painter().text(
                        rect.left_bottom() + Vec2::new(8.0, -16.0),
                        egui::Align2::LEFT_CENTER,
                        label,
                        egui::FontId::proportional(12.0),
                        pal.text,
                    );
                    if response.clicked() {
                        s.ui_theme = theme;
                    }
                }
            });
            forms::heading(ui, "Accent color", 14.0);
            ui.horizontal(|ui| {
                for (c, name) in [
                    (crate::themes::DEFAULT_ACCENT, "Ember"),
                    (crate::themes::COBALT_ACCENT, "Cobalt"),
                    (crate::themes::PATINA_ACCENT, "Patina"),
                    ([245, 166, 35], "Amber"),
                    ([232, 93, 117], "Rose"),
                    ([160, 120, 255], "Violet"),
                    ([76, 211, 138], "Green"),
                ] {
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::Role::RadioButton,
                            true,
                            s.accent == c,
                            name,
                        )
                    });
                    ui.painter().circle_filled(
                        rect.center(),
                        10.0,
                        Color32::from_rgb(c[0], c[1], c[2]),
                    );
                    if s.accent == c {
                        ui.painter()
                            .circle_stroke(rect.center(), 13.0, Stroke::new(2.0, pal.text));
                    }
                    if resp.on_hover_text(name).clicked() {
                        s.accent = c;
                    }
                }
                let mut col = Color32::from_rgb(s.accent[0], s.accent[1], s.accent[2]);
                if ui.color_edit_button_srgba(&mut col).changed() {
                    s.accent = [col.r(), col.g(), col.b()];
                }
            });
            forms::heading(ui, "Glass", 14.0);
            forms::help(
                ui,
                "Sets how clear the buttons, tabs, menus and other glass controls are. Tinted is calmer and easier to read.",
                pal,
            );
            forms::slider(ui, &mut s.glass_level, "Glass level", "Clear", "Tinted");
            forms::switch_row(
                ui,
                "Reduce transparency",
                "Draws glass controls, menus and dialogs solid. Your system setting is also respected.",
                &mut s.reduce_transparency,
            );
            forms::switch_row(
                ui,
                "Reduce motion",
                "Removes travel and fades. Your system setting is also respected.",
                &mut s.reduce_motion,
            );
        });
        forms::card(ui, |ui| {
            forms::heading(ui, "Library layout", 14.0);
            forms::label(ui, "Host rows", pal);
            forms::segmented(
                ui,
                "density",
                &mut s.hosts_compact,
                &[(false, "Comfortable"), (true, "Compact")],
            );
            forms::switch_row(ui, "Use grid view", "", &mut s.hosts_grid_view);
        });
        forms::card(ui, |ui| {
            forms::heading(ui, "Terminal color scheme", 14.0);
            ui.horizontal_wrapped(|ui| {
                for sc in crate::themes::schemes() {
                    let selected = s.terminal_scheme == sc.name;
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(150.0, 74.0), egui::Sense::click());
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, sc.name)
                    });
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(8),
                        sc.bg,
                        Stroke::new(
                            if selected { 2.0 } else { 1.0 },
                            if selected { pal.accent } else { pal.border },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        rect.min + Vec2::new(10.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        "$ ls -la",
                        egui::FontId::monospace(12.0),
                        sc.fg,
                    );
                    for (i, c) in sc.ansi[1..7].iter().enumerate() {
                        ui.painter().circle_filled(
                            rect.min + Vec2::new(16.0 + i as f32 * 16.0, 38.0),
                            5.0,
                            *c,
                        );
                    }
                    ui.painter().text(
                        rect.min + Vec2::new(10.0, 52.0),
                        egui::Align2::LEFT_TOP,
                        sc.name,
                        egui::FontId::proportional(12.0),
                        sc.fg,
                    );
                    if resp.clicked() {
                        s.terminal_scheme = sc.name.to_string();
                    }
                }
            });
        });
        forms::card(ui, |ui| {
            forms::heading(ui, "Terminal text", 14.0);
            ui.horizontal(|ui| {
                ui.label("Font size");
                ui.add(egui::Slider::new(&mut s.font_size, 8.0..=32.0).step_by(0.5));
            });
            ui.horizontal(|ui| {
                ui.label("Line height");
                ui.add(egui::Slider::new(&mut s.line_height, 1.0..=1.8).step_by(0.05));
            });
            ui.horizontal_top(|ui| {
                forms::Input::new("Custom font file")
                    .hint("JetBrainsMonoNerdFont-Regular.ttf")
                    .mono()
                    .width((ui.available_width() - 96.0).max(160.0))
                    .show(ui, &mut s.custom_font_path);
                ui.vertical(|ui| {
                    ui.add_space(22.0);
                    if ui.add(forms::Btn::secondary("Browse…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("Fonts", &["ttf", "otf"])
                            .pick_file()
                        {
                            s.custom_font_path = p.to_string_lossy().to_string();
                        }
                    }
                });
            });
            forms::label(ui, "Cursor", pal);
            forms::segmented(
                ui,
                "cursor",
                &mut s.cursor_shape,
                &[
                    (CursorShape::Block, "Block"),
                    (CursorShape::Beam, "Beam"),
                    (CursorShape::Underline, "Underline"),
                ],
            );
            forms::switch_row(ui, "Blinking cursor", "", &mut s.cursor_blink);
        });
    }

    fn settings_terminal(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.vault.settings;
        forms::card(ui, |ui| {
            forms::heading(ui, "Behavior", 14.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            forms::switch_row(ui, "Copy on select", "", &mut s.copy_on_select);
            forms::switch_row(
                ui,
                "Right click pastes",
                "Shift and right click opens the menu.",
                &mut s.right_click_paste,
            );
            forms::switch_row(
                ui,
                "Use Option or Alt as Meta key",
                "",
                &mut s.option_is_meta,
            );
            forms::switch_row(
                ui,
                "Show bold text in bright colors",
                "",
                &mut s.brighten_bold,
            );
            forms::switch_row(ui, "Bell requests attention", "", &mut s.bell_sound);
            forms::switch_row(
                ui,
                "Suggest commands",
                "From history, snippets and common commands. Press → to accept.",
                &mut s.autocomplete,
            );
        });
        forms::card(ui, |ui| {
            forms::heading(ui, "Session", 14.0);
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::number(ui, forms::Input::new("Scrollback lines"), &mut s.scrollback);
                } else {
                    forms::Input::new("TERM").mono().show(ui, &mut s.term_type);
                }
            });
            s.scrollback = s.scrollback.clamp(1000, 200_000);
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    ui.horizontal_top(|ui| {
                        forms::Input::new("Local shell")
                            .hint("System default")
                            .mono()
                            .width((ui.available_width() - 44.0).max(80.0))
                            .show(ui, &mut s.local_shell);
                        ui.vertical(|ui| {
                            ui.add_space(22.0);
                            forms::select(ui, "shells", 36.0, "", false, |ui| {
                                ui.set_min_width(200.0);
                                for sh in crate::conn::local::available_shells() {
                                    if forms::option(ui, s.local_shell == sh, &sh).clicked() {
                                        s.local_shell = sh;
                                        ui.close();
                                    }
                                }
                            })
                            .on_hover_text("Pick an installed shell");
                        });
                    });
                } else {
                    forms::Input::new("mosh-client path")
                        .mono()
                        .show(ui, &mut s.mosh_client_path);
                }
            });
        });
        forms::card(ui, |ui| {
            forms::switch_row(
                ui,
                "Keyword highlighting",
                "Colors matching words in terminal output.",
                &mut s.keyword_highlighting,
            );
            let mut remove = None;
            for (i, r) in s.keyword_rules.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        forms::switch(ui, &mut r.enabled, "Rule enabled");
                        let mut c = Color32::from_rgb(r.color[0], r.color[1], r.color[2]);
                        if ui.color_edit_button_srgba(&mut c).changed() {
                            r.color = [c.r(), c.g(), c.b()];
                        }
                        forms::Input::bare("Rule name")
                            .width(100.0)
                            .show(ui, &mut r.name);
                        let ok = regex::Regex::new(&r.pattern).is_ok();
                        forms::Input::bare("Pattern")
                            .mono()
                            .invalid(!ok)
                            .width((ui.available_width() - 40.0).max(80.0))
                            .show(ui, &mut r.pattern);
                        if ui
                            .add(forms::Btn::icon(crate::icons::X, "Remove rule"))
                            .clicked()
                        {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                s.keyword_rules.remove(i);
            }
            ui.horizontal(|ui| {
                if ui
                    .add(forms::Btn::ghost(format!("{}  Add rule", crate::icons::PLUS)).small())
                    .clicked()
                {
                    s.keyword_rules.push(KeywordRule {
                        name: "Custom".into(),
                        pattern: r"\bTODO\b".into(),
                        color: [255, 160, 0],
                        enabled: true,
                    });
                }
                if ui
                    .add(forms::Btn::ghost("Reset defaults").small())
                    .clicked()
                {
                    s.keyword_rules = default_keyword_rules();
                }
            });
        });
        if !self.vault.settings.keyword_highlighting {
            self.keyword_cache.1.clear();
            self.keyword_cache.0.clear();
        }
    }

    fn settings_connections(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.vault.settings;
        forms::card(ui, |ui| {
            forms::heading(ui, "Connections", 14.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            forms::switch_row(
                ui,
                "Automatically reconnect dropped sessions",
                "",
                &mut s.auto_reconnect,
            );
            forms::switch_row(
                ui,
                "Detect host operating system after connecting",
                "",
                &mut s.detect_os,
            );
            forms::switch_row(
                ui,
                "Save quick-connect targets as hosts",
                "",
                &mut s.save_quick_connect,
            );
            forms::switch_row(
                ui,
                "Confirm before closing an active session",
                "",
                &mut s.confirm_close_tab,
            );
            forms::switch_row(
                ui,
                "Show whether hosts are reachable",
                "Opens a TCP connection to each saved host's port once a minute while the vault is open.",
                &mut s.show_host_status,
            );
        });
        forms::card(ui, |ui| {
            forms::number(
                ui,
                forms::Input::new("Default keep-alive")
                    .suffix("sec")
                    .help("0 turns it off.")
                    .width(200.0),
                &mut s.default_keepalive_secs,
            );
            s.default_keepalive_secs = s.default_keepalive_secs.min(600);
            forms::Input::new("Session log folder")
                .hint(&crate::store::default_log_dir().to_string_lossy())
                .mono()
                .show(ui, &mut s.log_dir);
        });
    }

    fn settings_sftp(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        forms::card(ui, |ui| {
            forms::heading(ui, "File browser", 14.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            forms::switch_row(ui, "Show hidden files", "", &mut s.sftp_show_hidden);
            forms::switch_row(
                ui,
                "Confirm before overwriting files",
                "",
                &mut s.sftp_confirm_overwrite,
            );
        });
        forms::card(ui, |ui| {
            forms::Input::new("Open remote files with")
                .hint("System default, or a command like code -w")
                .mono()
                .show(ui, &mut s.external_editor);
            forms::label(ui, "File type associations", pal);
            let mut remove = None;
            for (i, (ext, cmd)) in s.file_associations.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        forms::Input::bare("Extension")
                            .hint("json")
                            .mono()
                            .width(90.0)
                            .show(ui, ext);
                        ui.label(RichText::new("→").color(pal.faint));
                        forms::Input::bare("Command")
                            .hint("code -w")
                            .mono()
                            .width((ui.available_width() - 40.0).max(80.0))
                            .show(ui, cmd);
                        if ui
                            .add(forms::Btn::icon(crate::icons::X, "Remove association"))
                            .clicked()
                        {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                s.file_associations.remove(i);
            }
            if ui
                .add(forms::Btn::ghost(format!("{}  Add association", crate::icons::PLUS)).small())
                .clicked()
            {
                s.file_associations.push((String::new(), String::new()));
            }
            forms::help(
                ui,
                "Edited files are uploaded back automatically every time you save them.",
                pal,
            );
        });
    }

    fn settings_security(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        forms::card(ui, |ui| {
            forms::heading(ui, "Vault encryption", 14.0);
            let has = self.store.master_password.is_some();
            ui.add(
                egui::Label::new(
                    RichText::new(if has {
                        "Your vault is encrypted with XChaCha20-Poly1305 using a key derived from your master password (Argon2id)."
                    } else {
                        "Your vault is encrypted with a random device key stored next to it. Add a master password for stronger protection."
                    })
                    .color(pal.text_dim),
                )
                .wrap(),
            );
            ui.horizontal(|ui| {
                if ui
                    .add(forms::Btn::primary(if has {
                        "Change master password"
                    } else {
                        "Set master password"
                    }))
                    .clicked()
                {
                    self.dialog = Some(Dialog::MasterPassword {
                        current: String::new(),
                        new1: String::new(),
                        new2: String::new(),
                        error: None,
                    });
                }
                if has && ui.add(forms::Btn::secondary("Lock now")).clicked() {
                    self.locked = true;
                }
            });
            if has && crate::biometric::SUPPORTED {
                self.settings_biometric(ui);
            }
            forms::number(
                ui,
                forms::Input::new("Auto-lock after")
                    .suffix("min")
                    .help("0 means never.")
                    .width(200.0),
                &mut self.vault.settings.lock_after_minutes,
            );
            self.vault.settings.lock_after_minutes =
                self.vault.settings.lock_after_minutes.min(600);
        });
    }

    fn settings_biometric(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let name = crate::biometric::NAME;
        ui.add_space(6.0);
        if self.biometric_enrolled {
            ui.horizontal(|ui| {
                ui.label(format!("{name} unlock is on."));
                if ui.add(forms::Btn::secondary("Turn off").small()).clicked() {
                    match crate::biometric::forget() {
                        Ok(()) => {
                            self.biometric_enrolled = false;
                            self.toast(&format!("{name} unlock turned off"), false);
                        }
                        Err(e) => self.toast(&format!("{e:#}"), true),
                    }
                }
            });
        } else if crate::biometric::available() {
            ui.horizontal(|ui| {
                if ui
                    .add(forms::Btn::secondary(format!("Unlock with {name}")).small())
                    .clicked()
                {
                    self.enable_biometric();
                }
                ui.label(
                    RichText::new("Saves your master password in the macOS Keychain.")
                        .small()
                        .color(pal.text_dim),
                );
            });
        } else {
            ui.label(
                RichText::new(format!("{name} is not set up on this Mac."))
                    .small()
                    .color(pal.text_dim),
            );
        }
    }

    fn enable_biometric(&mut self) {
        let Some(password) = self.store.master_password.clone() else {
            return;
        };
        let result = crate::biometric::authenticate("turn on Touch ID unlock for Ferrum")
            .and_then(|()| crate::biometric::enroll(&password));
        match result {
            Ok(()) => {
                self.biometric_enrolled = true;
                self.toast(
                    &format!("{} unlock turned on", crate::biometric::NAME),
                    false,
                );
            }
            Err(e) if e.to_string() == "cancelled" => {}
            Err(e) => self.toast(&format!("{e:#}"), true),
        }
    }

    fn settings_sync(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        forms::card(ui, |ui| {
            forms::heading(ui, "End-to-end encrypted sync", 14.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Point Ferrum at a folder synced by iCloud Drive, Dropbox, OneDrive, Syncthing or a network share. \
                         The vault is merged item by item, newest edit wins, and stored encrypted with your sync passphrase. \
                         No Ferrum account or server is involved.",
                    )
                    .color(pal.text_dim),
                )
                .wrap(),
            );
            ui.horizontal_top(|ui| {
                forms::Input::new("Folder")
                    .mono()
                    .width((ui.available_width() - 96.0).max(160.0))
                    .show(ui, &mut self.vault.settings.sync_folder);
                ui.vertical(|ui| {
                    ui.add_space(22.0);
                    if ui.add(forms::Btn::secondary("Browse…")).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.vault.settings.sync_folder = p.to_string_lossy().to_string();
                        }
                    }
                });
            });
            forms::Input::new("Passphrase")
                .hint("Same on every device")
                .password()
                .show(ui, &mut self.sync_passphrase);
            forms::switch_row(
                ui,
                "Sync automatically after changes",
                "The passphrase is kept in memory only.",
                &mut self.vault.settings.auto_sync,
            );
            ui.horizontal(|ui| {
                let busy = self.sync_busy;
                if ui
                    .add_enabled(
                        !busy,
                        forms::Btn::primary(if busy { "Syncing…" } else { "Sync now" }),
                    )
                    .clicked()
                {
                    self.start_sync();
                }
                forms::help(ui, &self.sync_status, pal);
            });
        });
        forms::card(ui, |ui| {
            forms::heading(ui, "Backup", 14.0);
            ui.horizontal(|ui| {
                if ui
                    .add(forms::Btn::secondary("Export encrypted vault…"))
                    .clicked()
                {
                    self.dialog = Some(Dialog::ExportVault {
                        passphrase: String::new(),
                        error: None,
                    });
                }
                if ui
                    .add(forms::Btn::secondary("Restore from backup…"))
                    .clicked()
                {
                    self.dialog = Some(Dialog::ImportVault {
                        passphrase: String::new(),
                        error: None,
                    });
                }
            });
        });
    }

    fn settings_integrations(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        forms::card(ui, |ui| {
            forms::heading(ui, "DigitalOcean", 14.0);
            forms::Input::new("Personal access token")
                .hint("Read access is enough")
                .password()
                .show(ui, &mut s.do_token);
        });
        forms::card(ui, |ui| {
            forms::heading(ui, "AWS EC2", 14.0);
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new("Access key ID")
                        .mono()
                        .show(ui, &mut s.aws_access_key);
                } else {
                    forms::Input::new("Secret access key")
                        .password()
                        .show(ui, &mut s.aws_secret_key);
                }
            });
            forms::Input::new("Regions")
                .hint("Comma separated")
                .mono()
                .show(ui, &mut s.aws_regions);
        });
        forms::card(ui, |ui| {
            forms::heading(ui, "Microsoft Azure service principal", 14.0);
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new("Tenant ID")
                        .mono()
                        .show(ui, &mut s.azure_tenant);
                } else {
                    forms::Input::new("Client ID")
                        .mono()
                        .show(ui, &mut s.azure_client_id);
                }
            });
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new("Client secret")
                        .password()
                        .show(ui, &mut s.azure_client_secret);
                } else {
                    forms::Input::new("Subscription ID")
                        .mono()
                        .show(ui, &mut s.azure_subscription);
                }
            });
        });
        forms::help(
            ui,
            "Import from the Hosts menu, under Import from cloud. Credentials stay in your encrypted vault.",
            pal,
        );
    }

    fn settings_ai(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        forms::card(ui, |ui| {
            forms::heading(ui, "AI command assistant", 14.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Describe what you want in plain language inside a terminal and get a ready-to-run command. Nothing runs until you insert it.")
                        .color(pal.text_dim),
                )
                .wrap(),
            );
            forms::label(ui, "Provider", pal);
            let mut provider = s.ai_provider;
            if forms::segmented(
                ui,
                "ai-provider",
                &mut provider,
                &[
                    (AiProvider::Anthropic, "Anthropic"),
                    (AiProvider::OpenAiCompatible, "OpenAI-compatible"),
                ],
            ) {
                s.ai_provider = provider;
                match provider {
                    AiProvider::Anthropic => {
                        s.ai_endpoint = "https://api.anthropic.com/v1/messages".into();
                        s.ai_model = "claude-sonnet-5-5".into();
                    }
                    AiProvider::OpenAiCompatible => {
                        s.ai_endpoint = "https://api.openai.com/v1/chat/completions".into();
                    }
                }
            }
            forms::Input::new("Endpoint")
                .mono()
                .show(ui, &mut s.ai_endpoint);
            forms::Input::new("Model").mono().show(ui, &mut s.ai_model);
            forms::Input::new("API key")
                .password()
                .show(ui, &mut s.ai_api_key);
        });
    }
}

fn section_item(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let pal = crate::themes::current(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(176.0, 34.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, label));
    let hover = super::motion::ease(
        ui.ctx(),
        resp.id.with("h"),
        resp.hovered() && !active,
        super::motion::FAST,
    );
    let on = super::motion::ease(ui.ctx(), resp.id.with("on"), active, super::motion::FAST);
    ui.painter()
        .rect_filled(rect, 8, pal.card.gamma_multiply(hover));
    ui.painter()
        .rect_filled(rect, 8, pal.card_hover.gamma_multiply(on));
    if on > 0.0 {
        let half = 7.0 * on;
        ui.painter().vline(
            rect.left() + 1.0,
            (rect.center().y - half)..=(rect.center().y + half),
            Stroke::new(2.0, pal.accent),
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(12.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        pal.text_dim.lerp_to_gamma(pal.text, hover.max(on)),
    );
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            8,
            Stroke::new(2.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    resp
}
