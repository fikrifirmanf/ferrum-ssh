//! Focused editor workspace for hosts, groups, identities, keys, port forwarding rules and snippets.

use egui::{RichText, TextEdit};
use uuid::Uuid;

use super::{danger_button, primary_button, section_title};
use crate::app::{App, Dialog, Editor};
use crate::model::*;
use crate::themes::Palette;

/// A test belongs to one exact draft. Dropping it cancels pending network work.
pub struct HostTest {
    pub host: Host,
    session: u64,
    task: tokio::task::JoinHandle<()>,
    result: tokio::sync::oneshot::Receiver<Result<Option<String>, String>>,
    outcome: Option<Result<Option<String>, String>>,
}

impl Drop for HostTest {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum Outcome {
    Keep,
    Close,
    Save,
    SaveAndConnect,
    Delete,
}

fn field(ui: &mut egui::Ui, label: &str, pal: Palette) {
    ui.label(RichText::new(label).small().color(pal.text_dim));
}

fn opt_text(ui: &mut egui::Ui, value: &mut Option<String>, hint: &str, password: bool) {
    let mut s = value.clone().unwrap_or_default();
    let r = if password {
        super::password_input(ui, &mut s, hint, f32::INFINITY)
    } else {
        ui.add(
            TextEdit::singleline(&mut s)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        )
    };
    if r.changed() {
        *value = if s.is_empty() { None } else { Some(s) };
    }
}

fn combo_uuid(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut Option<Uuid>,
    items: &[(Uuid, String)],
    none_label: &str,
) {
    let current = value
        .and_then(|v| items.iter().find(|(i, _)| *i == v).map(|(_, l)| l.clone()))
        .unwrap_or_else(|| none_label.to_string());
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, none_label);
            for (i, l) in items {
                ui.selectable_value(value, Some(*i), l);
            }
        });
}

fn tri_toggle(ui: &mut egui::Ui, label: &str, value: &mut Option<bool>, inherited: bool) {
    ui.horizontal(|ui| {
        let mut v = value.unwrap_or(inherited);
        if ui.checkbox(&mut v, label).changed() {
            *value = Some(v);
        }
        if value.is_none() {
            ui.label(RichText::new("(inherited)").small().weak());
        } else if ui
            .small_button("reset")
            .on_hover_text("Inherit from group / defaults")
            .clicked()
        {
            *value = None;
        }
    });
}

impl App {
    pub fn cancel_host_test(&mut self) {
        if let Some(test) = self.host_test.take() {
            self.prompts
                .retain(|p| p.req.as_ref().is_none_or(|r| r.session != test.session));
        }
    }

    fn test_host(&mut self, host: &Host) {
        self.cancel_host_test();
        let mut target = self.vault.resolve(host);
        target.address = target.address.trim().to_string();
        let mut ctx = self.conn_template().headless();
        ctx.session = crate::conn::next_id();
        let session = ctx.session;
        let (tx, result) = tokio::sync::oneshot::channel();
        let repaint = self.bridge.ctx.clone();
        let task = self.rt.spawn(async move {
            let outcome = crate::conn::ssh::test_connection(target, ctx).await;
            let _ = tx.send(outcome);
            repaint.request_repaint();
        });
        self.host_test = Some(HostTest {
            host: host.clone(),
            session,
            task,
            result,
            outcome: None,
        });
    }

    fn host_test_control(&mut self, ui: &mut egui::Ui, host: &mut Host, pal: Palette) {
        if !matches!(host.protocol, Protocol::Ssh | Protocol::Mosh) {
            return;
        }
        if self.host_test.as_ref().is_some_and(|t| t.host != *host) {
            self.cancel_host_test();
        }
        if let Some(test) = &mut self.host_test {
            if test.outcome.is_none() {
                match test.result.try_recv() {
                    Ok(result) => test.outcome = Some(result),
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        test.outcome = Some(Err("Connection test stopped. Try again.".into()));
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }
        }
        if let Some(test) = &self.host_test {
            if test.outcome.is_some() {
                self.prompts
                    .retain(|p| p.req.as_ref().is_none_or(|r| r.session != test.session));
            }
        }
        let busy = self.host_test.as_ref().is_some_and(|t| t.outcome.is_none());
        super::card_frame(pal).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy && !host.address.trim().is_empty(),
                        egui::Button::new(if host.protocol == Protocol::Mosh {
                            "Test SSH login"
                        } else {
                            "Test connection"
                        }),
                    )
                    .clicked()
                {
                    self.test_host(host);
                }
                if busy {
                    ui.spinner();
                    if ui.add(super::ghost_button("Cancel")).clicked() {
                        self.cancel_host_test();
                    }
                }
            });
            if let Some(test) = &self.host_test {
                match &test.outcome {
                    Some(Ok(os)) => {
                        ui.label(
                            RichText::new(format!(
                                "{}  SSH login successful",
                                crate::icons::CHECK_CIRCLE
                            ))
                            .color(pal.success),
                        );
                        if let Some(os) = os {
                            ui.label(RichText::new(os).small().color(pal.text_dim));
                            host.os = Some(os.clone());
                        }
                    }
                    Some(Err(error)) => {
                        ui.label(RichText::new("Connection failed").color(pal.danger));
                        ui.label(RichText::new(error).small().color(pal.text_dim));
                    }
                    None => {
                        ui.label(
                            RichText::new("Connecting and checking your SSH login…")
                                .small()
                                .color(pal.text_dim),
                        );
                    }
                }
            } else {
                ui.label(
                    RichText::new("Check your SSH login before saving this host.")
                        .small()
                        .color(pal.text_dim),
                );
            }
        });
        // OS detection updates the draft; keep the test tied to that updated draft.
        if let Some(test) = &mut self.host_test {
            test.host.os = host.os.clone();
        }
    }

    pub fn editor_panel(&mut self, ui: &mut egui::Ui) {
        let Some(mut ed) = self.editor.take() else {
            return;
        };
        let pal = self.pal();
        let mut outcome = Outcome::Keep;
        let (title, is_new) = match &ed {
            Editor::Host(_, n) => ("Host", *n),
            Editor::Group(_, n) => ("Group", *n),
            Editor::Identity(_, n) => ("Identity", *n),
            Editor::Key(_, n) => ("Key", *n),
            Editor::Forward(_, n) => ("Port forwarding rule", *n),
            Editor::Snippet(_, n) => ("Snippet", *n),
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if is_new {
                    format!("New {}", title.to_lowercase())
                } else {
                    format!("Edit {}", title.to_lowercase())
                })
                .size(crate::themes::tokens::TITLE)
                .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(super::ghost_button("Back to library")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    outcome = Outcome::Close;
                }
            });
        });
        ui.separator();
        let footer_h = if matches!(&ed, Editor::Host(h, _) if matches!(h.protocol, Protocol::Ssh | Protocol::Mosh))
        {
            168.0
        } else {
            52.0
        };
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - footer_h).max(80.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                match &mut ed {
                    Editor::Host(h, _) => {
                        ui.push_id(h.id, |ui| self.host_form(ui, h, pal));
                    }
                    Editor::Group(g, _) => self.group_form(ui, g, pal),
                    Editor::Identity(i, _) => self.identity_form(ui, i, pal),
                    Editor::Key(k, n) => self.key_form(ui, k, *n, pal),
                    Editor::Forward(f, _) => self.forward_form(ui, f, pal),
                    Editor::Snippet(s, _) => self.snippet_form(ui, s, pal),
                }
            });
        if let Editor::Host(h, _) = &mut ed {
            ui.add_space(8.0);
            self.host_test_control(ui, h, pal);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new("Save changes")).clicked()
                || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S))
            {
                outcome = Outcome::Save;
            }
            if let Editor::Host(..) = ed {
                if ui.add(primary_button("Save and connect", pal)).clicked() {
                    outcome = Outcome::SaveAndConnect;
                }
            }
            if let Editor::Snippet(s, _) = &ed {
                if ui.button("\u{E3D0} Run").clicked() {
                    let s = s.clone();
                    self.begin_run_snippet(s);
                }
            }
            if !is_new {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(danger_button("Delete", pal)).clicked() {
                        outcome = Outcome::Delete;
                    }
                });
            }
        });

        match outcome {
            Outcome::Keep => self.editor = Some(ed),
            Outcome::Close => {}
            Outcome::Delete => {
                let (id, what) = match &ed {
                    Editor::Host(h, _) => (h.id, h.display_name()),
                    Editor::Group(g, _) => (g.id, format!("group {}", g.name)),
                    Editor::Identity(i, _) => (i.id, i.label.clone()),
                    Editor::Key(k, _) => (k.id, k.label.clone()),
                    Editor::Forward(f, _) => (f.id, f.label.clone()),
                    Editor::Snippet(s, _) => (s.id, s.name.clone()),
                };
                self.dialog = Some(Dialog::ConfirmDelete {
                    ids: vec![id],
                    what,
                });
            }
            Outcome::Save | Outcome::SaveAndConnect => {
                let connect = matches!(outcome, Outcome::SaveAndConnect);
                if let Err(msg) = self.commit_editor(ed, connect) {
                    self.toast(msg, true);
                }
            }
        }
    }

    fn commit_editor(&mut self, ed: Editor, connect: bool) -> Result<(), String> {
        let ts = now_ts();
        macro_rules! upsert {
            ($list:expr, $item:expr) => {{
                let mut item = $item;
                item.updated = ts;
                match $list.iter_mut().find(|x| x.id == item.id) {
                    Some(x) => *x = item,
                    None => $list.push(item),
                }
            }};
        }
        match ed {
            Editor::Host(h, is_new) => {
                if h.address.trim().is_empty() && h.protocol != Protocol::Serial {
                    self.editor = Some(Editor::Host(h, is_new));
                    return Err("Address is required".into());
                }
                let id = h.id;
                let mut h = *h;
                h.address = h.address.trim().to_string();
                upsert!(self.vault.hosts, h);
                self.inspected_host = Some(id);
                if connect {
                    self.open_host(id);
                }
            }
            Editor::Group(g, _) => {
                if g.name.trim().is_empty() {
                    self.editor = Some(Editor::Group(g, true));
                    return Err("Group name is required".into());
                }
                upsert!(self.vault.groups, g);
            }
            Editor::Identity(i, _) => upsert!(self.vault.identities, i),
            Editor::Key(k, _) => {
                if k.private_key.trim().is_empty() && k.public_key.trim().is_empty() {
                    self.editor = Some(Editor::Key(k, true));
                    return Err("Paste a private key or generate one".into());
                }
                upsert!(self.vault.keys, k);
            }
            Editor::Forward(f, _) => upsert!(self.vault.forwards, f),
            Editor::Snippet(s, _) => upsert!(self.vault.snippets, s),
        }
        self.mark_dirty();
        Ok(())
    }

    fn host_form(&mut self, ui: &mut egui::Ui, h: &mut Host, pal: Palette) {
        section_title(ui, "Address", pal);
        ui.horizontal(|ui| {
            for p in [
                Protocol::Ssh,
                Protocol::Mosh,
                Protocol::Telnet,
                Protocol::Serial,
            ] {
                ui.selectable_value(&mut h.protocol, p, p.label());
            }
        });
        if h.protocol != Protocol::Serial {
            field(ui, "Hostname or IP", pal);
            ui.add(
                TextEdit::singleline(&mut h.address)
                    .hint_text("example.com")
                    .desired_width(f32::INFINITY),
            );
        }
        field(ui, "Label", pal);
        ui.add(
            TextEdit::singleline(&mut h.label)
                .hint_text("My server")
                .desired_width(f32::INFINITY),
        );
        let groups: Vec<(Uuid, String)> = self
            .vault
            .groups
            .iter()
            .map(|g| (g.id, self.vault.group_path(Some(g.id))))
            .collect();
        field(ui, "Group", pal);
        combo_uuid(ui, "host-group", &mut h.group_id, &groups, "(none)");
        field(ui, "Tags (comma separated)", pal);
        let mut tags = h.tags.join(", ");
        if ui
            .add(
                TextEdit::singleline(&mut tags)
                    .hint_text("prod, db")
                    .desired_width(f32::INFINITY),
            )
            .changed()
        {
            h.tags = tags
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if h.protocol == Protocol::Serial {
            section_title(ui, "Serial", pal);
            serial_form(ui, &mut h.serial, pal);
            if h.address.is_empty() {
                h.address = h.serial.device.clone();
            }
            return;
        }

        let inherited = {
            let mut tmp = h.clone();
            tmp.settings = ConnSettings::default();
            self.vault.resolve(&tmp)
        };
        section_title(ui, "Connection", pal);
        ui.horizontal(|ui| {
            field(ui, "Port", pal);
            let mut port = h.settings.port.map(|p| p.to_string()).unwrap_or_default();
            if ui
                .add(
                    TextEdit::singleline(&mut port)
                        .hint_text(inherited.port.to_string())
                        .desired_width(80.0),
                )
                .changed()
            {
                h.settings.port = port.trim().parse().ok();
            }
        });
        if matches!(h.protocol, Protocol::Ssh | Protocol::Mosh) {
            let source = if h.group_id.is_some() {
                self.vault.group_path(h.group_id)
            } else {
                "connection defaults".into()
            };
            ui.label(
                RichText::new(format!("Unset values inherit from {source}."))
                    .small()
                    .color(pal.text_dim),
            );
            self.credentials_form(ui, &mut h.settings, &inherited, pal, "host");
        } else {
            section_title(ui, "Authentication", pal);
            field(ui, "Username", pal);
            opt_text(ui, &mut h.settings.username, &inherited.username, false);
            field(ui, "Password", pal);
            opt_text(ui, &mut h.settings.password, "Ask when connecting", true);
            ui.label(
                RichText::new("Telnet does not encrypt traffic. Use it on a trusted network.")
                    .small()
                    .color(pal.warning),
            );
        }

        if h.protocol == Protocol::Mosh {
            section_title(ui, "Mosh", pal);
            field(ui, "Server command", pal);
            ui.add(
                TextEdit::singleline(&mut h.mosh.server_command)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            ui.label(
                RichText::new("Requires mosh-server on the host and mosh-client locally.")
                    .small()
                    .weak(),
            );
        }

        egui::CollapsingHeader::new(RichText::new("Advanced").strong())
            .default_open(false)
            .show(ui, |ui| {
                self.advanced_form(ui, &mut h.settings, &inherited, Some(h.id), pal, "host");
            });
        section_title(ui, "Notes", pal);
        ui.add(
            TextEdit::multiline(&mut h.notes)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if let Some(os) = &h.os {
            ui.label(
                RichText::new(format!("Detected OS: {os}"))
                    .small()
                    .color(pal.text_dim),
            );
        }
    }

    fn credentials_form(
        &self,
        ui: &mut egui::Ui,
        s: &mut ConnSettings,
        inherited: &ResolvedTarget,
        pal: Palette,
        salt: &str,
    ) {
        section_title(ui, "Authentication", pal);
        let identities: Vec<(Uuid, String)> = self
            .vault
            .identities
            .iter()
            .map(|i| (i.id, format!("{} ({})", i.label, i.username)))
            .collect();
        if !identities.is_empty() {
            field(ui, "Identity", pal);
            combo_uuid(
                ui,
                &format!("{salt}-identity"),
                &mut s.identity_id,
                &identities,
                "(none)",
            );
        }
        field(ui, "Username", pal);
        opt_text(
            ui,
            &mut s.username,
            if inherited.username.is_empty() {
                "root"
            } else {
                &inherited.username
            },
            false,
        );
        field(ui, "Password", pal);
        opt_text(
            ui,
            &mut s.password,
            if inherited.password.is_some() {
                "•••••• (inherited)"
            } else {
                "Leave empty to be asked"
            },
            true,
        );
        let keys: Vec<(Uuid, String)> = self
            .vault
            .keys
            .iter()
            .map(|k| (k.id, format!("{}  ·  {}", k.label, k.key_type)))
            .collect();
        field(ui, "SSH key", pal);
        combo_uuid(ui, &format!("{salt}-key"), &mut s.key_id, &keys, "(none)");
        tri_toggle(
            ui,
            "Use SSH agent (incl. hardware / FIDO2 keys)",
            &mut s.use_agent,
            inherited.use_agent,
        );
    }

    fn advanced_form(
        &self,
        ui: &mut egui::Ui,
        s: &mut ConnSettings,
        inherited: &ResolvedTarget,
        self_id: Option<Uuid>,
        pal: Palette,
        salt: &str,
    ) {
        tri_toggle(
            ui,
            "Agent forwarding",
            &mut s.agent_forwarding,
            inherited.agent_forwarding,
        );
        tri_toggle(
            ui,
            "Strict host key checking (refuse untrusted hosts)",
            &mut s.strict_host_key_check,
            inherited.strict_host_key_check,
        );
        // Host chain / jump hosts.
        field(ui, "Host chain (jump hosts, in order)", pal);
        let mut remove = None;
        let mut up = None;
        for (i, id) in s.host_chain.iter().enumerate() {
            ui.horizontal(|ui| {
                let name = self
                    .vault
                    .host(*id)
                    .map(|h| h.display_name())
                    .unwrap_or_else(|| "(missing host)".into());
                ui.label(format!("{}. {name}", i + 1));
                if i > 0 && ui.small_button("↑").clicked() {
                    up = Some(i);
                }
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            s.host_chain.remove(i);
        }
        if let Some(i) = up {
            s.host_chain.swap(i, i - 1);
        }
        egui::ComboBox::from_id_salt(format!("{salt}-chain-add"))
            .selected_text("+ Add jump host")
            .show_ui(ui, |ui| {
                for h in &self.vault.hosts {
                    if Some(h.id) != self_id
                        && !s.host_chain.contains(&h.id)
                        && h.protocol == Protocol::Ssh
                    {
                        if ui.selectable_label(false, h.display_name()).clicked() {
                            s.host_chain.push(h.id);
                        }
                    }
                }
            });

        // Proxy
        field(ui, "Proxy", pal);
        let mut use_proxy = s.proxy.is_some();
        if ui
            .checkbox(&mut use_proxy, "Connect through a proxy")
            .changed()
        {
            s.proxy = if use_proxy {
                Some(ProxyConfig {
                    port: 1080,
                    ..Default::default()
                })
            } else {
                None
            };
        }
        if let Some(p) = s.proxy.as_mut() {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut p.kind, ProxyKind::Socks5, "SOCKS5");
                ui.selectable_value(&mut p.kind, ProxyKind::Http, "HTTP CONNECT");
                ui.selectable_value(&mut p.kind, ProxyKind::Command, "ProxyCommand");
            });
            if p.kind == ProxyKind::Command {
                ui.add(
                    TextEdit::singleline(&mut p.command)
                        .hint_text("nc -X 5 -x proxy:1080 %h %p")
                        .desired_width(f32::INFINITY),
                );
            } else {
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut p.host)
                            .hint_text("proxy host")
                            .desired_width(180.0),
                    );
                    let mut port = p.port.to_string();
                    if ui
                        .add(TextEdit::singleline(&mut port).desired_width(60.0))
                        .changed()
                    {
                        p.port = port.parse().unwrap_or(p.port);
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut p.username)
                            .hint_text("user (optional)")
                            .desired_width(120.0),
                    );
                    super::password_input(ui, &mut p.password, "password", 120.0);
                });
            }
        }

        // Startup snippet
        let snippets: Vec<(Uuid, String)> = self
            .vault
            .snippets
            .iter()
            .map(|s| (s.id, s.name.clone()))
            .collect();
        field(ui, "Startup snippet (runs after login)", pal);
        combo_uuid(
            ui,
            &format!("{salt}-startup"),
            &mut s.startup_snippet_id,
            &snippets,
            "(none)",
        );

        // Environment variables
        field(ui, "Environment variables", pal);
        let mut rm = None;
        for (i, (k, v)) in s.env.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(k)
                        .hint_text("NAME")
                        .desired_width(110.0),
                );
                ui.add(
                    TextEdit::singleline(v)
                        .hint_text("value")
                        .desired_width(150.0),
                );
                if ui.small_button("×").clicked() {
                    rm = Some(i);
                }
            });
        }
        if let Some(i) = rm {
            s.env.remove(i);
        }
        if ui.small_button("+ Add variable").clicked() {
            s.env.push((String::new(), String::new()));
        }

        ui.horizontal(|ui| {
            field(ui, "Keep-alive (s)", pal);
            let mut ka = s.keepalive_secs.map(|k| k.to_string()).unwrap_or_default();
            if ui
                .add(
                    TextEdit::singleline(&mut ka)
                        .hint_text(inherited.keepalive_secs.to_string())
                        .desired_width(60.0),
                )
                .changed()
            {
                s.keepalive_secs = ka.trim().parse().ok();
            }
        });
        ui.horizontal(|ui| {
            field(ui, "Connect timeout (s)", pal);
            let mut ct = s
                .connect_timeout_secs
                .map(|k| k.to_string())
                .unwrap_or_default();
            if ui
                .add(
                    TextEdit::singleline(&mut ct)
                        .hint_text(inherited.connect_timeout_secs.to_string())
                        .desired_width(60.0),
                )
                .changed()
            {
                s.connect_timeout_secs = ct.trim().parse().ok();
            }
        });
        ui.horizontal(|ui| {
            field(ui, "Charset", pal);
            let cur = s
                .charset
                .clone()
                .unwrap_or_else(|| inherited.charset.clone());
            egui::ComboBox::from_id_salt(format!("{salt}-charset"))
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    for c in crate::term::CHARSETS {
                        if ui
                            .selectable_label(s.charset.as_deref() == Some(*c), *c)
                            .clicked()
                        {
                            s.charset = Some(c.to_string());
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            field(ui, "Backspace sends", pal);
            let mut b = s.backspace.unwrap_or(inherited.backspace);
            if ui
                .selectable_value(&mut b, BackspaceMode::Del, "DEL (^?)")
                .changed()
                || ui
                    .selectable_value(&mut b, BackspaceMode::CtrlH, "^H")
                    .changed()
            {
                s.backspace = Some(b);
            }
        });
        ui.horizontal(|ui| {
            field(ui, "Color scheme", pal);
            let cur = s.color_scheme.clone().unwrap_or_else(|| "(default)".into());
            egui::ComboBox::from_id_salt(format!("{salt}-scheme"))
                .selected_text(cur)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(s.color_scheme.is_none(), "(default)")
                        .clicked()
                    {
                        s.color_scheme = None;
                    }
                    for sc in crate::themes::schemes() {
                        if ui
                            .selectable_label(s.color_scheme.as_deref() == Some(sc.name), sc.name)
                            .clicked()
                        {
                            s.color_scheme = Some(sc.name.to_string());
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            field(ui, "Font size", pal);
            let mut fs = s.font_size.map(|f| f.to_string()).unwrap_or_default();
            if ui
                .add(
                    TextEdit::singleline(&mut fs)
                        .hint_text("default")
                        .desired_width(60.0),
                )
                .changed()
            {
                s.font_size = fs.trim().parse().ok();
            }
        });
    }

    fn group_form(&mut self, ui: &mut egui::Ui, g: &mut Group, pal: Palette) {
        section_title(ui, "Group", pal);
        field(ui, "Name", pal);
        ui.add(
            TextEdit::singleline(&mut g.name)
                .hint_text("Production")
                .desired_width(f32::INFINITY),
        );
        let gid = g.id;
        let parents: Vec<(Uuid, String)> = self
            .vault
            .groups
            .iter()
            .filter(|x| {
                x.id != gid
                    && !self
                        .vault
                        .group_chain(Some(x.id))
                        .iter()
                        .any(|a| a.id == gid)
            })
            .map(|x| (x.id, self.vault.group_path(Some(x.id))))
            .collect();
        field(ui, "Parent group", pal);
        combo_uuid(
            ui,
            "group-parent",
            &mut g.parent_id,
            &parents,
            "(top level)",
        );
        ui.label(
            RichText::new("Settings below are inherited by every host in this group unless the host overrides them.")
                .small()
                .color(pal.text_dim),
        );
        let fake = Host {
            group_id: g.parent_id,
            ..Default::default()
        };
        let inherited = self.vault.resolve(&fake);
        ui.horizontal(|ui| {
            field(ui, "Port", pal);
            let mut port = g.settings.port.map(|p| p.to_string()).unwrap_or_default();
            if ui
                .add(
                    TextEdit::singleline(&mut port)
                        .hint_text("22")
                        .desired_width(80.0),
                )
                .changed()
            {
                g.settings.port = port.trim().parse().ok();
            }
        });
        self.credentials_form(ui, &mut g.settings, &inherited, pal, "group");
        egui::CollapsingHeader::new(RichText::new("Advanced").strong()).show(ui, |ui| {
            self.advanced_form(ui, &mut g.settings, &inherited, None, pal, "group");
        });
    }

    fn identity_form(&mut self, ui: &mut egui::Ui, i: &mut Identity, pal: Palette) {
        section_title(ui, "Identity", pal);
        ui.label(
            RichText::new("Reusable username + password/key bundle you can attach to many hosts.")
                .small()
                .color(pal.text_dim),
        );
        field(ui, "Label", pal);
        ui.add(TextEdit::singleline(&mut i.label).desired_width(f32::INFINITY));
        field(ui, "Username", pal);
        ui.add(TextEdit::singleline(&mut i.username).desired_width(f32::INFINITY));
        field(ui, "Password", pal);
        super::password_input(ui, &mut i.password, "", f32::INFINITY);
        let keys: Vec<(Uuid, String)> = self
            .vault
            .keys
            .iter()
            .map(|k| (k.id, k.label.clone()))
            .collect();
        field(ui, "Key", pal);
        combo_uuid(ui, "identity-key", &mut i.key_id, &keys, "(none)");
    }

    fn key_form(&mut self, ui: &mut egui::Ui, k: &mut SshKey, is_new: bool, pal: Palette) {
        section_title(ui, "Key", pal);
        field(ui, "Label", pal);
        ui.add(TextEdit::singleline(&mut k.label).desired_width(f32::INFINITY));
        if !k.key_type.is_empty() {
            ui.label(
                RichText::new(format!("Type: {}", k.key_type))
                    .small()
                    .color(pal.text_dim),
            );
        }
        if !k.public_key.is_empty() {
            ui.label(
                RichText::new(format!(
                    "Fingerprint: {}",
                    crate::keys::fingerprint_of_public(&k.public_key)
                ))
                .small()
                .monospace()
                .color(pal.text_dim),
            );
        }
        field(ui, "Public key", pal);
        ui.add(
            TextEdit::multiline(&mut k.public_key)
                .desired_rows(2)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if ui.button("\u{E198} Copy public key").clicked() {
                ui.ctx().copy_text(k.public_key.clone());
            }
            if !is_new && ui.button("\u{E08E} Install on host…").clicked() {
                self.dialog = Some(Dialog::InstallKey {
                    key_id: k.id,
                    host_id: None,
                    busy: false,
                });
            }
        });
        field(ui, "Private key", pal);
        let id = ui.make_persistent_id(("show-priv", k.id));
        let mut show = ui.data_mut(|d| *d.get_temp_mut_or(id, is_new));
        if ui.checkbox(&mut show, "Show private key").changed() {
            ui.data_mut(|d| d.insert_temp(id, show));
        }
        if show {
            let r = ui.add(
                TextEdit::multiline(&mut k.private_key)
                    .desired_rows(6)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("-----BEGIN OPENSSH PRIVATE KEY-----")
                    .desired_width(f32::INFINITY),
            );
            if r.changed() {
                if let Ok(parsed) = crate::keys::import(&k.label, &k.private_key, &k.passphrase) {
                    k.public_key = parsed.public_key;
                    k.key_type = parsed.key_type;
                }
            }
        }
        field(ui, "Passphrase", pal);
        super::password_input(ui, &mut k.passphrase, "optional", f32::INFINITY);
        field(ui, "OpenSSH certificate (optional)", pal);
        ui.add(
            TextEdit::multiline(&mut k.certificate)
                .desired_rows(2)
                .font(egui::TextStyle::Monospace)
                .hint_text("ssh-ed25519-cert-v01@openssh.com AAAA…")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if ui.button("\u{E248} Export private…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name("id_ferrum")
                    .save_file()
                {
                    let r = std::fs::write(&p, &k.private_key);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ =
                            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
                    }
                    self.toast(
                        if r.is_ok() {
                            "Private key exported"
                        } else {
                            "Export failed"
                        },
                        r.is_err(),
                    );
                }
            }
            if ui.button("\u{E248} Export public…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .set_file_name("id_ferrum.pub")
                    .save_file()
                {
                    let r = std::fs::write(&p, format!("{}\n", k.public_key.trim()));
                    self.toast(
                        if r.is_ok() {
                            "Public key exported"
                        } else {
                            "Export failed"
                        },
                        r.is_err(),
                    );
                }
            }
        });
    }

    fn forward_form(&mut self, ui: &mut egui::Ui, f: &mut PortForward, pal: Palette) {
        section_title(ui, "Rule", pal);
        field(ui, "Label", pal);
        ui.add(
            TextEdit::singleline(&mut f.label)
                .hint_text("Postgres tunnel")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            for k in [
                ForwardKind::Local,
                ForwardKind::Remote,
                ForwardKind::Dynamic,
            ] {
                ui.selectable_value(&mut f.kind, k, k.label());
            }
        });
        ui.label(
            RichText::new(match f.kind {
                ForwardKind::Local => "Local: connections to bind address on this machine are forwarded through the host to the destination.",
                ForwardKind::Remote => "Remote: connections to bind address on the remote host come back to the destination reachable from this machine.",
                ForwardKind::Dynamic => "Dynamic: a local SOCKS4/5 proxy that routes any destination through the host.",
            })
            .small()
            .color(pal.text_dim),
        );
        let hosts: Vec<(Uuid, String)> = self
            .vault
            .hosts
            .iter()
            .filter(|h| h.protocol == Protocol::Ssh)
            .map(|h| (h.id, h.display_name()))
            .collect();
        field(ui, "Through host", pal);
        combo_uuid(ui, "fw-host", &mut f.host_id, &hosts, "(choose host)");
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                field(
                    ui,
                    if f.kind == ForwardKind::Remote {
                        "Remote bind address"
                    } else {
                        "Bind address"
                    },
                    pal,
                );
                ui.add(TextEdit::singleline(&mut f.bind_address).desired_width(150.0));
            });
            ui.vertical(|ui| {
                field(ui, "Port", pal);
                let mut p = f.bind_port.to_string();
                if ui
                    .add(TextEdit::singleline(&mut p).desired_width(70.0))
                    .changed()
                {
                    f.bind_port = p.parse().unwrap_or(f.bind_port);
                }
            });
        });
        if f.kind != ForwardKind::Dynamic {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    field(ui, "Destination host", pal);
                    ui.add(TextEdit::singleline(&mut f.dest_host).desired_width(150.0));
                });
                ui.vertical(|ui| {
                    field(ui, "Port", pal);
                    let mut p = f.dest_port.to_string();
                    if ui
                        .add(TextEdit::singleline(&mut p).desired_width(70.0))
                        .changed()
                    {
                        f.dest_port = p.parse().unwrap_or(f.dest_port);
                    }
                });
            });
        }
        ui.checkbox(
            &mut f.auto_start,
            "Start automatically when connecting to this host",
        );
    }

    fn hotkey_field(&mut self, ui: &mut egui::Ui, s: &mut Snippet, pal: Palette) {
        let rec_id = egui::Id::unique(("hotkey-recording", s.id));
        let mut recording = ui
            .memory(|m| m.data.get_temp::<bool>(rec_id))
            .unwrap_or(false);
        if recording {
            let pressed = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, mods)) = pressed {
                if key == egui::Key::Escape {
                    recording = false;
                } else if mods != egui::Modifiers::NONE {
                    s.hotkey = crate::hotkeys::format(mods, key);
                    recording = false;
                }
            }
        }
        ui.horizontal(|ui| {
            let label = if recording {
                "Press the shortcut… (Esc cancels)".to_string()
            } else if s.hotkey.is_empty() {
                "Record shortcut".to_string()
            } else {
                format!("\u{E2D8}  {}", s.hotkey)
            };
            if ui.button(label).clicked() {
                recording = !recording;
            }
            if !s.hotkey.is_empty() && ui.small_button("Clear").clicked() {
                s.hotkey.clear();
            }
        });
        ui.memory_mut(|m| m.data.insert_temp(rec_id, recording));
        if recording {
            ui.ctx().request_repaint();
        }
        if let Some((m, k)) = crate::hotkeys::parse(&s.hotkey) {
            if crate::hotkeys::is_reserved(m, k) {
                ui.colored_label(pal.danger, "Ferrum already uses this shortcut");
            } else if self
                .vault
                .snippets
                .iter()
                .any(|o| o.id != s.id && crate::hotkeys::parse(&o.hotkey) == Some((m, k)))
            {
                ui.colored_label(pal.warning, "Another snippet already uses this shortcut");
            }
        }
    }

    fn snippet_form(&mut self, ui: &mut egui::Ui, s: &mut Snippet, pal: Palette) {
        section_title(ui, "Snippet", pal);
        field(ui, "Name", pal);
        ui.add(TextEdit::singleline(&mut s.name).desired_width(f32::INFINITY));
        field(ui, "Package (folder)", pal);
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut s.package)
                    .hint_text("e.g. Docker, Deploy")
                    .desired_width(180.0),
            );
            let mut known: Vec<String> = self
                .vault
                .snippets
                .iter()
                .map(|x| x.package.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            known.sort();
            known.dedup();
            if !known.is_empty() {
                egui::ComboBox::from_id_salt("snippet-package")
                    .selected_text("existing…")
                    .show_ui(ui, |ui| {
                        for p in known {
                            if ui.selectable_label(s.package == p, &p).clicked() {
                                s.package = p;
                            }
                        }
                    });
            }
        });
        field(ui, "Hotkey (runs it in the active terminal)", pal);
        self.hotkey_field(ui, s, pal);
        field(ui, "Script", pal);
        ui.add(
            TextEdit::multiline(&mut s.script)
                .code_editor()
                .desired_rows(8)
                .desired_width(f32::INFINITY)
                .hint_text("sudo systemctl restart {{service}}"),
        );
        ui.label(
            RichText::new(
                "Use {{variable}} placeholders — you'll be asked for values when running.",
            )
            .small()
            .color(pal.text_dim),
        );
        field(ui, "Description", pal);
        ui.add(TextEdit::singleline(&mut s.description).desired_width(f32::INFINITY));
        field(ui, "Tags (comma separated)", pal);
        let mut tags = s.tags.join(", ");
        if ui
            .add(TextEdit::singleline(&mut tags).desired_width(f32::INFINITY))
            .changed()
        {
            s.tags = tags
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect();
        }
    }
}

pub fn serial_form(ui: &mut egui::Ui, cfg: &mut SerialConfig, pal: Palette) {
    let ports = crate::conn::serial::list_ports();
    field(ui, "Device", pal);
    ui.horizontal(|ui| {
        ui.add(
            TextEdit::singleline(&mut cfg.device)
                .hint_text("/dev/ttyUSB0 or COM3")
                .desired_width(200.0),
        );
        egui::ComboBox::from_id_salt("serial-ports")
            .selected_text("Detected")
            .show_ui(ui, |ui| {
                if ports.is_empty() {
                    ui.label("No serial ports found");
                }
                for p in &ports {
                    if ui.selectable_label(cfg.device == *p, p).clicked() {
                        cfg.device = p.clone();
                    }
                }
            });
    });
    egui::Grid::new("serial-grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Baud rate");
            egui::ComboBox::from_id_salt("baud")
                .selected_text(cfg.baud_rate.to_string())
                .show_ui(ui, |ui| {
                    for b in crate::conn::serial::BAUD_RATES {
                        ui.selectable_value(&mut cfg.baud_rate, *b, b.to_string());
                    }
                });
            ui.end_row();
            ui.label("Data bits");
            ui.horizontal(|ui| {
                for b in [5u8, 6, 7, 8] {
                    ui.selectable_value(&mut cfg.data_bits, b, b.to_string());
                }
            });
            ui.end_row();
            ui.label("Stop bits");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut cfg.stop_bits, 1, "1");
                ui.selectable_value(&mut cfg.stop_bits, 2, "2");
            });
            ui.end_row();
            ui.label("Parity");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut cfg.parity, Parity::None, "None");
                ui.selectable_value(&mut cfg.parity, Parity::Odd, "Odd");
                ui.selectable_value(&mut cfg.parity, Parity::Even, "Even");
            });
            ui.end_row();
            ui.label("Flow control");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut cfg.flow_control, FlowControl::None, "None");
                ui.selectable_value(&mut cfg.flow_control, FlowControl::Software, "XON/XOFF");
                ui.selectable_value(&mut cfg.flow_control, FlowControl::Hardware, "RTS/CTS");
            });
            ui.end_row();
            ui.label("Enter sends");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut cfg.line_ending, "\r".to_string(), "CR");
                ui.selectable_value(&mut cfg.line_ending, "\n".to_string(), "LF");
                ui.selectable_value(&mut cfg.line_ending, "\r\n".to_string(), "CRLF");
            });
            ui.end_row();
        });
    ui.checkbox(&mut cfg.local_echo, "Local echo");
}
