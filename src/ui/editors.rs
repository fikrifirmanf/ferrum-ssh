//! Focused editor workspace for hosts, groups, identities, keys, port forwarding rules and snippets.

use egui::RichText;
use uuid::Uuid;

use super::{forms, motion};
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

/// Picks one id from a list, or none. The chosen label shows even while inheriting.
fn select_uuid(
    ui: &mut egui::Ui,
    title: &str,
    value: &mut Option<Uuid>,
    items: &[(Uuid, String)],
    none_label: &str,
) {
    let pal = crate::themes::current(ui.ctx());
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        forms::label(ui, title, pal);
        let shown = value
            .and_then(|v| items.iter().find(|(i, _)| *i == v).map(|(_, l)| l.clone()))
            .unwrap_or_else(|| none_label.to_string());
        let width = ui.available_width();
        forms::select(ui, title, width, &shown, false, |ui| {
            forms::select_value(ui, value, None, none_label);
            for (i, l) in items {
                forms::select_value(ui, value, Some(*i), l);
            }
        });
    });
}

/// What the host draft would run in a terminal. Vault keys and passwords have no file or flag
/// to show, so they stay out, and a host with neither address nor user shows placeholders.
pub fn equivalent_command(h: &Host, r: &ResolvedTarget) -> String {
    let host = if h.address.trim().is_empty() {
        "<host>"
    } else {
        h.address.trim()
    };
    let user = if r.username.is_empty() {
        "<user>"
    } else {
        r.username.as_str()
    };
    match h.protocol {
        Protocol::Serial => format!("screen {} {}", h.serial.device, h.serial.baud_rate),
        Protocol::Telnet => format!("telnet {host} {}", r.port),
        Protocol::Mosh => {
            let ssh = if r.port == Protocol::Ssh.default_port() {
                String::new()
            } else {
                format!("--ssh=\"ssh -p {}\" ", r.port)
            };
            format!("mosh {ssh}{user}@{host}")
        }
        Protocol::Local => "$SHELL".into(),
        Protocol::Ssh => {
            let mut parts = vec!["ssh".to_string()];
            if r.port != Protocol::Ssh.default_port() {
                parts.push(format!("-p {}", r.port));
            }
            if !r.chain.is_empty() {
                let jumps: Vec<String> = r
                    .chain
                    .iter()
                    .map(|j| {
                        let port = if j.port == 22 {
                            String::new()
                        } else {
                            format!(":{}", j.port)
                        };
                        if j.username.is_empty() {
                            format!("{}{port}", j.address)
                        } else {
                            format!("{}@{}{port}", j.username, j.address)
                        }
                    })
                    .collect();
                parts.push(format!("-J {}", jumps.join(",")));
            }
            if r.agent_forwarding {
                parts.push("-A".into());
            }
            if let Some(proxy) = &r.proxy {
                let command = match proxy.kind {
                    ProxyKind::Command => proxy.command.clone(),
                    ProxyKind::Socks5 => {
                        format!("nc -X 5 -x {}:{} %h %p", proxy.host, proxy.port)
                    }
                    ProxyKind::Http => {
                        format!("nc -X connect -x {}:{} %h %p", proxy.host, proxy.port)
                    }
                };
                parts.push(format!("-o ProxyCommand=\"{command}\""));
            }
            parts.push(format!("{user}@{host}"));
            parts.join(" ")
        }
    }
}

fn plural(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
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

    fn poll_host_test(&mut self, host: &mut Host) {
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
        if let Some(test) = &self.host_test {
            if let Some(Ok(Some(os))) = &test.outcome {
                host.os = Some(os.clone());
            }
        }
        // OS detection updates the draft; keep the test tied to that updated draft.
        if let Some(test) = &mut self.host_test {
            test.host.os = host.os.clone();
        }
    }

    /// The result sits right above the Test button and folds open, so feedback lands where
    /// the eye already is. Failed results keep every field untouched.
    fn host_test_result(&mut self, ui: &mut egui::Ui, pal: Palette) {
        let outcome = self
            .host_test
            .as_ref()
            .and_then(|test| test.outcome.as_ref())
            .cloned();
        forms::reveal(
            ui,
            egui::Id::unique("host-test-result"),
            outcome.is_some(),
            |ui| {
                let Some(outcome) = &outcome else {
                    return;
                };
                super::glass::card_frame(ui.ctx(), crate::themes::tokens::RADIUS_INPUT)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 2.0;
                        match outcome {
                            Ok(os) => {
                                ui.label(
                                    RichText::new(format!(
                                        "{}  SSH login successful",
                                        crate::icons::CHECK_CIRCLE
                                    ))
                                    .family(crate::fonts::medium())
                                    .color(pal.success),
                                );
                                if let Some(os) = os {
                                    forms::help(ui, os, pal);
                                }
                            }
                            Err(error) => {
                                ui.label(
                                    RichText::new(format!(
                                        "{}  Connection failed",
                                        crate::icons::X_CIRCLE
                                    ))
                                    .family(crate::fonts::medium())
                                    .color(pal.danger),
                                );
                                forms::help(ui, error, pal);
                            }
                        }
                    });
                ui.add_space(8.0);
            },
        );
    }

    fn editor_is_dirty(&self, ed: &Editor) -> bool {
        match ed {
            Editor::Host(h, new) => !*new && self.vault.host(h.id).is_none_or(|x| x != &**h),
            Editor::Group(g, new) => !*new && self.vault.group(g.id).is_none_or(|x| x != g),
            Editor::Identity(i, new) => !*new && self.vault.identity(i.id).is_none_or(|x| x != i),
            Editor::Key(k, new) => !*new && self.vault.key(k.id).is_none_or(|x| x != k),
            Editor::Forward(f, new) => {
                !*new && self.vault.forwards.iter().find(|x| x.id == f.id) != Some(f)
            }
            Editor::Snippet(s, new) => !*new && self.vault.snippet(s.id).is_none_or(|x| x != s),
        }
    }

    pub fn editor_panel(&mut self, ui: &mut egui::Ui) {
        let Some(mut ed) = self.editor.take() else {
            return;
        };
        let pal = self.pal();
        let mut outcome = Outcome::Keep;
        let (title, is_new, back) = match &ed {
            Editor::Host(_, n) => ("host", *n, "Hosts"),
            Editor::Group(_, n) => ("group", *n, "Hosts"),
            Editor::Identity(_, n) => ("identity", *n, "Keychain"),
            Editor::Key(_, n) => ("key", *n, "Keychain"),
            Editor::Forward(_, n) => ("port forwarding rule", *n, "Port forwarding"),
            Editor::Snippet(_, n) => ("snippet", *n, "Snippets"),
        };
        let dirty = self.editor_is_dirty(&ed);
        if ui
            .add(forms::Btn::ghost(format!("{}  {back}", crate::icons::ARROW_LEFT)).small())
            .clicked()
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            outcome = Outcome::Close;
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            forms::heading(
                ui,
                &format!("{} {title}", if is_new { "New" } else { "Edit" }),
                crate::themes::tokens::TITLE,
            );
            let shown = motion::ease(
                ui.ctx(),
                egui::Id::unique("editor-dirty"),
                dirty,
                motion::CONTROL,
            );
            if shown > 0.0 {
                ui.scope(|ui| {
                    ui.set_opacity(shown);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    forms::dot(ui, pal.accent);
                    ui.label(RichText::new("Unsaved changes").small().color(pal.text_dim));
                });
            }
        });
        ui.add_space(4.0);
        let footer_key = egui::Id::unique("editor-footer-height");
        let footer_h = ui.data(|d| d.get_temp::<f32>(footer_key).unwrap_or(60.0));
        let body_height = (ui.available_height() - footer_h).max(80.0);
        // The scroll area overhangs the column by 4 pt so focus rings are not clipped.
        let overhang = egui::Rect::from_min_size(
            ui.cursor().min - egui::vec2(4.0, 0.0),
            egui::vec2(ui.available_width() + 8.0, body_height),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(overhang), |ui| {
            egui::ScrollArea::vertical()
                .max_height(body_height)
                .min_scrolled_height(body_height.min(80.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(4, 4))
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 12.0;
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
                });
        });
        let footer_top = ui.cursor().top();
        ui.painter().hline(
            ui.min_rect().x_range(),
            footer_top,
            egui::Stroke::new(1.0, pal.border),
        );
        ui.add_space(12.0);
        if let Editor::Host(h, _) = &mut ed {
            self.poll_host_test(h);
            self.host_test_result(ui, pal);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Editor::Host(h, _) = &mut ed {
                if matches!(h.protocol, Protocol::Ssh | Protocol::Mosh) {
                    let busy = self.host_test.as_ref().is_some_and(|t| t.outcome.is_none());
                    let label = if busy {
                        "Testing…"
                    } else if h.protocol == Protocol::Mosh {
                        "Test SSH login"
                    } else {
                        "Test connection"
                    };
                    if ui
                        .add_enabled(
                            !busy && !h.address.trim().is_empty(),
                            forms::Btn::secondary(label),
                        )
                        .clicked()
                    {
                        self.test_host(h);
                    }
                    if busy {
                        ui.spinner();
                        if ui.add(forms::Btn::ghost("Cancel")).clicked() {
                            self.cancel_host_test();
                        }
                    }
                }
            }
            if let Editor::Snippet(s, _) = &ed {
                if ui
                    .add(forms::Btn::secondary(format!(
                        "{}  Run",
                        crate::icons::PLAY
                    )))
                    .clicked()
                {
                    let s = s.clone();
                    self.begin_run_snippet(s);
                }
            }
            if !is_new && ui.add(forms::Btn::danger("Delete")).clicked() {
                outcome = Outcome::Delete;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let is_host = matches!(ed, Editor::Host(..));
                if is_host {
                    if ui.add(forms::Btn::primary("Save and connect")).clicked() {
                        outcome = Outcome::SaveAndConnect;
                    }
                }
                let can_save = is_new || dirty;
                let save = if is_host {
                    forms::Btn::ghost("Save")
                } else {
                    forms::Btn::primary("Save")
                };
                if ui.add_enabled(can_save, save).clicked()
                    || (can_save
                        && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)))
                {
                    outcome = Outcome::Save;
                }
            });
        });
        let footer_bottom = ui.min_rect().bottom();
        ui.data_mut(|d| d.insert_temp(footer_key, (footer_bottom - footer_top + 8.0).max(48.0)));

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
                match self.commit_editor(ed, connect) {
                    Ok(saved) => self.toast(format!("Saved {saved}"), false),
                    Err(msg) => {
                        ui.data_mut(|d| d.insert_temp(address_error_key(), true));
                        self.toast(msg, true);
                    }
                }
            }
        }
    }

    fn commit_editor(&mut self, ed: Editor, connect: bool) -> Result<String, String> {
        let ts = now_ts();
        let saved = match &ed {
            Editor::Host(h, _) => h.display_name(),
            Editor::Group(g, _) => g.name.clone(),
            Editor::Identity(i, _) => i.label.clone(),
            Editor::Key(k, _) => k.label.clone(),
            Editor::Forward(f, _) => f.label.clone(),
            Editor::Snippet(s, _) => s.name.clone(),
        };
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
        Ok(saved)
    }

    fn host_form(&mut self, ui: &mut egui::Ui, h: &mut Host, pal: Palette) {
        let inherited = {
            let mut tmp = h.clone();
            tmp.settings = ConnSettings::default();
            self.vault.resolve(&tmp)
        };
        let source = if h.group_id.is_some() {
            self.vault.group_path(h.group_id)
        } else {
            "connection defaults".to_string()
        };
        let resolved = self.vault.resolve(h);
        forms::command_line(ui, &equivalent_command(h, &resolved));
        forms::card(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                forms::label(ui, "Protocol", pal);
                forms::segmented(
                    ui,
                    "protocol",
                    &mut h.protocol,
                    &[
                        (Protocol::Ssh, "SSH"),
                        (Protocol::Mosh, "Mosh"),
                        (Protocol::Telnet, "Telnet"),
                        (Protocol::Serial, "Serial"),
                    ],
                );
            });
            let wants_user = matches!(h.protocol, Protocol::Ssh | Protocol::Mosh);
            let address_error = ui
                .data(|d| d.get_temp::<bool>(address_error_key()).unwrap_or(false))
                && h.address.trim().is_empty();
            forms::reveal(
                ui,
                egui::Id::unique("address-network"),
                h.protocol != Protocol::Serial,
                |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let default_port = h.protocol.default_port();
                    let note = if h.settings.port.is_some() {
                        "Port set on this host".to_string()
                    } else {
                        format!("Port {} comes from {source}", inherited.port)
                    };
                    forms::label_row(ui, "Address", &note, pal);
                    let user_hint = if inherited.username.is_empty() {
                        "user".to_string()
                    } else {
                        inherited.username.clone()
                    };
                    let port_hint = if h.protocol == Protocol::Ssh {
                        inherited.port.to_string()
                    } else {
                        default_port.to_string()
                    };
                    forms::address_line(
                        ui,
                        wants_user.then_some(&mut h.settings.username),
                        &mut h.address,
                        &mut h.settings.port,
                        forms::AddressHints {
                            user: &user_hint,
                            host: "host or IP",
                            port: &port_hint,
                        },
                        address_error,
                    );
                    if address_error {
                        forms::error_line(ui, "Enter a hostname or IP to connect.", pal);
                    }
                },
            );
            forms::reveal(
                ui,
                egui::Id::unique("address-serial"),
                h.protocol == Protocol::Serial,
                |ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    serial_form(ui, &mut h.serial, pal);
                    if h.address.is_empty() {
                        h.address = h.serial.device.clone();
                    }
                },
            );
            let groups: Vec<(Uuid, String)> = self
                .vault
                .groups
                .iter()
                .map(|g| (g.id, self.vault.group_path(Some(g.id))))
                .collect();
            let label_hint = if h.address.is_empty() {
                "Name shown in the library".to_string()
            } else {
                h.address.clone()
            };
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new("Label")
                        .hint(&label_hint)
                        .show(ui, &mut h.label);
                } else {
                    select_uuid(ui, "Group", &mut h.group_id, &groups, "Ungrouped");
                }
            });
            forms::tag_input(ui, "Tags", &mut h.tags);
        });

        if matches!(h.protocol, Protocol::Ssh | Protocol::Mosh) {
            self.credentials_section(ui, &mut h.settings, &inherited, &source, pal, "host");
        }
        forms::reveal(
            ui,
            egui::Id::unique("telnet-signin"),
            h.protocol == Protocol::Telnet,
            |ui| {
                forms::section(ui, "telnet", "Sign-in", "", true, |ui| {
                    forms::columns(ui, &[1.0, 1.0], |column, ui| {
                        if column == 0 {
                            forms::opt_text(
                                ui,
                                forms::Input::new("Username").hint(&inherited.username),
                                &mut h.settings.username,
                            );
                        } else {
                            let mut password = h.settings.password.clone().unwrap_or_default();
                            if forms::Input::new("Password")
                                .hint("Ask when connecting")
                                .password()
                                .show(ui, &mut password)
                                .changed()
                            {
                                h.settings.password = (!password.is_empty()).then_some(password);
                            }
                        }
                    });
                    ui.label(
                        RichText::new(format!(
                            "{}  Telnet does not encrypt traffic. Use it on a trusted network.",
                            crate::icons::WARNING
                        ))
                        .size(12.0)
                        .color(pal.warning),
                    );
                });
                ui.add_space(12.0);
            },
        );
        forms::reveal(
            ui,
            egui::Id::unique("mosh-section"),
            h.protocol == Protocol::Mosh,
            |ui| {
                forms::section(
                    ui,
                    "mosh",
                    "Mosh",
                    &h.mosh.server_command.clone(),
                    false,
                    |ui| {
                        forms::Input::new("Server command")
                            .mono()
                            .help("Requires mosh-server on the host and mosh-client locally.")
                            .show(ui, &mut h.mosh.server_command);
                    },
                );
                ui.add_space(12.0);
            },
        );
        if h.protocol != Protocol::Serial {
            self.advanced_sections(
                ui,
                &mut h.settings,
                &inherited,
                Some(h.id),
                &source,
                pal,
                "host",
            );
        }
        let first_note = h.notes.lines().next().unwrap_or("").to_string();
        forms::section(ui, "notes", "Notes", &first_note, false, |ui| {
            forms::text_area(
                ui,
                "",
                &mut h.notes,
                3,
                false,
                "Anything worth remembering about this host",
            );
            if let Some(os) = &h.os {
                forms::help(ui, &format!("Detected system: {os}"), pal);
            }
        });
    }

    fn credentials_section(
        &self,
        ui: &mut egui::Ui,
        s: &mut ConnSettings,
        inherited: &ResolvedTarget,
        source: &str,
        pal: Palette,
        salt: &str,
    ) {
        let key_label = s
            .key_id
            .and_then(|id| self.vault.key(id))
            .or(inherited.key.as_ref())
            .map(|k| k.label.clone());
        let mut methods = vec![];
        if let Some(label) = &key_label {
            methods.push(format!("Key: {label}"));
        }
        if s.use_agent.unwrap_or(inherited.use_agent) {
            methods.push("SSH agent".to_string());
        }
        if s.password.is_some() || inherited.password.is_some() {
            methods.push("Password".to_string());
        }
        let summary = if methods.is_empty() {
            "Ask when connecting".to_string()
        } else {
            methods.join(", ")
        };
        forms::section(
            ui,
            &format!("{salt}-auth"),
            "Authentication",
            &summary,
            true,
            |ui| {
                let identities: Vec<(Uuid, String)> = self
                    .vault
                    .identities
                    .iter()
                    .map(|i| (i.id, format!("{} ({})", i.label, i.username)))
                    .collect();
                if !identities.is_empty() {
                    select_uuid(ui, "Identity", &mut s.identity_id, &identities, "None");
                }
                // These are views onto settings that can all be on at once, so switching never
                // erases a saved key or password.
                let tab_key = ui.scope_id().with("auth-tab");
                let default_tab = if s.key_id.is_some() {
                    1
                } else if s.password.is_some() {
                    2
                } else {
                    0
                };
                let mut tab: u8 = ui.data(|d| d.get_temp(tab_key).unwrap_or(default_tab));
                if forms::segmented(
                    ui,
                    "auth-tab",
                    &mut tab,
                    &[(0, "Agent"), (1, "Key"), (2, "Password")],
                ) {
                    ui.data_mut(|d| d.insert_temp(tab_key, tab));
                }
                forms::reveal(ui, tab_key.with(0), tab == 0, |ui| {
                    forms::inherit_row(
                        ui,
                        "Use SSH agent",
                        "Uses the keys in your agent, including hardware and FIDO2 keys.",
                        &mut s.use_agent,
                        inherited.use_agent,
                        source,
                    );
                });
                forms::reveal(ui, tab_key.with(1), tab == 1, |ui| {
                    let keys: Vec<(Uuid, String)> = self
                        .vault
                        .keys
                        .iter()
                        .map(|k| (k.id, format!("{}  ·  {}", k.label, k.key_type)))
                        .collect();
                    let none = inherited
                        .key
                        .as_ref()
                        .map(|k| format!("{} (from {source})", k.label))
                        .unwrap_or_else(|| "None".to_string());
                    select_uuid(ui, "Key", &mut s.key_id, &keys, &none);
                });
                forms::reveal(ui, tab_key.with(2), tab == 2, |ui| {
                    let mut password = s.password.clone().unwrap_or_default();
                    let hint = if inherited.password.is_some() {
                        format!("Inherited from {source}")
                    } else {
                        "Stored in your encrypted vault".to_string()
                    };
                    if forms::Input::new("Password")
                        .hint(&hint)
                        .password()
                        .show(ui, &mut password)
                        .changed()
                    {
                        s.password = (!password.is_empty()).then_some(password);
                    }
                });
                forms::help(
                    ui,
                    "Ferrum tries your key, then the agent, then the password, and asks if none work.",
                    pal,
                );
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn advanced_sections(
        &self,
        ui: &mut egui::Ui,
        s: &mut ConnSettings,
        inherited: &ResolvedTarget,
        self_id: Option<Uuid>,
        source: &str,
        pal: Palette,
        salt: &str,
    ) {
        let route = {
            let mut parts = vec![];
            if s.host_chain.is_empty() && inherited.chain.is_empty() {
                parts.push("Direct".to_string());
            } else {
                parts.push(format!(
                    "Via {}",
                    plural(s.host_chain.len().max(inherited.chain.len()), "jump host")
                ));
            }
            if let Some(proxy) = s.proxy.as_ref().or(inherited.proxy.as_ref()) {
                parts.push(
                    match proxy.kind {
                        ProxyKind::Socks5 => "SOCKS5",
                        ProxyKind::Http => "HTTP",
                        ProxyKind::Command => "ProxyCommand",
                    }
                    .to_string(),
                );
            }
            parts.join(", ")
        };
        forms::section(
            ui,
            &format!("{salt}-route"),
            "Route and proxy",
            &route,
            false,
            |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    forms::label(ui, "Jump hosts, in order", pal);
                    let mut remove = None;
                    let mut up = None;
                    for (i, id) in s.host_chain.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let name = self
                                .vault
                                .host(*id)
                                .map(|h| h.display_name())
                                .unwrap_or_else(|| "(missing host)".into());
                            ui.label(
                                RichText::new(format!("{}.", i + 1))
                                    .monospace()
                                    .color(pal.faint),
                            );
                            ui.label(name);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            forms::Btn::icon(crate::icons::X, "Remove jump host")
                                                .height(28.0),
                                        )
                                        .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                    if i > 0
                                        && ui
                                            .add(
                                                forms::Btn::icon(crate::icons::ARROW_UP, "Move up")
                                                    .height(28.0),
                                            )
                                            .clicked()
                                    {
                                        up = Some(i);
                                    }
                                },
                            );
                        });
                    }
                    if let Some(i) = remove {
                        s.host_chain.remove(i);
                    }
                    if let Some(i) = up {
                        s.host_chain.swap(i, i - 1);
                    }
                    let width = ui.available_width();
                    forms::select(
                        ui,
                        &format!("{salt}-chain-add"),
                        width,
                        if s.host_chain.is_empty() {
                            "Connect directly"
                        } else {
                            "Add another jump host"
                        },
                        false,
                        |ui| {
                            let mut any = false;
                            for h in &self.vault.hosts {
                                if Some(h.id) != self_id
                                    && !s.host_chain.contains(&h.id)
                                    && h.protocol == Protocol::Ssh
                                {
                                    any = true;
                                    if forms::option(ui, false, &h.display_name()).clicked() {
                                        s.host_chain.push(h.id);
                                        ui.close();
                                    }
                                }
                            }
                            if !any {
                                ui.label(RichText::new("No other SSH hosts").color(pal.text_dim));
                            }
                        },
                    );
                });
                // Proxy
                let mut kind = s.proxy.as_ref().map(|p| p.kind);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    forms::label(ui, "Proxy", pal);
                    if forms::segmented(
                        ui,
                        &format!("{salt}-proxy"),
                        &mut kind,
                        &[
                            (None, "None"),
                            (Some(ProxyKind::Socks5), "SOCKS5"),
                            (Some(ProxyKind::Http), "HTTP"),
                            (Some(ProxyKind::Command), "Command"),
                        ],
                    ) {
                        match kind {
                            None => s.proxy = None,
                            Some(kind) => {
                                s.proxy
                                    .get_or_insert_with(|| ProxyConfig {
                                        port: 1080,
                                        ..Default::default()
                                    })
                                    .kind = kind;
                            }
                        }
                    }
                });
                let open = s.proxy.is_some();
                forms::reveal(ui, ui.scope_id().with((salt, "proxy-fields")), open, |ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;
                    let Some(p) = s.proxy.as_mut() else {
                        return;
                    };
                    if p.kind == ProxyKind::Command {
                        forms::Input::new("Command")
                            .hint("nc -X 5 -x proxy:1080 %h %p")
                            .mono()
                            .show(ui, &mut p.command);
                    } else {
                        forms::columns(ui, &[3.0, 1.0], |column, ui| {
                            if column == 0 {
                                forms::Input::new("Proxy host")
                                    .hint("127.0.0.1")
                                    .mono()
                                    .show(ui, &mut p.host);
                            } else {
                                forms::number(ui, forms::Input::new("Port"), &mut p.port);
                            }
                        });
                        forms::columns(ui, &[1.0, 1.0], |column, ui| {
                            if column == 0 {
                                forms::Input::new("Proxy user")
                                    .hint("Optional")
                                    .show(ui, &mut p.username);
                            } else {
                                forms::Input::new("Proxy password")
                                    .hint("Optional")
                                    .password()
                                    .show(ui, &mut p.password);
                            }
                        });
                    }
                });
            },
        );
        let behavior = format!(
            "{} s timeout, {}",
            s.connect_timeout_secs
                .unwrap_or(inherited.connect_timeout_secs),
            if s.strict_host_key_check
                .unwrap_or(inherited.strict_host_key_check)
            {
                "strict host keys"
            } else {
                "host keys not enforced"
            }
        );
        forms::section(
            ui,
            &format!("{salt}-behavior"),
            "Behavior",
            &behavior,
            false,
            |ui| {
                forms::columns(ui, &[1.0, 1.0], |column, ui| {
                    let from = format!("Inherited from {source}");
                    if column == 0 {
                        let hint = inherited.connect_timeout_secs.to_string();
                        let mut input = forms::Input::new("Connect timeout")
                            .hint(&hint)
                            .suffix("sec");
                        if s.connect_timeout_secs.is_none() {
                            input = input.help(&from);
                        }
                        forms::opt_number(ui, input, &mut s.connect_timeout_secs);
                    } else {
                        let hint = inherited.keepalive_secs.to_string();
                        let mut input = forms::Input::new("Keep-alive").hint(&hint).suffix("sec");
                        if s.keepalive_secs.is_none() {
                            input = input.help(&from);
                        }
                        forms::opt_number(ui, input, &mut s.keepalive_secs);
                    }
                });
                forms::inherit_row(
                    ui,
                    "Strict host key checking",
                    "Refuse hosts you have not trusted yet.",
                    &mut s.strict_host_key_check,
                    inherited.strict_host_key_check,
                    source,
                );
                forms::inherit_row(
                    ui,
                    "Forward SSH agent",
                    "Only enable this on servers you trust.",
                    &mut s.agent_forwarding,
                    inherited.agent_forwarding,
                    source,
                );
                let snippets: Vec<(Uuid, String)> = self
                    .vault
                    .snippets
                    .iter()
                    .map(|s| (s.id, s.name.clone()))
                    .collect();
                select_uuid(
                    ui,
                    "Startup snippet, runs after sign-in",
                    &mut s.startup_snippet_id,
                    &snippets,
                    "None",
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    forms::label(ui, "Environment variables", pal);
                    let mut rm = None;
                    for (i, (k, v)) in s.env.iter_mut().enumerate() {
                        ui.push_id(i, |ui| {
                            ui.horizontal(|ui| {
                                let width = (ui.available_width() - 48.0) / 2.0;
                                forms::Input::bare("Variable name")
                                    .hint("NAME")
                                    .mono()
                                    .width(width)
                                    .show(ui, k);
                                forms::Input::bare("Variable value")
                                    .hint("value")
                                    .mono()
                                    .width(width)
                                    .show(ui, v);
                                if ui
                                    .add(forms::Btn::icon(crate::icons::X, "Remove variable"))
                                    .clicked()
                                {
                                    rm = Some(i);
                                }
                            });
                        });
                    }
                    if let Some(i) = rm {
                        s.env.remove(i);
                    }
                    if ui
                        .add(
                            forms::Btn::ghost(format!("{}  Add variable", crate::icons::PLUS))
                                .small(),
                        )
                        .clicked()
                    {
                        s.env.push((String::new(), String::new()));
                    }
                });
            },
        );
        let terminal = s
            .color_scheme
            .clone()
            .unwrap_or_else(|| "Default colors".to_string());
        forms::section(
            ui,
            &format!("{salt}-terminal"),
            "Terminal",
            &terminal,
            false,
            |ui| {
                forms::columns(ui, &[1.0, 1.0], |column, ui| {
                    if column == 0 {
                        let mut options: Vec<(Option<String>, String)> =
                            vec![(None, format!("Use {}", inherited.charset))];
                        options.extend(
                            crate::term::CHARSETS
                                .iter()
                                .map(|c| (Some(c.to_string()), c.to_string())),
                        );
                        forms::select_field(ui, "Charset", &mut s.charset, &options);
                    } else {
                        let mut options: Vec<(Option<String>, String)> =
                            vec![(None, "Default colors".to_string())];
                        options.extend(
                            crate::themes::schemes()
                                .iter()
                                .map(|sc| (Some(sc.name.to_string()), sc.name.to_string())),
                        );
                        forms::select_field(ui, "Color scheme", &mut s.color_scheme, &options);
                    }
                });
                forms::columns(ui, &[1.0, 1.0], |column, ui| {
                    if column == 0 {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            forms::label(ui, "Backspace sends", pal);
                            let mut mode = s.backspace.unwrap_or(inherited.backspace);
                            if forms::segmented(
                                ui,
                                &format!("{salt}-backspace"),
                                &mut mode,
                                &[(BackspaceMode::Del, "DEL ^?"), (BackspaceMode::CtrlH, "^H")],
                            ) {
                                s.backspace = Some(mode);
                            }
                        });
                    } else {
                        forms::opt_number(
                            ui,
                            forms::Input::new("Font size").hint("Default"),
                            &mut s.font_size,
                        );
                    }
                });
            },
        );
    }

    fn group_form(&mut self, ui: &mut egui::Ui, g: &mut Group, pal: Palette) {
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
        let fake = Host {
            group_id: g.parent_id,
            ..Default::default()
        };
        let inherited = self.vault.resolve(&fake);
        let source = if g.parent_id.is_some() {
            self.vault.group_path(g.parent_id)
        } else {
            "connection defaults".to_string()
        };
        forms::card(ui, |ui| {
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new("Name")
                        .hint("Production")
                        .show(ui, &mut g.name);
                } else {
                    select_uuid(ui, "Parent group", &mut g.parent_id, &parents, "Top level");
                }
            });
            forms::opt_number(
                ui,
                forms::Input::new("Port").hint("22").width(120.0),
                &mut g.settings.port,
            );
            forms::opt_text(
                ui,
                forms::Input::new("Username").hint(if inherited.username.is_empty() {
                    "root"
                } else {
                    &inherited.username
                }),
                &mut g.settings.username,
            );
            forms::help(
                ui,
                "Hosts in this group inherit these settings unless they set their own.",
                pal,
            );
        });
        self.credentials_section(ui, &mut g.settings, &inherited, &source, pal, "group");
        self.advanced_sections(ui, &mut g.settings, &inherited, None, &source, pal, "group");
    }

    fn identity_form(&mut self, ui: &mut egui::Ui, i: &mut Identity, pal: Palette) {
        forms::card(ui, |ui| {
            forms::help(
                ui,
                "A reusable username with a password or key that many hosts can share.",
                pal,
            );
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new("Label").show(ui, &mut i.label);
                } else {
                    forms::Input::new("Username")
                        .mono()
                        .show(ui, &mut i.username);
                }
            });
            forms::Input::new("Password")
                .password()
                .show(ui, &mut i.password);
            let keys: Vec<(Uuid, String)> = self
                .vault
                .keys
                .iter()
                .map(|k| (k.id, k.label.clone()))
                .collect();
            select_uuid(ui, "Key", &mut i.key_id, &keys, "None");
        });
    }

    fn key_form(&mut self, ui: &mut egui::Ui, k: &mut SshKey, is_new: bool, pal: Palette) {
        forms::card(ui, |ui| {
            forms::Input::new("Label").show(ui, &mut k.label);
            if !k.key_type.is_empty() || !k.public_key.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    if !k.key_type.is_empty() {
                        forms::chip(ui, &k.key_type, true, pal);
                    }
                    if !k.public_key.is_empty() {
                        ui.label(
                            RichText::new(crate::keys::fingerprint_of_public(&k.public_key))
                                .monospace()
                                .size(12.0)
                                .color(pal.text_dim),
                        );
                    }
                });
            }
            forms::text_area(ui, "Public key", &mut k.public_key, 2, true, "");
            ui.horizontal(|ui| {
                if ui
                    .add(
                        forms::Btn::secondary(format!("{}  Copy public key", crate::icons::COPY))
                            .small(),
                    )
                    .clicked()
                {
                    ui.ctx().copy_text(k.public_key.clone());
                }
                if !is_new
                    && ui
                        .add(
                            forms::Btn::secondary(format!(
                                "{}  Install on host…",
                                crate::icons::ARROW_UP
                            ))
                            .small(),
                        )
                        .clicked()
                {
                    self.dialog = Some(Dialog::InstallKey {
                        key_id: k.id,
                        host_id: None,
                        busy: false,
                    });
                }
            });
        });
        let id = ui.make_persistent_id(("show-priv", k.id));
        let mut show = ui.data_mut(|d| *d.get_temp_mut_or(id, is_new));
        forms::section(
            ui,
            "private",
            "Private key",
            "Stored encrypted in your vault",
            is_new,
            |ui| {
                if forms::switch_row(ui, "Show private key", "", &mut show).changed() {
                    ui.data_mut(|d| d.insert_temp(id, show));
                }
                forms::reveal(ui, id.with("body"), show, |ui| {
                    let r = forms::text_area(
                        ui,
                        "",
                        &mut k.private_key,
                        6,
                        true,
                        "-----BEGIN OPENSSH PRIVATE KEY-----",
                    );
                    if r.changed() {
                        if let Ok(parsed) =
                            crate::keys::import(&k.label, &k.private_key, &k.passphrase)
                        {
                            k.public_key = parsed.public_key;
                            k.key_type = parsed.key_type;
                        }
                    }
                });
                forms::Input::new("Passphrase")
                    .hint("Optional")
                    .password()
                    .show(ui, &mut k.passphrase);
                forms::text_area(
                    ui,
                    "OpenSSH certificate, optional",
                    &mut k.certificate,
                    2,
                    true,
                    "ssh-ed25519-cert-v01@openssh.com AAAA…",
                );
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            forms::Btn::secondary(format!(
                                "{}  Export private…",
                                crate::icons::FLOPPY_DISK
                            ))
                            .small(),
                        )
                        .clicked()
                    {
                        if let Some(p) = rfd::FileDialog::new()
                            .set_file_name("id_ferrum")
                            .save_file()
                        {
                            let r = std::fs::write(&p, &k.private_key);
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let _ = std::fs::set_permissions(
                                    &p,
                                    std::fs::Permissions::from_mode(0o600),
                                );
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
                    if ui
                        .add(
                            forms::Btn::secondary(format!(
                                "{}  Export public…",
                                crate::icons::FLOPPY_DISK
                            ))
                            .small(),
                        )
                        .clicked()
                    {
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
            },
        );
    }

    fn forward_form(&mut self, ui: &mut egui::Ui, f: &mut PortForward, pal: Palette) {
        forms::card(ui, |ui| {
            forms::Input::new("Label")
                .hint("Postgres tunnel")
                .show(ui, &mut f.label);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                forms::label(ui, "Direction", pal);
                forms::segmented(
                    ui,
                    "forward-kind",
                    &mut f.kind,
                    &[
                        (ForwardKind::Local, "Local"),
                        (ForwardKind::Remote, "Remote"),
                        (ForwardKind::Dynamic, "Dynamic"),
                    ],
                );
                forms::help(
                    ui,
                    match f.kind {
                        ForwardKind::Local => "Connections to the bind address on this machine go through the host to the destination.",
                        ForwardKind::Remote => "Connections to the bind address on the host come back to a destination this machine can reach.",
                        ForwardKind::Dynamic => "A local SOCKS4/5 proxy that routes any destination through the host.",
                    },
                    pal,
                );
            });
            let hosts: Vec<(Uuid, String)> = self
                .vault
                .hosts
                .iter()
                .filter(|h| h.protocol == Protocol::Ssh)
                .map(|h| (h.id, h.display_name()))
                .collect();
            select_uuid(ui, "Through host", &mut f.host_id, &hosts, "Choose a host");
            forms::columns(ui, &[3.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new(if f.kind == ForwardKind::Remote {
                        "Remote bind address"
                    } else {
                        "Bind address"
                    })
                    .mono()
                    .show(ui, &mut f.bind_address);
                } else {
                    forms::number(ui, forms::Input::new("Port").salt("bind"), &mut f.bind_port);
                }
            });
            forms::reveal(
                ui,
                egui::Id::unique("forward-destination"),
                f.kind != ForwardKind::Dynamic,
                |ui| {
                    forms::columns(ui, &[3.0, 1.0], |column, ui| {
                        if column == 0 {
                            forms::Input::new("Destination host")
                                .mono()
                                .show(ui, &mut f.dest_host);
                        } else {
                            forms::number(
                                ui,
                                forms::Input::new("Port").salt("destination"),
                                &mut f.dest_port,
                            );
                        }
                    });
                },
            );
            forms::switch_row(
                ui,
                "Start automatically",
                "Opens this tunnel whenever you connect to the host.",
                &mut f.auto_start,
            );
        });
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
                format!("{}  {}", crate::icons::KEYBOARD, s.hotkey)
            };
            if ui.add(forms::Btn::secondary(label)).clicked() {
                recording = !recording;
            }
            if !s.hotkey.is_empty() && ui.add(forms::Btn::ghost("Clear").small()).clicked() {
                s.hotkey.clear();
            }
        });
        ui.memory_mut(|m| m.data.insert_temp(rec_id, recording));
        if recording {
            ui.ctx().request_repaint();
        }
        if let Some((m, k)) = crate::hotkeys::parse(&s.hotkey) {
            if crate::hotkeys::is_reserved(m, k) {
                forms::error_line(ui, "Ferrum already uses this shortcut", pal);
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
        forms::card(ui, |ui| {
            forms::columns(ui, &[1.0, 1.0], |column, ui| {
                if column == 0 {
                    forms::Input::new("Name").show(ui, &mut s.name);
                } else {
                    let mut known: Vec<String> = self
                        .vault
                        .snippets
                        .iter()
                        .map(|x| x.package.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect();
                    known.sort();
                    known.dedup();
                    ui.horizontal_top(|ui| {
                        let width = if known.is_empty() {
                            ui.available_width()
                        } else {
                            ui.available_width() - 44.0
                        };
                        forms::Input::new("Package")
                            .hint("Docker, Deploy")
                            .width(width)
                            .show(ui, &mut s.package);
                        if !known.is_empty() {
                            ui.vertical(|ui| {
                                ui.add_space(22.0);
                                let shown = "";
                                forms::select(ui, "snippet-package", 36.0, shown, false, |ui| {
                                    ui.set_min_width(180.0);
                                    for p in &known {
                                        if forms::option(ui, s.package == *p, p).clicked() {
                                            s.package = p.clone();
                                            ui.close();
                                        }
                                    }
                                })
                                .on_hover_text("Use an existing package");
                            });
                        }
                    });
                }
            });
            forms::text_area(
                ui,
                "Script",
                &mut s.script,
                8,
                true,
                "sudo systemctl restart {{service}}",
            );
            forms::help(
                ui,
                "Use {{variable}} placeholders and you'll be asked for values when running.",
                pal,
            );
            forms::Input::new("Description").show(ui, &mut s.description);
            forms::tag_input(ui, "Tags", &mut s.tags);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                forms::label(ui, "Hotkey, runs it in the active terminal", pal);
                self.hotkey_field(ui, s, pal);
            });
        });
    }
}

pub fn serial_form(ui: &mut egui::Ui, cfg: &mut SerialConfig, pal: Palette) {
    let ports = crate::conn::serial::list_ports();
    let baud_options: Vec<(u32, String)> = crate::conn::serial::BAUD_RATES
        .iter()
        .map(|b| (*b, b.to_string()))
        .collect();
    forms::columns(ui, &[2.0, 1.0], |column, ui| {
        if column == 0 {
            forms::Input::new("Device")
                .hint("/dev/ttyUSB0 or COM3")
                .mono()
                .show(ui, &mut cfg.device);
            if !ports.is_empty() {
                let width = ui.available_width();
                forms::select(ui, "detected-ports", width, "Detected ports", true, |ui| {
                    for port in &ports {
                        if forms::option(ui, cfg.device == *port, port).clicked() {
                            cfg.device = port.clone();
                            ui.close();
                        }
                    }
                });
            }
        } else {
            forms::select_field(ui, "Baud rate", &mut cfg.baud_rate, &baud_options);
        }
    });
    forms::columns(ui, &[1.0, 1.0], |column, ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if column == 0 {
                forms::label(ui, "Data bits", pal);
                forms::segmented(
                    ui,
                    "data-bits",
                    &mut cfg.data_bits,
                    &[(5u8, "5"), (6, "6"), (7, "7"), (8, "8")],
                );
            } else {
                forms::label(ui, "Stop bits", pal);
                forms::segmented(ui, "stop-bits", &mut cfg.stop_bits, &[(1u8, "1"), (2, "2")]);
            }
        });
    });
    forms::columns(ui, &[1.0, 1.0], |column, ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if column == 0 {
                forms::label(ui, "Parity", pal);
                forms::segmented(
                    ui,
                    "parity",
                    &mut cfg.parity,
                    &[
                        (Parity::None, "None"),
                        (Parity::Odd, "Odd"),
                        (Parity::Even, "Even"),
                    ],
                );
            } else {
                forms::label(ui, "Flow control", pal);
                forms::segmented(
                    ui,
                    "flow",
                    &mut cfg.flow_control,
                    &[
                        (FlowControl::None, "None"),
                        (FlowControl::Software, "XON/XOFF"),
                        (FlowControl::Hardware, "RTS/CTS"),
                    ],
                );
            }
        });
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        forms::label(ui, "Enter sends", pal);
        forms::segmented(
            ui,
            "line-ending",
            &mut cfg.line_ending,
            &[
                ("\r".to_string(), "CR"),
                ("\n".to_string(), "LF"),
                ("\r\n".to_string(), "CRLF"),
            ],
        );
    });
    forms::switch_row(
        ui,
        "Local echo",
        "Show what you type before the device echoes it.",
        &mut cfg.local_echo,
    );
}

fn address_error_key() -> egui::Id {
    egui::Id::unique("editor-address-error")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(h: &Host) -> ResolvedTarget {
        Vault::default().resolve(h)
    }

    fn ssh_host() -> Host {
        let mut h = Host {
            address: "10.0.1.12".into(),
            ..Default::default()
        };
        h.settings.username = Some("deploy".into());
        h
    }

    #[test]
    fn plain_ssh_omits_the_default_port() {
        let h = ssh_host();
        assert_eq!(equivalent_command(&h, &target(&h)), "ssh deploy@10.0.1.12");
    }

    #[test]
    fn ssh_flags_follow_the_draft_and_never_show_secrets() {
        let mut h = ssh_host();
        h.settings.port = Some(2222);
        h.settings.agent_forwarding = Some(true);
        h.settings.password = Some("hunter2".into());
        h.settings.proxy = Some(ProxyConfig {
            kind: ProxyKind::Socks5,
            host: "127.0.0.1".into(),
            port: 1080,
            password: "proxy-secret".into(),
            ..Default::default()
        });
        let command = equivalent_command(&h, &target(&h));
        assert_eq!(
            command,
            "ssh -p 2222 -A -o ProxyCommand=\"nc -X 5 -x 127.0.0.1:1080 %h %p\" deploy@10.0.1.12"
        );
        assert!(!command.contains("hunter2"));
        assert!(!command.contains("proxy-secret"));
    }

    #[test]
    fn missing_address_shows_a_placeholder() {
        let h = Host::default();
        assert!(equivalent_command(&h, &target(&h)).ends_with("@<host>"));
    }

    #[test]
    fn other_protocols_use_their_own_command() {
        let mut mosh = ssh_host();
        mosh.protocol = Protocol::Mosh;
        mosh.settings.port = Some(2200);
        assert_eq!(
            equivalent_command(&mosh, &target(&mosh)),
            "mosh --ssh=\"ssh -p 2200\" deploy@10.0.1.12"
        );
        let telnet = Host {
            protocol: Protocol::Telnet,
            address: "router".into(),
            ..Default::default()
        };
        assert_eq!(
            equivalent_command(&telnet, &target(&telnet)),
            "telnet router 23"
        );
        let mut serial = Host {
            protocol: Protocol::Serial,
            ..Default::default()
        };
        serial.serial.device = "/dev/ttyUSB0".into();
        serial.serial.baud_rate = 115_200;
        assert_eq!(
            equivalent_command(&serial, &target(&serial)),
            "screen /dev/ttyUSB0 115200"
        );
    }
}
