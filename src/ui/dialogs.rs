//! Modal prompts raised by connections (host keys, passwords, keyboard-interactive)
//! and the app-level dialogs (key generation, imports, serial, vault password...).

use egui::{Key, RichText};

use super::{danger_button, ghost_button, primary_button};
use crate::app::{App, Dialog, Tab};
use crate::conn::{PromptKind, PromptReply};
use crate::model::{FlowControl, Host, Parity, Protocol};
use crate::themes::Palette;

fn modal_frame(pal: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(pal.panel)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(20))
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(18.0).strong());
    ui.add_space(6.0);
}

fn field_label(ui: &mut egui::Ui, text: &str, pal: Palette) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).small().color(pal.text_dim));
}

fn enter_pressed(ui: &egui::Ui) -> bool {
    ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift)
}

/// What a dialog asked the app to do once the frame is drawn.
enum Outcome {
    Keep,
    Close,
}

impl App {
    pub fn draw_prompts(&mut self, ctx: &egui::Context) {
        let pal = self.pal();
        let Some(state) = self.prompts.front_mut() else {
            return;
        };
        let Some(req) = state.req.as_ref() else {
            self.prompts.pop_front();
            return;
        };
        let mut reply: Option<PromptReply> = None;
        #[allow(deprecated)]
        let id = egui::Id::new(("prompt", req.session));
        egui::Modal::new(id).frame(modal_frame(pal)).show(ctx, |ui| {
            ui.set_width(440.0);
            match &req.kind {
                PromptKind::HostKeyUnknown { host, port, key_type, fingerprint } => {
                    title(ui, "Unknown host");
                    ui.label(format!("The authenticity of {host}:{port} can't be established."));
                    ui.add_space(6.0);
                    field_label(ui, key_type, pal);
                    ui.label(RichText::new(fingerprint).monospace());
                    ui.add_space(8.0);
                    ui.checkbox(&mut state.save_host_key, "Remember this host in Known Hosts");
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_button("Continue", pal)).clicked() || enter_pressed(ui) {
                            reply = Some(PromptReply::Accept { save: state.save_host_key });
                        }
                        if ui.add(ghost_button("Cancel")).clicked() {
                            reply = Some(PromptReply::Cancel);
                        }
                    });
                }
                PromptKind::HostKeyChanged { host, port, key_type, fingerprint, old_fingerprint } => {
                    title(ui, "\u{E4E0} Host key changed");
                    ui.colored_label(
                        pal.danger,
                        format!("The key for {host}:{port} is different from the one you saved. Someone could be intercepting the connection, or the server was reinstalled."),
                    );
                    field_label(ui, "Saved fingerprint", pal);
                    ui.label(RichText::new(old_fingerprint).monospace());
                    field_label(ui, &format!("New fingerprint ({key_type})"), pal);
                    ui.label(RichText::new(fingerprint).monospace());
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(danger_button("Replace and connect", pal)).clicked() {
                            reply = Some(PromptReply::Accept { save: true });
                        }
                        if ui.add(ghost_button("Connect once")).clicked() {
                            reply = Some(PromptReply::Accept { save: false });
                        }
                        if ui.add(primary_button("Cancel", pal)).clicked() {
                            reply = Some(PromptReply::Cancel);
                        }
                    });
                }
                PromptKind::Fields { title: t, message, fields } => {
                    title(ui, t);
                    if !message.is_empty() {
                        ui.label(RichText::new(message).color(pal.text_dim));
                    }
                    let mut submit = false;
                    for (i, f) in fields.iter().enumerate() {
                        field_label(ui, &f.label, pal);
                        let r = if f.secret { super::password_input(ui, &mut state.values[i], "", f32::INFINITY) } else { ui.add(egui::TextEdit::singleline(&mut state.values[i]).desired_width(f32::INFINITY)) };
                        if i == 0 && !r.has_focus() && ui.memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if r.lost_focus() && enter_pressed(ui) {
                            submit = true;
                        }
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary_button("OK", pal)).clicked() || submit {
                            reply = Some(PromptReply::Values(state.values.clone()));
                        }
                        if ui.add(ghost_button("Cancel")).clicked() {
                            reply = Some(PromptReply::Cancel);
                        }
                    });
                }
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                reply = Some(PromptReply::Cancel);
            }
        });
        if let Some(r) = reply {
            if let Some(mut s) = self.prompts.pop_front() {
                if let Some(req) = s.req.take() {
                    let _ = req.reply.send(r);
                }
            }
        }
    }

    pub fn draw_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let pal = self.pal();
        let mut outcome = Outcome::Keep;
        let resp = egui::Modal::new(egui::Id::new("app-dialog"))
            .frame(modal_frame(pal))
            .show(ctx, |ui| {
                ui.set_width(460.0);
                outcome = self.dialog_body(ui, &mut dialog, pal);
            });
        if resp.should_close() {
            outcome = Outcome::Close;
        }
        if matches!(outcome, Outcome::Keep) && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    fn dialog_body(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog, pal: Palette) -> Outcome {
        let mut close = false;
        match dialog {
            Dialog::KeyGen {
                kind,
                label,
                passphrase,
                busy,
            } => {
                title(ui, "Generate SSH key");
                field_label(ui, "Label", pal);
                ui.add(egui::TextEdit::singleline(label).desired_width(f32::INFINITY));
                field_label(ui, "Type", pal);
                egui::ComboBox::from_id_salt("keygen-kind")
                    .width(260.0)
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        for k in crate::keys::KeyKind::ALL {
                            ui.selectable_value(kind, k, k.label());
                        }
                    });
                field_label(ui, "Passphrase (optional)", pal);
                super::password_input(ui, passphrase, "", f32::INFINITY);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !*busy && !label.trim().is_empty(),
                            primary_button("Generate", pal),
                        )
                        .clicked()
                    {
                        *busy = true;
                        match crate::keys::generate(*kind, label.trim(), passphrase) {
                            Ok(k) => {
                                self.toast(format!("Generated {}", k.label), false);
                                self.vault.keys.push(k);
                                self.mark_dirty();
                                close = true;
                            }
                            Err(e) => {
                                *busy = false;
                                self.toast(format!("Key generation failed: {e:#}"), true);
                            }
                        }
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::KeyImport {
                label,
                private,
                passphrase,
                error,
            } => {
                title(ui, "Import private key");
                field_label(ui, "Label", pal);
                ui.add(egui::TextEdit::singleline(label).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    field_label(ui, "Private key (OpenSSH or PEM)", pal);
                    if ui.small_button("Choose file…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_file() {
                            match std::fs::read_to_string(&p) {
                                Ok(t) => {
                                    *private = t;
                                    if label.is_empty() {
                                        *label = p
                                            .file_name()
                                            .map(|n| n.to_string_lossy().to_string())
                                            .unwrap_or_default();
                                    }
                                }
                                Err(e) => *error = Some(e.to_string()),
                            }
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(private)
                                .code_editor()
                                .desired_rows(6)
                                .desired_width(f32::INFINITY),
                        );
                    });
                field_label(ui, "Passphrase (if the key is encrypted)", pal);
                super::password_input(ui, passphrase, "", f32::INFINITY);
                if let Some(e) = error {
                    ui.colored_label(pal.danger, e.as_str());
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!private.trim().is_empty(), primary_button("Import", pal))
                        .clicked()
                    {
                        let name = if label.trim().is_empty() {
                            "imported-key"
                        } else {
                            label.trim()
                        };
                        match crate::keys::import(name, private, passphrase) {
                            Ok(k) => {
                                self.toast(format!("Imported {}", k.label), false);
                                self.vault.keys.push(k);
                                self.mark_dirty();
                                close = true;
                            }
                            Err(e) => *error = Some(format!("{e:#}")),
                        }
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Import { error } => {
                title(ui, "Import hosts");
                ui.label(
                    RichText::new(
                        "Bring hosts, groups, keys and tunnels over from another client.",
                    )
                    .color(pal.text_dim),
                );
                ui.add_space(8.0);
                let ssh_config =
                    directories::UserDirs::new().map(|u| u.home_dir().join(".ssh").join("config"));
                let mut picked: Option<(u8, String)> = None;
                if let Some(p) = ssh_config.as_ref().filter(|p| p.exists()) {
                    if ui
                        .add(primary_button("Import ~/.ssh/config", pal))
                        .clicked()
                    {
                        match std::fs::read_to_string(p) {
                            Ok(t) => picked = Some((0, t)),
                            Err(e) => *error = Some(e.to_string()),
                        }
                    }
                    ui.add_space(6.0);
                }
                let sources: [(&str, u8, &[&str]); 5] = [
                    ("OpenSSH config file…", 0, &[]),
                    ("PuTTY sessions (.reg export)…", 1, &["reg"]),
                    (
                        "MobaXterm sessions (.mxtsessions / .ini)…",
                        2,
                        &["mxtsessions", "ini"],
                    ),
                    ("SecureCRT sessions (.xml export)…", 3, &["xml"]),
                    ("CSV (label,address,port,username,…)…", 4, &["csv"]),
                ];
                for (name, which, exts) in sources {
                    if ui.add(ghost_button(name)).clicked() {
                        let mut fd = rfd::FileDialog::new();
                        if !exts.is_empty() {
                            fd = fd.add_filter(name, exts);
                        }
                        if let Some(p) = fd.pick_file() {
                            match std::fs::read(&p) {
                                Ok(bytes) => picked = Some((which, decode_text(&bytes))),
                                Err(e) => *error = Some(e.to_string()),
                            }
                        }
                    }
                }
                if let Some((which, text)) = picked {
                    let res = match which {
                        0 => crate::importers::parse_ssh_config(&text, &self.vault),
                        1 => crate::importers::parse_putty_reg(&text),
                        2 => crate::importers::parse_mobaxterm(&text),
                        3 => crate::importers::parse_securecrt_xml(&text),
                        _ => crate::importers::parse_csv(&text),
                    };
                    if res.hosts.is_empty() && res.keys.is_empty() && res.forwards.is_empty() {
                        *error = Some("Nothing to import was found in that file".into());
                    } else {
                        let summary = res.apply(&mut self.vault);
                        self.toast(summary, false);
                        self.mark_dirty();
                        close = true;
                    }
                }
                if let Some(e) = error {
                    ui.colored_label(pal.danger, e.as_str());
                }
                ui.add_space(12.0);
                if ui.add(ghost_button("Close")).clicked() {
                    close = true;
                }
            }
            Dialog::Serial { cfg, ports } => {
                title(ui, "Serial connection");
                field_label(ui, "Port", pal);
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("serial-port")
                        .width(300.0)
                        .selected_text(if cfg.device.is_empty() {
                            "Choose a port"
                        } else {
                            cfg.device.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for p in ports.iter() {
                                ui.selectable_value(&mut cfg.device, p.clone(), p);
                            }
                        });
                    if ui
                        .small_button("\u{E036}")
                        .on_hover_text("Rescan ports")
                        .clicked()
                    {
                        *ports = crate::conn::serial::list_ports();
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut cfg.device)
                        .hint_text("or type a path, e.g. /dev/ttyUSB0 or COM3")
                        .desired_width(f32::INFINITY),
                );
                egui::Grid::new("serial-grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Baud rate");
                        egui::ComboBox::from_id_salt("baud")
                            .selected_text(cfg.baud_rate.to_string())
                            .show_ui(ui, |ui| {
                                for b in [
                                    300, 1200, 2400, 4800, 9600, 19_200, 38_400, 57_600, 115_200,
                                    230_400, 460_800, 921_600,
                                ] {
                                    ui.selectable_value(&mut cfg.baud_rate, b, b.to_string());
                                }
                            });
                        ui.end_row();
                        ui.label("Data bits");
                        egui::ComboBox::from_id_salt("databits")
                            .selected_text(cfg.data_bits.to_string())
                            .show_ui(ui, |ui| {
                                for b in [5u8, 6, 7, 8] {
                                    ui.selectable_value(&mut cfg.data_bits, b, b.to_string());
                                }
                            });
                        ui.end_row();
                        ui.label("Stop bits");
                        egui::ComboBox::from_id_salt("stopbits")
                            .selected_text(cfg.stop_bits.to_string())
                            .show_ui(ui, |ui| {
                                for b in [1u8, 2] {
                                    ui.selectable_value(&mut cfg.stop_bits, b, b.to_string());
                                }
                            });
                        ui.end_row();
                        ui.label("Parity");
                        egui::ComboBox::from_id_salt("parity")
                            .selected_text(format!("{:?}", cfg.parity))
                            .show_ui(ui, |ui| {
                                for p in [Parity::None, Parity::Odd, Parity::Even] {
                                    ui.selectable_value(&mut cfg.parity, p, format!("{p:?}"));
                                }
                            });
                        ui.end_row();
                        ui.label("Flow control");
                        egui::ComboBox::from_id_salt("flow")
                            .selected_text(format!("{:?}", cfg.flow_control))
                            .show_ui(ui, |ui| {
                                for f in [
                                    FlowControl::None,
                                    FlowControl::Software,
                                    FlowControl::Hardware,
                                ] {
                                    ui.selectable_value(&mut cfg.flow_control, f, format!("{f:?}"));
                                }
                            });
                        ui.end_row();
                        ui.label("Line ending");
                        egui::ComboBox::from_id_salt("eol")
                            .selected_text(eol_label(&cfg.line_ending))
                            .show_ui(ui, |ui| {
                                for e in ["\r", "\n", "\r\n"] {
                                    ui.selectable_value(
                                        &mut cfg.line_ending,
                                        e.to_string(),
                                        eol_label(e),
                                    );
                                }
                            });
                        ui.end_row();
                    });
                ui.checkbox(&mut cfg.local_echo, "Local echo");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let ready = !cfg.device.trim().is_empty();
                    if ui
                        .add_enabled(ready, primary_button("Connect", pal))
                        .clicked()
                    {
                        let host = Host {
                            label: cfg.device.clone(),
                            address: cfg.device.clone(),
                            protocol: Protocol::Serial,
                            serial: cfg.clone(),
                            ..Default::default()
                        };
                        let mut t = self.vault.resolve(&host);
                        t.host_id = None;
                        self.open_target(t);
                        close = true;
                    }
                    if ui
                        .add_enabled(ready, ghost_button("Save as host"))
                        .clicked()
                    {
                        let host = Host {
                            label: cfg.device.clone(),
                            address: cfg.device.clone(),
                            protocol: Protocol::Serial,
                            serial: cfg.clone(),
                            ..Default::default()
                        };
                        self.vault.hosts.push(host);
                        self.mark_dirty();
                        self.toast("Saved serial host", false);
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::InstallKey {
                key_id,
                host_id,
                busy,
            } => {
                title(ui, "Install public key on a host");
                let key_label = self
                    .vault
                    .key(*key_id)
                    .map(|k| k.label.clone())
                    .unwrap_or_default();
                ui.label(
                    RichText::new(format!(
                        "Appends “{key_label}” to ~/.ssh/authorized_keys on the server."
                    ))
                    .color(pal.text_dim),
                );
                field_label(ui, "Host", pal);
                let current = host_id
                    .and_then(|h| self.vault.host(h))
                    .map(|h| h.display_name())
                    .unwrap_or_else(|| "Choose a host".into());
                egui::ComboBox::from_id_salt("install-host")
                    .width(320.0)
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for h in self
                            .vault
                            .hosts
                            .iter()
                            .filter(|h| h.protocol == Protocol::Ssh)
                        {
                            ui.selectable_value(host_id, Some(h.id), h.display_name());
                        }
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!*busy && host_id.is_some(), primary_button("Install", pal))
                        .clicked()
                    {
                        *busy = true;
                        self.install_key_on_host(*key_id, host_id.unwrap_or_default());
                        close = true;
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::MasterPassword {
                current,
                new1,
                new2,
                error,
            } => {
                let has = self.store.master_password.is_some();
                title(
                    ui,
                    if has {
                        "Change master password"
                    } else {
                        "Set master password"
                    },
                );
                ui.label(RichText::new("The vault is encrypted on disk with this password (Argon2id + ChaCha20-Poly1305). It can't be recovered if you forget it.").color(pal.text_dim));
                if has {
                    field_label(ui, "Current password", pal);
                    super::password_input(ui, current, "", f32::INFINITY);
                }
                field_label(ui, "New password (leave empty to remove)", pal);
                super::password_input(ui, new1, "", f32::INFINITY);
                field_label(ui, "Repeat new password", pal);
                super::password_input(ui, new2, "", f32::INFINITY);
                if let Some(e) = error {
                    ui.colored_label(pal.danger, e.as_str());
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(primary_button("Save", pal)).clicked() {
                        if has && self.store.master_password.as_deref() != Some(current.as_str()) {
                            *error = Some("Current password is wrong".into());
                        } else if new1 != new2 {
                            *error = Some("Passwords don't match".into());
                        } else {
                            let previous = self.store.master_password.take();
                            self.store.master_password = if new1.is_empty() {
                                None
                            } else {
                                Some(new1.clone())
                            };
                            match self.store.save(&self.vault) {
                                Ok(()) => {
                                    if self.biometric_enrolled {
                                        // Keep the saved copy in step, or drop it with the password.
                                        let synced = if new1.is_empty() {
                                            crate::biometric::forget()
                                        } else {
                                            crate::biometric::enroll(new1)
                                        };
                                        if synced.is_err() || new1.is_empty() {
                                            let _ = crate::biometric::forget();
                                            self.biometric_enrolled = false;
                                        }
                                    }
                                    self.toast(
                                        if new1.is_empty() {
                                            "Master password removed"
                                        } else {
                                            "Master password set"
                                        },
                                        false,
                                    );
                                    close = true;
                                }
                                Err(e) => {
                                    self.store.master_password = previous;
                                    *error = Some(format!("{e:#}"));
                                }
                            }
                        }
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ExportVault { passphrase, error } => {
                title(ui, "Export encrypted vault");
                ui.label(RichText::new("Creates a backup file with every host, key, snippet and setting, encrypted with this passphrase.").color(pal.text_dim));
                field_label(ui, "Passphrase", pal);
                super::password_input(ui, passphrase, "", f32::INFINITY);
                if let Some(e) = error {
                    ui.colored_label(pal.danger, e.as_str());
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(passphrase.len() >= 4, primary_button("Export…", pal))
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_file_name("ferrum-vault.ferrum")
                            .save_file()
                        {
                            match crate::store::export_encrypted(&self.vault, &p, passphrase) {
                                Ok(()) => {
                                    self.toast("Vault exported", false);
                                    close = true;
                                }
                                Err(e) => *error = Some(format!("{e:#}")),
                            }
                        }
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ImportVault { passphrase, error } => {
                title(ui, "Import encrypted vault");
                ui.label(
                    RichText::new("Merges a Ferrum backup into this vault. Newer items win.")
                        .color(pal.text_dim),
                );
                field_label(ui, "Passphrase", pal);
                super::password_input(ui, passphrase, "", f32::INFINITY);
                if let Some(e) = error {
                    ui.colored_label(pal.danger, e.as_str());
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(primary_button("Choose file…", pal)).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_file() {
                            match crate::store::import_encrypted(&p, passphrase) {
                                Ok(remote) => {
                                    crate::store::merge_vaults(&mut self.vault, &remote);
                                    self.mark_dirty();
                                    self.toast("Vault imported", false);
                                    close = true;
                                }
                                Err(e) => *error = Some(format!("{e:#}")),
                            }
                        }
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::ConfirmDelete { ids, what } => {
                title(ui, "Delete?");
                ui.label(format!(
                    "{what} will be removed from the vault. This can't be undone."
                ));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(danger_button("Delete", pal)).clicked() {
                        for id in ids.iter() {
                            self.vault.delete(*id);
                            self.selected_hosts.remove(id);
                        }
                        self.mark_dirty();
                        close = true;
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::RunSnippet {
                snippet,
                values,
                targets,
            } => {
                title(ui, &format!("Run “{}”", snippet.name));
                for (name, value) in values.iter_mut() {
                    field_label(ui, name, pal);
                    ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                }
                field_label(ui, "Run in", pal);
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for (sid, on) in targets.iter_mut() {
                            let name = self
                                .session_title(*sid)
                                .unwrap_or_else(|| format!("Session {sid}"));
                            ui.checkbox(on, name);
                        }
                    });
                let mut preview = snippet.script.clone();
                for (name, value) in values.iter() {
                    preview = preview.replace(&format!("{{{{{name}}}}}"), value);
                }
                field_label(ui, "Command", pal);
                ui.label(RichText::new(&preview).monospace().color(pal.text_dim));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let chosen: Vec<u64> = targets.iter().filter(|t| t.1).map(|t| t.0).collect();
                    if ui
                        .add_enabled(!chosen.is_empty(), primary_button("Run", pal))
                        .clicked()
                    {
                        self.run_snippet_now(&preview, &chosen);
                        close = true;
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::SaveWorkspace { name } => {
                title(ui, "Save workspace");
                ui.label(
                    RichText::new("Remembers every open tab, its split layout and hosts so you can reopen them together.")
                        .color(pal.text_dim),
                );
                field_label(ui, "Name", pal);
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                r.request_focus();
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let go = ui.add(primary_button("Save", pal)).clicked()
                        || (r.lost_focus() && enter_pressed(ui));
                    if go && self.save_workspace(name) {
                        close = true;
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::RenameTab { tab, title: t } => {
                title(ui, "Rename tab");
                let r = ui.add(egui::TextEdit::singleline(t).desired_width(f32::INFINITY));
                r.request_focus();
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(primary_button("Rename", pal)).clicked()
                        || (r.lost_focus() && enter_pressed(ui))
                    {
                        let name = t.trim().to_string();
                        for tb in self.tabs.iter_mut() {
                            match tb {
                                Tab::Terminal(tt) if tt.id == *tab => {
                                    tt.custom_title = (!name.is_empty()).then(|| name.clone())
                                }
                                Tab::Sftp(st) if st.id == *tab && !name.is_empty() => {
                                    st.title = name.clone()
                                }
                                _ => {}
                            }
                        }
                        close = true;
                    }
                    if ui.add(ghost_button("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Updates => {
                close = self.update_dialog(ui);
            }
            Dialog::About => {
                ui.horizontal(|ui| {
                    super::logo_mark(ui, 52.0, pal.accent);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Ferrum").size(28.0).strong());
                        ui.label(
                            RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                                .color(pal.text_dim),
                        );
                    });
                });
                ui.add_space(16.0);
                ui.label(RichText::new("Open-source SSH client").size(18.0).strong());
                ui.label("Connect to servers, transfer files, and work in your terminal across macOS, Windows, and Linux.");
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Free to use, study, modify, and share under the MIT license.")
                        .color(pal.text_dim),
                );
                ui.label(
                    RichText::new(
                        "Created by Fikri Firman Fadilah. Built with Rust, egui, and russh.",
                    )
                    .color(pal.text_dim),
                );
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to("View source", "https://github.com/fikrifirmanf/ferrum-ssh");
                    ui.hyperlink_to(
                        "MIT license",
                        "https://github.com/fikrifirmanf/ferrum-ssh/blob/main/LICENSE",
                    );
                    ui.hyperlink_to(
                        "Credits",
                        "https://github.com/fikrifirmanf/ferrum-ssh#credits",
                    );
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.hyperlink_to("Help", crate::ui::menus::HELP_URL);
                    ui.hyperlink_to("Releases", crate::updates::RELEASES_URL);
                });
                ui.add_space(12.0);
                if ui.add(primary_button("Close", pal)).clicked() {
                    close = true;
                }
            }
        }
        if close { Outcome::Close } else { Outcome::Keep }
    }

    fn session_title(&self, sid: u64) -> Option<String> {
        self.tabs.iter().find_map(|t| match t {
            Tab::Terminal(tt) => tt
                .panes
                .iter()
                .find(|p| p.session.id == sid)
                .map(|p| p.session.title.clone()),
            _ => None,
        })
    }

    fn install_key_on_host(&mut self, key_id: uuid::Uuid, host_id: uuid::Uuid) {
        let (Some(key), Some(host)) = (
            self.vault.key(key_id).cloned(),
            self.vault.host(host_id).cloned(),
        ) else {
            return;
        };
        let target = self.vault.resolve(&host);
        let ctx = self.conn_template().headless();
        let bridge = self.bridge.clone();
        let name = host.display_name();
        self.rt.spawn(async move {
            let res = async {
                let client = crate::conn::ssh::connect_headless(target, ctx).await?;
                crate::conn::ssh::install_public_key(&client, &key.public_key).await?;
                client.disconnect().await;
                anyhow::Ok(())
            }
            .await;
            match res {
                Ok(()) => bridge.toast(format!("Installed {} on {name}", key.label), false),
                Err(e) => bridge.toast(format!("Installing key failed: {e:#}"), true),
            }
        });
    }
}

fn eol_label(e: &str) -> &'static str {
    match e {
        "\n" => "LF",
        "\r\n" => "CR LF",
        _ => "CR",
    }
}

/// Import files are often UTF-16 (PuTTY .reg exports) or Latin-1.
fn decode_text(bytes: &[u8]) -> String {
    if let Some((enc, _)) = encoding_rs::Encoding::for_bom(bytes) {
        return enc.decode(bytes).0.into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}
