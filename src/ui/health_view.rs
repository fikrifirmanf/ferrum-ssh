//! Side panel showing live CPU, memory, disk and process figures for the active SSH session.

use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::card_frame;
use crate::health::{Monitor, Snapshot, format_bytes_kb, format_uptime};
use crate::themes::Palette;

/// What the panel can show for the active pane.
pub enum Source<'a> {
    /// A live SSH session labelled with its host name.
    Ssh(&'a str),
    /// The pane is not an SSH session (local shell, serial, Telnet ...).
    Unsupported,
    /// An SSH pane that is not connected right now.
    Disconnected(&'a str),
}

pub fn show(ui: &mut egui::Ui, pal: Palette, monitor: &Monitor, source: Source) {
    ui.label(
        RichText::new(format!("{} Server health", crate::icons::MONITOR))
            .strong()
            .size(16.0),
    );
    let host = match source {
        Source::Ssh(host) => host,
        Source::Unsupported => {
            dim(ui, pal, "Health is available for SSH connections.");
            return;
        }
        Source::Disconnected(host) => {
            dim(ui, pal, &format!("{host} is not connected."));
            return;
        }
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(host).small().color(pal.text_dim));
        let os = monitor
            .snapshot
            .as_ref()
            .map(|s| s.os.as_str())
            .filter(|os| !os.is_empty());
        if let Some(os) = os {
            ui.label(RichText::new(format!("· {os}")).small().color(pal.text_dim));
        }
        if let Some(updated) = monitor.updated {
            ui.label(
                RichText::new(format!("· updated {}s ago", updated.elapsed().as_secs()))
                    .small()
                    .color(pal.text_dim),
            );
        }
    });
    if let Some(error) = &monitor.error {
        ui.colored_label(pal.warning, format!("Last update failed: {error}"));
    }
    let Some(snapshot) = &monitor.snapshot else {
        if monitor.error.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                dim(ui, pal, "Collecting…");
            });
        }
        return;
    };
    ui.add_space(6.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            cpu_card(ui, pal, snapshot, &monitor.cpu_history);
            memory_card(ui, pal, snapshot, &monitor.mem_history);
            system_card(ui, pal, snapshot);
            if !snapshot.disks.is_empty() {
                disks_card(ui, pal, snapshot);
            }
            if !snapshot.processes.is_empty() {
                processes_card(ui, pal, snapshot);
            }
        });
}

fn dim(ui: &mut egui::Ui, pal: Palette, text: &str) {
    ui.label(RichText::new(text).color(pal.text_dim));
}

fn level_color(pal: Palette, percent: f32) -> Color32 {
    if percent >= 90.0 {
        pal.danger
    } else if percent >= 75.0 {
        pal.warning
    } else {
        pal.accent
    }
}

fn card(ui: &mut egui::Ui, pal: Palette, add: impl FnOnce(&mut egui::Ui)) {
    card_frame(pal).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(8.0);
}

fn heading(ui: &mut egui::Ui, pal: Palette, title: &str, value: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).small().color(pal.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).strong().size(18.0).color(color));
        });
    });
}

fn bar(ui: &mut egui::Ui, pal: Palette, percent: f32) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(3), pal.bg);
    let width = rect.width() * (percent / 100.0).clamp(0.0, 1.0);
    if width > 0.0 {
        let filled = Rect::from_min_size(rect.min, Vec2::new(width.max(3.0), rect.height()));
        ui.painter()
            .rect_filled(filled, CornerRadius::same(3), level_color(pal, percent));
    }
    response.on_hover_text(format!("{percent:.0}%"));
}

fn sparkline(ui: &mut egui::Ui, pal: Palette, history: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(6), pal.bg);
    if history.len() < 2 {
        return;
    }
    let step = rect.width() / (crate::health::HISTORY - 1) as f32;
    let left = rect.right() - step * (history.len() - 1) as f32;
    let points: Vec<Pos2> = history
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let y = rect.bottom() - 3.0 - (rect.height() - 6.0) * (v / 100.0).clamp(0.0, 1.0);
            Pos2::new(left + step * i as f32, y)
        })
        .collect();
    let latest = history.last().copied().unwrap_or(0.0);
    ui.painter().add(egui::Shape::line(
        points,
        Stroke::new(1.5, level_color(pal, latest)),
    ));
}

fn cpu_card(ui: &mut egui::Ui, pal: Palette, s: &Snapshot, history: &[f32]) {
    // Without CPU counters (anything but Linux) the load average is all there is.
    if s.cpu_times.is_none() && s.load.is_none() {
        return;
    }
    card(ui, pal, |ui| {
        match (s.cpu_times, s.cpu_percent, s.load) {
            (Some(_), Some(cpu), _) => {
                heading(ui, pal, "CPU", &format!("{cpu:.0}%"), level_color(pal, cpu));
                bar(ui, pal, cpu);
            }
            (Some(_), None, _) => heading(ui, pal, "CPU", "…", pal.text_dim),
            (None, _, Some([one, ..])) => {
                heading(ui, pal, "Load average", &format!("{one:.2}"), pal.text)
            }
            (None, _, None) => {}
        }
        if let Some([one, five, fifteen]) = s.load {
            ui.label(
                RichText::new(format!("{one:.2}  {five:.2}  {fifteen:.2}  (1, 5, 15 min)"))
                    .small()
                    .color(pal.text_dim),
            );
        }
        if !history.is_empty() {
            sparkline(ui, pal, history);
        }
    });
}

fn memory_card(ui: &mut egui::Ui, pal: Palette, s: &Snapshot, history: &[f32]) {
    let (Some(total), Some(used)) = (s.mem_total_kb, s.mem_used_kb) else {
        return;
    };
    let percent = s.mem_percent().unwrap_or(0.0);
    card(ui, pal, |ui| {
        heading(
            ui,
            pal,
            "Memory",
            &format!("{percent:.0}%"),
            level_color(pal, percent),
        );
        bar(ui, pal, percent);
        ui.label(
            RichText::new(format!(
                "{} of {}",
                format_bytes_kb(used),
                format_bytes_kb(total)
            ))
            .small()
            .color(pal.text_dim),
        );
        if let (Some(swap_total), Some(swap_used)) = (s.swap_total_kb, s.swap_used_kb) {
            ui.label(
                RichText::new(format!(
                    "Swap {} of {}",
                    format_bytes_kb(swap_used),
                    format_bytes_kb(swap_total)
                ))
                .small()
                .color(pal.text_dim),
            );
        }
        if history.len() > 1 {
            sparkline(ui, pal, history);
        }
    });
}

fn system_card(ui: &mut egui::Ui, pal: Palette, s: &Snapshot) {
    if let Some(uptime) = s.uptime_secs {
        card(ui, pal, |ui| {
            heading(ui, pal, "Uptime", &format_uptime(uptime), pal.text)
        });
    }
}

fn disks_card(ui: &mut egui::Ui, pal: Palette, s: &Snapshot) {
    card(ui, pal, |ui| {
        ui.label(RichText::new("Disks").small().color(pal.text_dim));
        for disk in &s.disks {
            ui.add_space(4.0);
            heading(
                ui,
                pal,
                &disk.mount,
                &format!("{:.0}%", disk.percent()),
                level_color(pal, disk.percent()),
            );
            bar(ui, pal, disk.percent());
            ui.label(
                RichText::new(format!(
                    "{} of {}",
                    format_bytes_kb(disk.used_kb),
                    format_bytes_kb(disk.total_kb)
                ))
                .small()
                .color(pal.text_dim),
            );
        }
    });
}

fn processes_card(ui: &mut egui::Ui, pal: Palette, s: &Snapshot) {
    card(ui, pal, |ui| {
        ui.label(RichText::new("Top processes").small().color(pal.text_dim));
        egui::Grid::new("health-processes")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new("CPU").small().color(pal.text_dim));
                ui.label(RichText::new("MEM").small().color(pal.text_dim));
                ui.label(RichText::new("Command").small().color(pal.text_dim));
                ui.end_row();
                for p in &s.processes {
                    ui.label(RichText::new(format!("{:.1}%", p.cpu)).monospace().small());
                    ui.label(RichText::new(format!("{:.1}%", p.mem)).monospace().small());
                    ui.label(RichText::new(&p.name).monospace().small())
                        .on_hover_text(format!("pid {}", p.pid));
                    ui.end_row();
                }
            });
    });
}
