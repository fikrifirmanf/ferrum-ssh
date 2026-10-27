//! Saved workspaces (restore a set of tabs in one click) and tab duplication.

use uuid::Uuid;

use crate::app::{App, Pane, SplitLayout, Tab, TermTab};
use crate::conn;
use crate::model::{Protocol, Workspace, WorkspaceTab, now_ts};
use crate::ui::term_view::ViewState;

fn layout_to_u8(l: SplitLayout) -> u8 {
    match l {
        SplitLayout::Horizontal => 0,
        SplitLayout::Vertical => 1,
        SplitLayout::Grid => 2,
    }
}

fn layout_from_u8(n: u8) -> SplitLayout {
    match n {
        1 => SplitLayout::Vertical,
        2 => SplitLayout::Grid,
        _ => SplitLayout::Horizontal,
    }
}

impl App {
    /// Describe the open tabs. Returns the tabs plus how many panes could not be
    /// captured (quick-connect or serial sessions that aren't saved hosts).
    pub fn capture_tabs(&self) -> (Vec<WorkspaceTab>, usize) {
        let mut skipped = 0;
        let mut out = vec![];
        for t in &self.tabs {
            match t {
                Tab::Terminal(tt) => {
                    let mut hosts = vec![];
                    for p in &tt.panes {
                        let target = &p.session.target;
                        if target.protocol == Protocol::Local {
                            hosts.push(None);
                        } else if let Some(id) =
                            target.host_id.filter(|id| self.vault.host(*id).is_some())
                        {
                            hosts.push(Some(id));
                        } else {
                            skipped += 1;
                        }
                    }
                    if !hosts.is_empty() {
                        out.push(WorkspaceTab {
                            hosts,
                            layout: layout_to_u8(tt.layout),
                            sftp: false,
                            title: tt.custom_title.clone(),
                        });
                    }
                }
                Tab::Sftp(st) => {
                    let host = st.right.host_id.or(st.left.host_id);
                    out.push(WorkspaceTab {
                        hosts: vec![host],
                        layout: 0,
                        sftp: true,
                        title: None,
                    });
                }
            }
        }
        (out, skipped)
    }

    pub fn save_workspace(&mut self, name: &str) -> bool {
        let (tabs, skipped) = self.capture_tabs();
        if tabs.is_empty() {
            self.toast("Open at least one saved host or local terminal first", true);
            return false;
        }
        let name = name.trim();
        let name = if name.is_empty() { "Workspace" } else { name };
        self.vault.workspaces.push(Workspace {
            id: Uuid::new_v4(),
            name: name.to_string(),
            tabs,
            updated: now_ts(),
        });
        self.mark_dirty();
        let msg = if skipped > 0 {
            format!("Saved “{name}” ({skipped} unsaved sessions left out)")
        } else {
            format!("Saved workspace “{name}”")
        };
        self.toast(msg, false);
        true
    }

    pub fn open_workspace(&mut self, id: Uuid) {
        let Some(ws) = self.vault.workspaces.iter().find(|w| w.id == id).cloned() else {
            return;
        };
        let mut missing = 0;
        let mut last = None;
        for tab in &ws.tabs {
            if tab.sftp {
                match tab.hosts.first().copied().flatten() {
                    Some(h) if self.vault.host(h).is_none() => missing += 1,
                    host => self.open_sftp(host),
                }
                last = self.tabs.last().map(|t| t.id());
                continue;
            }
            let mut panes: Vec<Pane> = vec![];
            for host in &tab.hosts {
                let target = match host {
                    None => self.local_target(),
                    Some(h) => match self.vault.host(*h).cloned() {
                        Some(host) => self.vault.resolve(&host),
                        None => {
                            missing += 1;
                            continue;
                        }
                    },
                };
                let session = self.new_session(target);
                panes.push(Pane {
                    session,
                    view: ViewState::default(),
                });
            }
            if panes.is_empty() {
                continue;
            }
            let id = conn::next_id();
            self.tabs.push(Tab::Terminal(TermTab {
                id,
                active: panes.len() - 1,
                panes,
                layout: layout_from_u8(tab.layout),
                broadcast: false,
                side_panel: None,
                ai_query: String::new(),
                ai_result: None,
                ai_busy: false,
                snippet_filter: String::new(),
                containers: Default::default(),
                custom_title: tab.title.clone(),
                health: Default::default(),
            }));
            last = Some(id);
        }
        if last.is_some() {
            self.active_tab = last;
        }
        if missing > 0 {
            self.toast(
                format!("{missing} hosts in “{}” no longer exist", ws.name),
                true,
            );
        }
    }

    /// Open a copy of a tab with fresh connections to the same targets.
    pub fn duplicate_tab(&mut self, id: u64) {
        let Some(tab) = self.tabs.iter().find(|t| t.id() == id) else {
            return;
        };
        match tab {
            Tab::Terminal(tt) => {
                let targets: Vec<_> = tt.panes.iter().map(|p| p.session.target.clone()).collect();
                let (layout, title, active) = (tt.layout, tt.custom_title.clone(), tt.active);
                let panes: Vec<Pane> = targets
                    .into_iter()
                    .map(|t| Pane {
                        session: self.new_session(t),
                        view: ViewState::default(),
                    })
                    .collect();
                let new_id = conn::next_id();
                self.tabs.push(Tab::Terminal(TermTab {
                    id: new_id,
                    active: active.min(panes.len().saturating_sub(1)),
                    panes,
                    layout,
                    broadcast: false,
                    side_panel: None,
                    ai_query: String::new(),
                    ai_result: None,
                    ai_busy: false,
                    snippet_filter: String::new(),
                    containers: Default::default(),
                    custom_title: title,
                    health: Default::default(),
                }));
                self.active_tab = Some(new_id);
            }
            Tab::Sftp(st) => {
                let host = st.right.host_id.or(st.left.host_id);
                self.open_sftp(host);
            }
        }
    }
}
