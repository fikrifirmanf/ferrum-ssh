//! Settings page.

use egui::{Color32, CornerRadius, RichText, Stroke, TextEdit, Vec2};

use super::{card_frame, primary_button};
use crate::app::{App, Dialog};
use crate::model::*;

const SECTIONS: [&str; 9] = [
    "Appearance",
    "Terminal",
    "Connections",
    "SFTP",
    "Security",
    "Sync & Backup",
    "Integrations",
    "AI assistant",
    "Updates",
];

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let before = self.vault.settings.clone();
        ui.label(RichText::new("Settings").size(22.0).strong());
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for (i, s) in SECTIONS.iter().enumerate() {
                if ui
                    .selectable_label(self.settings_section == i as u8, *s)
                    .clicked()
                {
                    self.settings_section = i as u8;
                }
            }
        });
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.set_max_width(760.0);
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
                ui.add_space(20.0);
                ui.label(
                    RichText::new(format!(
                        "Ferrum {} · vault at {}",
                        env!("CARGO_PKG_VERSION"),
                        self.store.path.display()
                    ))
                    .small()
                    .color(pal.text_dim),
                );
            });
        if before != self.vault.settings {
            crate::store::touch_settings(&mut self.vault);
            self.mark_dirty();
        }
    }

    fn settings_updates(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        card_frame(pal).show(ui, |ui| {
            ui.label(RichText::new("Software updates").strong());
            if ui.checkbox(&mut self.vault.settings.check_updates_on_startup,
                "Check for updates on startup").changed()
                && !self.vault.settings.check_updates_on_startup {
                self.updater.dismiss_notice();
            }
            ui.label(RichText::new("A new release shows a notice you can close. Downloads and installation always wait for you.").color(pal.text_dim));
            ui.add_space(8.0);
            if ui.button("Check for updates").clicked() {
                self.dialog = Some(Dialog::Updates);
                self.updater.check(self.rt.handle(), ui.ctx());
            }
        });
    }

    fn settings_appearance(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Interface theme").strong());
            ui.horizontal(|ui| {
                ui.selectable_value(&mut s.ui_theme, UiTheme::Dark, "Dark");
                ui.selectable_value(&mut s.ui_theme, UiTheme::Midnight, "Midnight (OLED)");
                ui.selectable_value(&mut s.ui_theme, UiTheme::Light, "Light");
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Accent color").strong());
            ui.horizontal(|ui| {
                for c in [
                    [99u8, 132, 255],
                    [76, 211, 138],
                    [245, 166, 35],
                    [232, 93, 117],
                    [160, 120, 255],
                    [60, 196, 214],
                    [240, 120, 70],
                ] {
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::click());
                    ui.painter().circle_filled(
                        rect.center(),
                        10.0,
                        Color32::from_rgb(c[0], c[1], c[2]),
                    );
                    if s.accent == c {
                        ui.painter()
                            .circle_stroke(rect.center(), 12.0, Stroke::new(2.0, pal.text));
                    }
                    if resp.clicked() {
                        s.accent = c;
                    }
                }
                let mut col = Color32::from_rgb(s.accent[0], s.accent[1], s.accent[2]);
                if ui.color_edit_button_srgba(&mut col).changed() {
                    s.accent = [col.r(), col.g(), col.b()];
                }
            });
        });
        ui.add_space(10.0);
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Terminal color scheme").strong());
            ui.horizontal_wrapped(|ui| {
                for sc in crate::themes::schemes() {
                    let selected = s.terminal_scheme == sc.name;
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(150.0, 74.0), egui::Sense::click());
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(10),
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
                        egui::FontId::proportional(11.0),
                        sc.fg.gamma_multiply(0.8),
                    );
                    if resp.clicked() {
                        s.terminal_scheme = sc.name.to_string();
                    }
                }
            });
        });
        ui.add_space(10.0);
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("font-grid")
                .num_columns(2)
                .spacing([16.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Font size");
                    ui.add(egui::Slider::new(&mut s.font_size, 8.0..=32.0).step_by(0.5));
                    ui.end_row();
                    ui.label("Line height");
                    ui.add(egui::Slider::new(&mut s.line_height, 1.0..=1.8).step_by(0.05));
                    ui.end_row();
                    ui.label("Custom font file");
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut s.custom_font_path)
                                .hint_text("e.g. JetBrainsMonoNerdFont-Regular.ttf")
                                .desired_width(320.0),
                        );
                        if ui.button("Browse…").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Fonts", &["ttf", "otf"])
                                .pick_file()
                            {
                                s.custom_font_path = p.to_string_lossy().to_string();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Cursor");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut s.cursor_shape, CursorShape::Block, "█ Block");
                        ui.selectable_value(&mut s.cursor_shape, CursorShape::Beam, "▏Beam");
                        ui.selectable_value(
                            &mut s.cursor_shape,
                            CursorShape::Underline,
                            "▁ Underline",
                        );
                        ui.checkbox(&mut s.cursor_blink, "Blink");
                    });
                    ui.end_row();
                });
        });
    }

    fn settings_terminal(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(&mut s.copy_on_select, "Copy on select");
            ui.checkbox(
                &mut s.right_click_paste,
                "Right click pastes (Shift+right click for menu)",
            );
            ui.checkbox(&mut s.option_is_meta, "Use Option / Alt as Meta key");
            ui.checkbox(&mut s.brighten_bold, "Show bold text in bright colors");
            ui.checkbox(&mut s.bell_sound, "Bell requests attention");
            ui.checkbox(
                &mut s.autocomplete,
                "Suggest commands from history, snippets and common commands (press → to accept)",
            );
            ui.add_space(6.0);
            egui::Grid::new("term-grid")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Scrollback lines");
                    ui.add(
                        egui::DragValue::new(&mut s.scrollback)
                            .range(1000..=200_000)
                            .speed(100),
                    );
                    ui.end_row();
                    ui.label("TERM");
                    ui.add(TextEdit::singleline(&mut s.term_type).desired_width(160.0));
                    ui.end_row();
                    ui.label("Local shell");
                    ui.horizontal(|ui| {
                        ui.add(
                            TextEdit::singleline(&mut s.local_shell)
                                .hint_text("system default")
                                .desired_width(200.0),
                        );
                        egui::ComboBox::from_id_salt("shells")
                            .selected_text("Pick")
                            .show_ui(ui, |ui| {
                                for sh in crate::conn::local::available_shells() {
                                    if ui.selectable_label(s.local_shell == sh, &sh).clicked() {
                                        s.local_shell = sh;
                                    }
                                }
                            });
                    });
                    ui.end_row();
                    ui.label("mosh-client path");
                    ui.add(TextEdit::singleline(&mut s.mosh_client_path).desired_width(200.0));
                    ui.end_row();
                });
        });
        ui.add_space(10.0);
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut s.keyword_highlighting,
                    RichText::new("Keyword highlighting").strong(),
                );
                if ui.small_button("Reset defaults").clicked() {
                    s.keyword_rules = default_keyword_rules();
                }
            });
            let mut remove = None;
            for (i, r) in s.keyword_rules.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut r.enabled, "");
                    let mut c = Color32::from_rgb(r.color[0], r.color[1], r.color[2]);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        r.color = [c.r(), c.g(), c.b()];
                    }
                    ui.add(TextEdit::singleline(&mut r.name).desired_width(90.0));
                    let ok = regex::Regex::new(&r.pattern).is_ok();
                    ui.add(
                        TextEdit::singleline(&mut r.pattern)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(380.0)
                            .text_color(if ok { pal.text } else { pal.danger }),
                    );
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                s.keyword_rules.remove(i);
            }
            if ui.small_button("+ Add rule").clicked() {
                s.keyword_rules.push(KeywordRule {
                    name: "Custom".into(),
                    pattern: r"\bTODO\b".into(),
                    color: [255, 160, 0],
                    enabled: true,
                });
            }
        });
        if !self.vault.settings.keyword_highlighting {
            self.keyword_cache.1.clear();
            self.keyword_cache.0.clear();
        }
    }

    fn settings_connections(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(
                &mut s.auto_reconnect,
                "Automatically reconnect dropped sessions",
            );
            ui.checkbox(
                &mut s.detect_os,
                "Detect host operating system after connecting",
            );
            ui.checkbox(
                &mut s.save_quick_connect,
                "Save quick-connect targets as hosts",
            );
            ui.checkbox(
                &mut s.confirm_close_tab,
                "Confirm before closing an active session",
            );
            ui.horizontal(|ui| {
                ui.label("Default keep-alive interval (seconds, 0 = off)");
                ui.add(egui::DragValue::new(&mut s.default_keepalive_secs).range(0..=600));
            });
            ui.horizontal(|ui| {
                ui.label("Session log folder");
                ui.add(
                    TextEdit::singleline(&mut s.log_dir)
                        .hint_text(crate::store::default_log_dir().to_string_lossy())
                        .desired_width(300.0),
                );
            });
        });
    }

    fn settings_sftp(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(&mut s.sftp_show_hidden, "Show hidden files");
            ui.checkbox(
                &mut s.sftp_confirm_overwrite,
                "Confirm before overwriting files",
            );
            ui.horizontal(|ui| {
                ui.label("Open remote files with");
                ui.add(
                    TextEdit::singleline(&mut s.external_editor)
                        .hint_text("system default (e.g. code -w)")
                        .desired_width(260.0),
                );
            });
            ui.add_space(6.0);
            ui.label(RichText::new("File type associations").strong());
            let mut remove = None;
            for (i, (ext, cmd)) in s.file_associations.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(ext)
                            .hint_text("json")
                            .desired_width(70.0),
                    );
                    ui.label("→");
                    ui.add(
                        TextEdit::singleline(cmd)
                            .hint_text("code -w")
                            .desired_width(220.0),
                    );
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                s.file_associations.remove(i);
            }
            if ui.small_button("+ Add association").clicked() {
                s.file_associations.push((String::new(), String::new()));
            }
            ui.label(
                RichText::new(
                    "Edited files are uploaded back automatically every time you save them.",
                )
                .small()
                .color(pal.text_dim),
            );
        });
    }

    fn settings_security(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Vault encryption").strong());
            let has = self.store.master_password.is_some();
            ui.label(
                RichText::new(if has {
                    "Your vault is encrypted with XChaCha20-Poly1305 using a key derived from your master password (Argon2id)."
                } else {
                    "Your vault is encrypted with a random device key stored next to it. Add a master password for stronger protection."
                })
                .color(pal.text_dim),
            );
            ui.horizontal(|ui| {
                if ui.add(primary_button(if has { "Change master password" } else { "Set master password" }, pal)).clicked() {
                    self.dialog = Some(Dialog::MasterPassword { current: String::new(), new1: String::new(), new2: String::new(), error: None });
                }
                if has && ui.button("Lock now").clicked() {
                    self.locked = true;
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Auto-lock after (minutes, 0 = never)");
                ui.add(egui::DragValue::new(&mut self.vault.settings.lock_after_minutes).range(0..=600));
            });
        });
    }

    fn settings_sync(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("End-to-end encrypted sync").strong());
            ui.label(
                RichText::new(
                    "Point Ferrum at a folder synced by iCloud Drive, Dropbox, OneDrive, Syncthing or a network share. \
                     The vault is merged entity-by-entity (newest edit wins) and stored encrypted with your sync passphrase — \
                     no Ferrum account or server involved.",
                )
                .color(pal.text_dim),
            );
            ui.horizontal(|ui| {
                ui.label("Folder");
                ui.add(TextEdit::singleline(&mut self.vault.settings.sync_folder).desired_width(340.0));
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.vault.settings.sync_folder = p.to_string_lossy().to_string();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Passphrase");
                super::password_input(ui, &mut self.sync_passphrase, "same on every device", 240.0);
            });
            ui.checkbox(&mut self.vault.settings.auto_sync, "Sync automatically after changes (passphrase is kept in memory only)");
            ui.horizontal(|ui| {
                let busy = self.sync_busy;
                if ui.add_enabled(!busy, primary_button(if busy { "Syncing…" } else { "Sync now" }, pal)).clicked() {
                    self.start_sync();
                }
                ui.label(RichText::new(&self.sync_status).small().color(pal.text_dim));
            });
        });
        ui.add_space(10.0);
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Backup").strong());
            ui.horizontal(|ui| {
                if ui.button("Export encrypted vault…").clicked() {
                    self.dialog = Some(Dialog::ExportVault {
                        passphrase: String::new(),
                        error: None,
                    });
                }
                if ui.button("Restore from backup…").clicked() {
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
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("DigitalOcean").strong());
            super::password_input(ui, &mut s.do_token, "Personal access token (read)", f32::INFINITY);
            ui.add_space(8.0);
            ui.label(RichText::new("AWS EC2").strong());
            ui.add(TextEdit::singleline(&mut s.aws_access_key).hint_text("Access key ID").desired_width(f32::INFINITY));
            super::password_input(ui, &mut s.aws_secret_key, "Secret access key", f32::INFINITY);
            ui.add(TextEdit::singleline(&mut s.aws_regions).hint_text("Regions, comma separated").desired_width(f32::INFINITY));
            ui.add_space(8.0);
            ui.label(RichText::new("Microsoft Azure (service principal)").strong());
            ui.add(TextEdit::singleline(&mut s.azure_tenant).hint_text("Tenant ID").desired_width(f32::INFINITY));
            ui.add(TextEdit::singleline(&mut s.azure_client_id).hint_text("Client ID").desired_width(f32::INFINITY));
            super::password_input(ui, &mut s.azure_client_secret, "Client secret", f32::INFINITY);
            ui.add(TextEdit::singleline(&mut s.azure_subscription).hint_text("Subscription ID").desired_width(f32::INFINITY));
            ui.label(RichText::new("Import from Hosts → \u{E1FE} → Import from cloud. Credentials stay in your encrypted vault.").small().color(pal.text_dim));
        });
    }

    fn settings_ai(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let s = &mut self.vault.settings;
        card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("AI command assistant").strong());
            ui.label(RichText::new("Describe what you want in plain language inside a terminal (\u{E6A2} button) and get a ready-to-run command. Nothing runs until you insert it.").color(pal.text_dim));
            ui.horizontal(|ui| {
                if ui.selectable_value(&mut s.ai_provider, AiProvider::Anthropic, "Anthropic").clicked() {
                    s.ai_endpoint = "https://api.anthropic.com/v1/messages".into();
                    s.ai_model = "claude-sonnet-5-5".into();
                }
                if ui.selectable_value(&mut s.ai_provider, AiProvider::OpenAiCompatible, "OpenAI-compatible").clicked() {
                    s.ai_endpoint = "https://api.openai.com/v1/chat/completions".into();
                }
            });
            egui::Grid::new("ai-grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label("Endpoint");
                ui.add(TextEdit::singleline(&mut s.ai_endpoint).desired_width(380.0));
                ui.end_row();
                ui.label("Model");
                ui.add(TextEdit::singleline(&mut s.ai_model).desired_width(220.0));
                ui.end_row();
                ui.label("API key");
                super::password_input(ui, &mut s.ai_api_key, "", 380.0);
                ui.end_row();
            });
        });
    }
}
