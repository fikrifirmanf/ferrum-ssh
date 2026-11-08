//! Keychain, port forwarding, snippets, known hosts, and history/logs pages.

use egui::RichText;
use uuid::Uuid;

use super::{card_frame, forms, hash_color, section_title};
use crate::app::{App, Dialog, Editor};
use crate::model::*;

fn page_header(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    pal: crate::themes::Palette,
    right: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            forms::heading(ui, title, crate::themes::tokens::TITLE);
            ui.label(RichText::new(subtitle).size(13.0).color(pal.text_dim));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
    ui.add_space(16.0);
}

fn row_frame(pal: crate::themes::Palette) -> egui::Frame {
    super::card_frame(pal)
        .corner_radius(egui::CornerRadius::same(
            crate::themes::tokens::RADIUS_INPUT,
        ))
        .inner_margin(egui::Margin::symmetric(12, 10))
}

impl App {
    pub fn keychain_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let mut open: Option<Editor> = None;
        page_header(
            ui,
            "Keychain",
            "SSH keys, certificates, and reusable identities.",
            pal,
            |ui| {
                ui.menu_button("Import \u{E136}", |ui| {
                    crate::ui::forms::menu_padding(ui);
                    if ui.button("Paste / import key file…").clicked() {
                        self.dialog = Some(Dialog::KeyImport {
                            label: String::new(),
                            private: String::new(),
                            passphrase: String::new(),
                            error: None,
                        });
                        ui.close();
                    }
                    let local = crate::keys::discover_local_keys();
                    if !local.is_empty() {
                        ui.separator();
                        ui.label(RichText::new("From ~/.ssh").small().weak());
                        for (name, text) in local {
                            if ui.button(&name).clicked() {
                                match crate::keys::import(&name, &text, "") {
                                    Ok(k) => {
                                        open = Some(Editor::Key(k, true));
                                    }
                                    Err(_) => {
                                        self.dialog = Some(Dialog::KeyImport {
                                            label: name.clone(),
                                            private: text.clone(),
                                            passphrase: String::new(),
                                            error: Some(
                                                "This key is encrypted — enter its passphrase"
                                                    .into(),
                                            ),
                                        });
                                    }
                                }
                                ui.close();
                            }
                        }
                    }
                });
                if ui
                    .add(forms::Btn::secondary(format!(
                        "{}  Identity",
                        crate::icons::PLUS
                    )))
                    .clicked()
                {
                    open = Some(Editor::Identity(Identity::default(), true));
                }
                if ui
                    .add(forms::Btn::primary(format!(
                        "{}  Generate key",
                        crate::icons::PLUS
                    )))
                    .clicked()
                {
                    self.dialog = Some(Dialog::KeyGen {
                        kind: crate::keys::KeyKind::Ed25519,
                        label: "my-key".into(),
                        passphrase: String::new(),
                        busy: false,
                    });
                }
            },
        );
        forms::Input::bare("Filter keys and identities")
            .hint("Filter keys and identities")
            .icon(crate::icons::MAGNIFYING_GLASS)
            .width(320.0)
            .show(ui, &mut self.key_search);
        ui.add_space(8.0);
        let q = self.key_search.to_lowercase();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            section_title(ui, &format!("Keys · {}", self.vault.keys.len()), pal);
            if self.vault.keys.is_empty() {
                ui.label(RichText::new("No keys yet. Generate an ED25519 key or import one from ~/.ssh.").color(pal.text_dim));
            }
            ui.vertical(|ui| {
                for k in self.vault.keys.iter().filter(|k| q.is_empty() || k.label.to_lowercase().contains(&q)) {
                    let resp = row_frame(pal)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                super::os_avatar(ui, crate::icons::KEY, pal.text_dim, 34.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&k.label).family(crate::fonts::semibold()));
                                    let mut sub = k.key_type.clone();
                                    if !k.certificate.trim().is_empty() {
                                        sub.push_str(" · certificate");
                                    }
                                    if !k.passphrase.is_empty() || k.private_key.contains("ENCRYPTED") || k.key_type == "encrypted" {
                                        sub.push_str(" · passphrase");
                                    }
                                    ui.label(RichText::new(sub).small().color(pal.text_dim));
                                });
                            });
                            let used = self.vault.hosts.iter().filter(|h| h.settings.key_id == Some(k.id)).count();
                            ui.label(RichText::new(format!("Used by {used} hosts")).small().color(pal.text_dim));
                        })
                        .response
                        .interact(egui::Sense::click());
                    if resp.clicked() {
                        open = Some(Editor::Key(k.clone(), false));
                    }
                    resp.context_menu(|ui| {
                        crate::ui::forms::menu_padding(ui);
                        if ui.button("Copy public key").clicked() {
                            ui.ctx().copy_text(k.public_key.clone());
                            ui.close();
                        }
                        if ui.button("Install on host…").clicked() {
                            self.dialog = Some(Dialog::InstallKey { key_id: k.id, host_id: None, busy: false });
                            ui.close();
                        }
                    });
                }
            });
            ui.add_space(14.0);
            section_title(ui, &format!("Identities · {}", self.vault.identities.len()), pal);
            if self.vault.identities.is_empty() {
                ui.label(RichText::new("Identities bundle a username with a password and/or key so many hosts can share them.").color(pal.text_dim));
            }
            for i in self.vault.identities.iter().filter(|i| q.is_empty() || i.label.to_lowercase().contains(&q)) {
                let r = row_frame(pal)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            super::avatar(ui, &i.username, hash_color(&i.username), 28.0);
                            ui.label(RichText::new(&i.label).family(crate::fonts::semibold()));
                            ui.label(RichText::new(&i.username).color(pal.text_dim));
                            if i.key_id.is_some() {
                                forms::chip(ui, "key", false, pal);
                            }
                            if !i.password.is_empty() {
                                forms::chip(ui, "password", false, pal);
                            }
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if r.clicked() {
                    open = Some(Editor::Identity(i.clone(), false));
                }
            }
        });
        if open.is_some() {
            self.editor = open;
        }
    }

    pub fn forwarding_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let mut open: Option<Editor> = None;
        let mut toggle: Option<(PortForward, bool)> = None;
        page_header(
            ui,
            "Port forwarding",
            "Local, remote and dynamic (SOCKS) tunnels over SSH.",
            pal,
            |ui| {
                if ui
                    .add(forms::Btn::primary(format!(
                        "{}  New rule",
                        crate::icons::PLUS
                    )))
                    .clicked()
                {
                    open = Some(Editor::Forward(PortForward::default(), true));
                }
            },
        );
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if self.vault.forwards.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("\u{E0A0}").size(40.0).color(pal.text_dim));
                    forms::heading(ui, "No port forwarding rules", 16.0);
                    ui.label(RichText::new("Example: forward localhost:5432 to a database only reachable from your bastion.").color(pal.text_dim));
                });
            }
            for f in &self.vault.forwards {
                let running = self.forwards.is_running(f.id);
                let err = self.forward_status.get(&f.id).and_then(|s| s.1.clone());
                let host = f.host_id.and_then(|h| self.vault.host(h)).map(|h| h.display_name()).unwrap_or("(no host)".into());
                row_frame(pal).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let dot = if running { pal.success } else if err.is_some() { pal.danger } else { pal.text_dim };
                        forms::dot(ui, dot);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(if f.label.is_empty() { f.kind.label() } else { &f.label }).family(crate::fonts::semibold()));
                            let desc = match f.kind {
                                ForwardKind::Local => format!("{}:{}  →  {host}  →  {}:{}", f.bind_address, f.bind_port, f.dest_host, f.dest_port),
                                ForwardKind::Remote => format!("{host} {}:{}  →  {}:{}", f.bind_address, f.bind_port, f.dest_host, f.dest_port),
                                ForwardKind::Dynamic => format!("SOCKS {}:{}  →  {host}", f.bind_address, f.bind_port),
                            };
                            ui.label(RichText::new(desc).monospace().small().color(pal.text_dim));
                            if let Some(e) = &err {
                                ui.label(RichText::new(e).small().color(pal.danger));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(forms::Btn::icon(crate::icons::PENCIL_SIMPLE, "Edit rule")).on_hover_text("Edit").clicked() {
                                open = Some(Editor::Forward(f.clone(), false));
                            }
                            let label = if running { "\u{E46C} Stop" } else { "\u{E3D0} Start" };
                            if ui.add(forms::Btn::secondary(label).small()).clicked() {
                                toggle = Some((f.clone(), !running));
                            }
                            if f.auto_start {
                                forms::chip(ui, "auto", false, pal);
                            }
                            forms::chip(ui, f.kind.label(), true, pal);
                        });
                    });
                });
            }
        });
        if let Some((f, start)) = toggle {
            if start {
                self.start_forward(f);
            } else {
                let b = self.bridge.clone();
                self.forwards.stop(f.id, &b);
            }
        }
        if open.is_some() {
            self.editor = open;
        }
    }

    pub fn snippets_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let mut open: Option<Editor> = None;
        let mut run: Option<Snippet> = None;
        page_header(
            ui,
            "Snippets",
            "Reusable commands and scripts. Run them in one or many terminals at once.",
            pal,
            |ui| {
                if ui
                    .add(forms::Btn::primary(format!(
                        "{}  New snippet",
                        crate::icons::PLUS
                    )))
                    .clicked()
                {
                    open = Some(Editor::Snippet(Snippet::default(), true));
                }
            },
        );
        forms::Input::bare("Filter snippets")
            .hint("Filter snippets")
            .icon(crate::icons::MAGNIFYING_GLASS)
            .width(320.0)
            .show(ui, &mut self.snippet_search);
        ui.add_space(8.0);
        let q = self.snippet_search.to_lowercase();
        let mut packages: Vec<String> = self
            .vault
            .snippets
            .iter()
            .map(|s| s.package.trim().to_string())
            .collect();
        packages.sort_by_key(|p| (p.is_empty(), p.to_lowercase()));
        packages.dedup();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for pkg in &packages {
                    let items: Vec<&Snippet> = self
                        .vault
                        .snippets
                        .iter()
                        .filter(|s| s.package.trim() == pkg)
                        .filter(|s| {
                            q.is_empty()
                                || s.name.to_lowercase().contains(&q)
                                || s.script.to_lowercase().contains(&q)
                                || s.package.to_lowercase().contains(&q)
                                || s.tags.iter().any(|t| t.to_lowercase().contains(&q))
                        })
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    if packages.len() > 1 || !pkg.is_empty() {
                        let name = if pkg.is_empty() {
                            "No package"
                        } else {
                            pkg.as_str()
                        };
                        section_title(ui, &format!("\u{E390}  {name} · {}", items.len()), pal);
                    }
                    for s in items {
                        snippet_card(ui, s, pal, &mut open, &mut run);
                    }
                    ui.add_space(8.0);
                }
            });
        if let Some(s) = run {
            self.begin_run_snippet(s);
        }
        if open.is_some() {
            self.editor = open;
        }
    }

    pub fn known_hosts_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let mut delete: Option<Uuid> = None;
        page_header(
            ui,
            "Known hosts",
            "Server host keys you have trusted. Ferrum warns you when a key changes.",
            pal,
            |ui| {
                if ui
                    .add(forms::Btn::secondary("Import ~/.ssh/known_hosts"))
                    .clicked()
                {
                    let n = self.import_known_hosts();
                    self.toast(format!("Imported {n} host keys"), false);
                }
            },
        );
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if self.vault.known_hosts.is_empty() {
                    ui.label(
                        RichText::new("Trusted server keys appear here after you connect.")
                            .color(pal.text_dim),
                    );
                }
                egui::Grid::new("known-hosts")
                    .num_columns(4)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        forms::label(ui, "Host", pal);
                        forms::label(ui, "Key type", pal);
                        forms::label(ui, "Fingerprint", pal);
                        ui.label("");
                        ui.end_row();
                        for k in &self.vault.known_hosts {
                            ui.label(if k.port == 22 {
                                k.host.clone()
                            } else {
                                format!("[{}]:{}", k.host, k.port)
                            });
                            ui.label(&k.key_type);
                            ui.label(RichText::new(&k.fingerprint).monospace().small());
                            if ui
                                .add(forms::Btn::icon(
                                    crate::icons::TRASH,
                                    "Forget this host key",
                                ))
                                .on_hover_text("Forget")
                                .clicked()
                            {
                                delete = Some(k.id);
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(id) = delete {
            self.vault.delete(id);
            self.mark_dirty();
        }
    }

    pub fn import_known_hosts(&mut self) -> usize {
        let Some(path) =
            directories::UserDirs::new().map(|u| u.home_dir().join(".ssh/known_hosts"))
        else {
            return 0;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return 0;
        };
        let mut n = 0;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with('|')
                || line.starts_with('@')
            {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(hosts), Some(kt), Some(b64)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let public = format!("{kt} {b64}");
            let fp = crate::keys::fingerprint_of_public(&public);
            for h in hosts.split(',') {
                let (host, port) = match h.strip_prefix('[').and_then(|r| r.split_once("]:")) {
                    Some((h, p)) => (h.to_string(), p.parse().unwrap_or(22)),
                    None => (h.to_string(), 22),
                };
                if self
                    .vault
                    .known_hosts
                    .iter()
                    .any(|k| k.host == host && k.port == port && k.fingerprint == fp)
                {
                    continue;
                }
                self.vault.known_hosts.push(KnownHost {
                    id: Uuid::new_v4(),
                    host,
                    port,
                    key_type: kt.to_string(),
                    public_key: public.clone(),
                    fingerprint: fp.clone(),
                    added: now_ts(),
                    updated: now_ts(),
                });
                n += 1;
            }
        }
        self.mark_dirty();
        n
    }

    pub fn history_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let mut save_snippet: Option<String> = None;
        let mut clear_history = false;
        page_header(
            ui,
            "History and logs",
            "Commands you've run and every connection, with session recordings.",
            pal,
            |ui| {
                let mut on = self.vault.settings.session_logging;
                if forms::check(ui, &mut on, "Record session logs").changed() {
                    self.vault.settings.session_logging = on;
                    crate::store::touch_settings(&mut self.vault);
                    self.mark_dirty();
                }
            },
        );
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            ui.horizontal(|ui| {
                section_title(ui, "Command history", pal);
                if ui.add(forms::Btn::ghost("Clear").small()).clicked() {
                    clear_history = true;
                }
            });
            forms::Input::bare("Filter commands")
                .hint("Filter commands")
                .icon(crate::icons::MAGNIFYING_GLASS)
                .show(ui, &mut self.history_filter);
            let q = self.history_filter.to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("hist")
                .auto_shrink(false)
                .show(ui, |ui| {
                    for h in self
                        .vault
                        .history
                        .iter()
                        .rev()
                        .filter(|h| q.is_empty() || h.command.to_lowercase().contains(&q))
                        .take(500)
                    {
                        row_frame(pal).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&h.command).monospace());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(forms::Btn::icon(
                                                crate::icons::TERMINAL,
                                                "Save as snippet",
                                            ))
                                            .on_hover_text("Save as snippet")
                                            .clicked()
                                        {
                                            save_snippet = Some(h.command.clone());
                                        }
                                        if ui
                                            .add(forms::Btn::icon(
                                                crate::icons::COPY,
                                                "Copy command",
                                            ))
                                            .on_hover_text("Copy")
                                            .clicked()
                                        {
                                            ui.ctx().copy_text(h.command.clone());
                                        }
                                        ui.label(
                                            RichText::new(&h.host_label)
                                                .small()
                                                .color(pal.text_dim),
                                        );
                                    },
                                );
                            });
                        });
                    }
                });
            let ui = &mut cols[1];
            section_title(ui, "Connections", pal);
            egui::ScrollArea::vertical()
                .id_salt("logs")
                .auto_shrink(false)
                .show(ui, |ui| {
                    for l in self.vault.connection_logs.iter().rev().take(300) {
                        row_frame(pal).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{}  ·  {}",
                                            l.host_label,
                                            l.protocol.label()
                                        ))
                                        .family(crate::fonts::semibold()),
                                    );
                                    let start = chrono::DateTime::from_timestamp_millis(l.started)
                                        .map(|d| {
                                            d.with_timezone(&chrono::Local)
                                                .format("%Y-%m-%d %H:%M")
                                                .to_string()
                                        })
                                        .unwrap_or_default();
                                    let dur = l
                                        .ended
                                        .map(|e| format!(" · {}s", (e - l.started) / 1000))
                                        .unwrap_or_default();
                                    ui.label(
                                        RichText::new(format!("{start}{dur} · {}", l.status))
                                            .small()
                                            .color(pal.text_dim),
                                    );
                                });
                                if let Some(f) = &l.log_file {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(forms::Btn::secondary("Open log").small())
                                                .clicked()
                                            {
                                                let _ = open::that_detached(f);
                                            }
                                        },
                                    );
                                }
                            });
                        });
                    }
                });
        });
        if clear_history {
            self.vault.history.clear();
            self.mark_dirty();
        }
        if let Some(cmd) = save_snippet {
            let name: String = cmd.chars().take(40).collect();
            self.editor = Some(Editor::Snippet(
                Snippet {
                    name,
                    script: cmd,
                    ..Default::default()
                },
                true,
            ));
        }
    }
}

fn snippet_card(
    ui: &mut egui::Ui,
    s: &Snippet,
    pal: crate::themes::Palette,
    open: &mut Option<Editor>,
    run: &mut Option<Snippet>,
) {
    card_frame(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&s.name).family(crate::fonts::semibold()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(forms::Btn::icon(
                            crate::icons::PENCIL_SIMPLE,
                            "Edit snippet",
                        ))
                        .on_hover_text("Edit snippet")
                        .clicked()
                    {
                        *open = Some(Editor::Snippet(s.clone(), false));
                    }
                    if ui
                        .add(forms::Btn::icon(crate::icons::PLAY, "Run in terminals"))
                        .on_hover_text("Run in terminals")
                        .clicked()
                    {
                        *run = Some(s.clone());
                    }
                });
            });
            let preview: String = s.script.lines().take(3).collect::<Vec<_>>().join("\n");
            super::glass::well_frame(ui.ctx(), 8)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(preview).monospace().small());
                });
            ui.horizontal(|ui| {
                if !s.hotkey.is_empty() {
                    forms::kbd(ui, &s.hotkey, pal);
                }
                for t in &s.tags {
                    forms::chip(ui, t, false, pal);
                }
            });
        });
    });
}

impl App {
    pub fn workspaces_page(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let mut open: Option<Uuid> = None;
        let mut delete: Option<Uuid> = None;
        let mut save = false;
        page_header(
            ui,
            "Workspaces",
            "Reopen a whole set of tabs, splits and hosts in one click.",
            pal,
            |ui| {
                if ui
                    .add_enabled(
                        !self.tabs.is_empty(),
                        forms::Btn::primary(format!("{}  Save open tabs", crate::icons::PLUS)),
                    )
                    .on_disabled_hover_text("Open some tabs first")
                    .clicked()
                {
                    save = true;
                }
            },
        );
        if self.vault.workspaces.is_empty() {
            ui.label(
                RichText::new("No workspaces yet. Open the tabs you use together, then choose “Save open tabs”.")
                    .color(pal.text_dim),
            );
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    for w in &self.vault.workspaces {
                        row_frame(pal).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&w.name).family(crate::fonts::semibold()),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add(forms::Btn::icon(
                                                    crate::icons::TRASH,
                                                    "Delete workspace",
                                                ))
                                                .on_hover_text("Delete")
                                                .clicked()
                                            {
                                                delete = Some(w.id);
                                            }
                                            if ui.add(forms::Btn::primary("Open").small()).clicked()
                                            {
                                                open = Some(w.id);
                                            }
                                        },
                                    );
                                });
                                for t in &w.tabs {
                                    let names: Vec<String> = t
                                        .hosts
                                        .iter()
                                        .map(|h| match h {
                                            None => "Local".to_string(),
                                            Some(id) => self
                                                .vault
                                                .host(*id)
                                                .map(|h| h.display_name())
                                                .unwrap_or_else(|| "(deleted host)".into()),
                                        })
                                        .collect();
                                    let prefix = if t.sftp { "\u{E098} SFTP · " } else { "" };
                                    ui.label(
                                        RichText::new(format!("{prefix}{}", names.join("  |  ")))
                                            .small()
                                            .color(pal.text_dim),
                                    );
                                }
                            });
                        });
                    }
                });
            });
        if save {
            self.dialog = Some(Dialog::SaveWorkspace {
                name: String::new(),
            });
        }
        if let Some(id) = open {
            self.open_workspace(id);
        }
        if let Some(id) = delete {
            self.vault.delete(id);
            self.mark_dirty();
        }
    }
}
