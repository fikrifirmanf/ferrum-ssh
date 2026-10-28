#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod app;
mod biometric;
mod cloud;
mod conn;
mod containers;
mod fonts;
mod forward;
mod health;
mod hotkeys;
mod icons;
mod importers;
mod instance;
#[cfg(test)]
mod it_tests;
mod keys;
mod model;
#[cfg(target_os = "macos")]
mod native;
mod qa;
mod sftp;
mod store;
mod suggest;
mod term;
mod themes;
mod ui;
mod updates;
mod workspaces;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // `ferrum ssh://user@host:22` opens a quick-connect tab on launch.
    let deep_link = std::env::args().nth(1);

    // A second launch focuses the running window, passing on its deep link, and exits.
    // QA runs are disposable and may run beside a real instance.
    let instance = if std::env::var_os("FERRUM_QA").is_some() {
        None
    } else {
        match instance::claim(&store::data_dir(), deep_link.as_deref()) {
            instance::Claim::Primary(guard) => Some(guard),
            instance::Claim::Handed => return Ok(()),
            instance::Claim::Unguarded => None,
        }
    };

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Ferrum")
        .with_app_id("ferrum-ssh")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([760.0, 480.0]);
    if std::env::var("FERRUM_QA")
        .is_ok_and(|steps| !steps.split(',').any(|step| step.trim() == "interactive"))
    {
        viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnTop);
    }
    if let Some(icon) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    #[cfg(target_os = "macos")]
    {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_transparent(true);
    }
    let options = eframe::NativeOptions {
        viewport,
        persist_window: std::env::var_os("FERRUM_QA").is_none(),
        ..Default::default()
    };

    eframe::run_native(
        "Ferrum",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, deep_link, instance)))),
    )
}
