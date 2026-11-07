//! Motion: three durations and one curve for fades, plus embun springs for anything glass
//! that moves (lenses, presses, the heat glow), so travel feels like liquid.
//!
//! Terminal text, connection state, host-key warnings and transfer progress never animate.
//! With Reduce motion on, every helper here lands on its final value at once.

use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Context, Id};

/// Hover and press color changes.
pub const FAST: f32 = 0.12;
/// Switches, segments, chevrons, selection markers.
pub const CONTROL: f32 = 0.18;
/// Focus heat line and ring.
pub const HEAT: f32 = 0.22;
/// Sections, inspector, navigation, side panels.
pub const PANEL: f32 = 0.24;
/// Closing runs quicker than opening.
pub const EXIT: f32 = 0.14;

static SYSTEM_REDUCED: AtomicBool = AtomicBool::new(false);

fn flag_id() -> Id {
    Id::unique("forge-reduce-motion")
}

/// Reads the operating system's preference once, away from the UI thread.
pub fn detect_system_preference() {
    std::thread::spawn(|| {
        SYSTEM_REDUCED.store(system_prefers_reduced(), Ordering::Relaxed);
    });
}

#[cfg(target_os = "macos")]
fn system_prefers_reduced() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}

#[cfg(target_os = "linux")]
fn system_prefers_reduced() -> bool {
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn system_prefers_reduced() -> bool {
    false
}

pub fn set_user_preference(ctx: &Context, reduce: bool) {
    let reduce = reduce || SYSTEM_REDUCED.load(Ordering::Relaxed);
    if ctx.data(|d| d.get_temp::<bool>(flag_id())) != Some(reduce) {
        ctx.data_mut(|d| d.insert_temp(flag_id(), reduce));
        ctx.global_style_mut(|s| s.animation_time = if reduce { 0.0 } else { CONTROL });
    }
}

pub fn reduced(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(flag_id()).unwrap_or(false))
}

/// Eased 0..1 toward `on`. The curve flips on the way back, so both directions feel the same.
pub fn ease(ctx: &Context, id: Id, on: bool, secs: f32) -> f32 {
    let secs = if reduced(ctx) { 0.0 } else { secs };
    ctx.animate_bool_with_time_and_easing(id, on, secs, emath::easing::cubic_out)
}

/// Same as [`ease`], closing at [`EXIT`] speed.
pub fn ease_panel(ctx: &Context, id: Id, on: bool) -> f32 {
    ease(ctx, id, on, if on { PANEL } else { EXIT })
}

/// Eased approach to a moving number, such as a segmented thumb position.
pub fn value(ctx: &Context, id: Id, target: f32, secs: f32) -> f32 {
    let secs = if reduced(ctx) { 0.0 } else { secs };
    ctx.animate_value_with_time(id, target, secs)
}

/// Eased 0..1 since `key` last changed; 1 on first sight, so opening a screen does not replay.
pub fn since_change(
    ctx: &Context,
    id: Id,
    key: impl std::hash::Hash + std::fmt::Debug,
    secs: f32,
) -> f32 {
    if reduced(ctx) {
        return 1.0;
    }
    let key = Id::unique(key).value();
    let now = ctx.input(|i| i.time);
    let (seen, start) = ctx.data_mut(|d| {
        let entry = d.get_temp_mut_or_insert_with(id, || (key, now - secs as f64));
        if entry.0 != key {
            *entry = (key, now);
        }
        *entry
    });
    debug_assert_eq!(seen, key);
    let t = ((now - start) as f32 / secs).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    emath::easing::cubic_out(t)
}

// ---------- Liquid springs ----------

pub use embun_motion::Spring;

/// Glass that moves: lenses, selection pills, the heat glow. Lands with a soft overshoot.
pub const LIQUID: Spring = Spring {
    response: 0.42,
    damping_fraction: 0.74,
};
/// Hover and press on glass: quick, no overshoot.
pub const TOUCH: Spring = Spring {
    response: 0.22,
    damping_fraction: 1.0,
};
/// Panels and sections opening.
pub const PANEL_SPRING: Spring = Spring {
    response: 0.36,
    damping_fraction: 0.9,
};

#[derive(Clone, Copy)]
struct SpringState {
    anim: embun_motion::Animated,
    time: f64,
}

/// A spring toward `target`. Returns the current value and its velocity per second.
/// The first call starts at the target, so opening a screen does not replay motion.
pub fn spring_with_velocity(ctx: &Context, id: Id, target: f32, spring: Spring) -> (f32, f32) {
    let now = ctx.input(|i| i.time);
    if reduced(ctx) {
        ctx.data_mut(|d| d.remove::<SpringState>(id));
        return (target, 0.0);
    }
    let state = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_insert_with(id, || SpringState {
            anim: embun_motion::Animated::new(target, spring),
            time: now,
        });
        s.anim.target = target;
        s.anim.spring = spring;
        let dt = (now - s.time).clamp(0.0, 1.0 / 20.0) as f32;
        s.time = now;
        s.anim.step(dt);
        *s
    });
    if !state.anim.is_settled() {
        ctx.request_repaint();
    }
    (state.anim.value, state.anim.velocity)
}

/// The value of a spring toward `target`. See [`spring_with_velocity`].
pub fn spring(ctx: &Context, id: Id, target: f32, spring: Spring) -> f32 {
    spring_with_velocity(ctx, id, target, spring).0
}

/// 0..1 toward `on` on a spring.
pub fn spring_bool(ctx: &Context, id: Id, on: bool, s: Spring) -> f32 {
    spring(ctx, id, if on { 1.0 } else { 0.0 }, s)
}

/// A rectangle that glides to `target` like liquid: it stretches along its travel and
/// squeezes across it, in proportion to speed, then settles back to `target`.
pub fn liquid_rect(ctx: &Context, id: Id, target: egui::Rect) -> egui::Rect {
    let (x, vx) = spring_with_velocity(ctx, id.with("x"), target.center().x, LIQUID);
    let (y, vy) = spring_with_velocity(ctx, id.with("y"), target.center().y, LIQUID);
    let w = spring(ctx, id.with("w"), target.width(), LIQUID);
    let h = spring(ctx, id.with("h"), target.height(), LIQUID);
    let stretch = |v: f32| (v.abs() / 2400.0).min(0.18);
    let (sx, sy) = (stretch(vx), stretch(vy));
    let size = egui::vec2(w * (1.0 + sx - sy * 0.5), h * (1.0 + sy - sx * 0.5));
    egui::Rect::from_center_size(egui::pos2(x, y), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn springs_start_at_the_target_and_snap_with_reduce_motion() {
        let ctx = Context::default();
        let id = Id::unique("s");
        assert_eq!(spring(&ctx, id, 5.0, LIQUID), 5.0);
        set_user_preference(&ctx, true);
        assert_eq!(spring(&ctx, id, 9.0, LIQUID), 9.0);
    }

    #[test]
    fn reduce_motion_lands_on_the_final_value_at_once() {
        let ctx = Context::default();
        set_user_preference(&ctx, true);
        let id = Id::unique("t");
        assert_eq!(ease(&ctx, id, true, PANEL), 1.0);
        assert_eq!(value(&ctx, id.with("v"), 3.0, PANEL), 3.0);
        assert_eq!(since_change(&ctx, id.with("k"), 1, PANEL), 1.0);
        assert_eq!(since_change(&ctx, id.with("k"), 2, PANEL), 1.0);
    }

    #[test]
    fn a_changed_key_restarts_from_zero_and_the_first_key_does_not() {
        let ctx = Context::default();
        let id = Id::unique("k");
        assert_eq!(since_change(&ctx, id, "a", PANEL), 1.0);
        assert_eq!(since_change(&ctx, id, "a", PANEL), 1.0);
        assert_eq!(since_change(&ctx, id, "b", PANEL), 0.0);
    }
}
