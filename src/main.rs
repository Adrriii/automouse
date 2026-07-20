// AutoMouse - mouse automation for Windows.
// Copyright (C) 2026 Adrien Boitelle.
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed WITHOUT ANY WARRANTY; see the LICENSE file or
// <https://www.gnu.org/licenses/> for details.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Media::timeBeginPeriod;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, UnregisterHotKey, INPUT, INPUT_MOUSE, MOD_NOREPEAT,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE, WM_HOTKEY};

/// Brand purple from the app icon, plus a lighter tint for text on dark bg.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(122, 82, 235);
const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(172, 145, 250);

/// Window size with the advanced section collapsed.
const BASE_W: f32 = 320.0;
const BASE_H: f32 = 372.0;

const MODE_CLICK: u8 = 0;
const MODE_HOLD: u8 = 1;
const REPEAT_FOREVER: u8 = 0;
const REPEAT_TIMES: u8 = 1;
const REPEAT_DURATION: u8 = 2;
const TOP_OFF: u8 = 0;
const TOP_ALWAYS: u8 = 1;
const TOP_WHILE_RUNNING: u8 = 2;
const BUTTON_LEFT: u8 = 0;
const BUTTON_RIGHT: u8 = 1;

const HOTKEY_ID: i32 = 1;
const VK_F1: u32 = 0x70;

struct Shared {
    active: AtomicBool,
    mode: AtomicU8,
    button: AtomicU8,
    interval_ms: AtomicU64,
    /// 0 = no click-count limit
    times: AtomicU64,
    /// 0 = no time limit
    duration_ms: AtomicU64,
    hotkey_vk: AtomicU32,
    /// False when RegisterHotKey failed (key owned by another app).
    hotkey_ok: AtomicBool,
    clicks_done: AtomicU64,
}

impl Shared {
    fn new() -> Self {
        Shared {
            active: AtomicBool::new(false),
            mode: AtomicU8::new(MODE_CLICK),
            button: AtomicU8::new(BUTTON_LEFT),
            interval_ms: AtomicU64::new(100),
            times: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            hotkey_vk: AtomicU32::new(VK_F1 + 8), // F9
            hotkey_ok: AtomicBool::new(true),
            clicks_done: AtomicU64::new(0),
        }
    }
}

fn send_mouse(flags: u32) {
    let mut input: INPUT = unsafe { std::mem::zeroed() };
    input.r#type = INPUT_MOUSE;
    input.Anonymous.mi.dwFlags = flags;
    unsafe {
        SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
    }
}

fn button_flags(button: u8) -> (u32, u32) {
    if button == BUTTON_RIGHT {
        (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
    } else {
        (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
    }
}

fn worker_thread(shared: Arc<Shared>) {
    loop {
        if !shared.active.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        // Snapshot settings for this run.
        let mode = shared.mode.load(Ordering::Relaxed);
        let (down, up) = button_flags(shared.button.load(Ordering::Relaxed));
        shared.clicks_done.store(0, Ordering::Relaxed);

        if mode == MODE_HOLD {
            let duration_ms = shared.duration_ms.load(Ordering::Relaxed);
            let end =
                (duration_ms != 0).then(|| Instant::now() + Duration::from_millis(duration_ms));
            send_mouse(down);
            while shared.active.load(Ordering::Relaxed) {
                if end.is_some_and(|e| Instant::now() >= e) {
                    shared.active.store(false, Ordering::Relaxed);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            send_mouse(up);
        } else {
            let times = shared.times.load(Ordering::Relaxed);
            let duration_ms = shared.duration_ms.load(Ordering::Relaxed);
            let end =
                (duration_ms != 0).then(|| Instant::now() + Duration::from_millis(duration_ms));
            let expired = |end: Option<Instant>| end.is_some_and(|e| Instant::now() >= e);
            let mut count: u64 = 0;
            // Absolute deadlines so Sleep() overshoot never accumulates.
            let mut next = Instant::now();
            while shared.active.load(Ordering::Relaxed) {
                send_mouse(down);
                send_mouse(up);
                count += 1;
                shared.clicks_done.store(count, Ordering::Relaxed);
                if (times != 0 && count >= times) || expired(end) {
                    shared.active.store(false, Ordering::Relaxed);
                    break;
                }
                next += Duration::from_millis(shared.interval_ms.load(Ordering::Relaxed).max(1));
                let now = Instant::now();
                if next < now {
                    next = now; // fell behind; don't burst to catch up
                }
                // Short chunks so an abort is picked up quickly.
                while shared.active.load(Ordering::Relaxed) {
                    if expired(end) {
                        shared.active.store(false, Ordering::Relaxed);
                        break;
                    }
                    let now = Instant::now();
                    if now >= next {
                        break;
                    }
                    std::thread::sleep((next - now).min(Duration::from_millis(10)));
                }
            }
        }
    }
}

/// Owns the global hotkey. Registration is thread-bound, so the same thread
/// polls its message queue and re-registers whenever the configured key changes.
fn hotkey_thread(shared: Arc<Shared>) {
    // MOD_NOREPEAT: holding the key must not auto-repeat WM_HOTKEY, or the
    // active state toggles ~30x/s and Hold mode degenerates into a clicker.
    let register =
        |vk: u32| unsafe { RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, MOD_NOREPEAT, vk) } != 0;

    let mut registered_vk = shared.hotkey_vk.load(Ordering::Relaxed);
    let mut ok = register(registered_vk);
    shared.hotkey_ok.store(ok, Ordering::Relaxed);
    let mut tick: u32 = 0;
    loop {
        let wanted = shared.hotkey_vk.load(Ordering::Relaxed);
        // Re-register on key change, and retry every ~2 s while the key is
        // owned by another app (it may get closed).
        if wanted != registered_vk || (!ok && tick.is_multiple_of(100)) {
            unsafe {
                UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
            }
            ok = register(wanted);
            shared.hotkey_ok.store(ok, Ordering::Relaxed);
            registered_vk = wanted;
        }
        tick = tick.wrapping_add(1);
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_HOTKEY {
                shared.active.fetch_xor(true, Ordering::Relaxed);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct App {
    shared: Arc<Shared>,
    mode: u8,
    button: u8,
    interval_ms: u64,
    repeat: u8,
    times: u64,
    dur_h: u32,
    dur_m: u32,
    dur_s: u32,
    hotkey_index: usize, // 0 => F1 ... 11 => F12
    on_top: u8,
    presets: Vec<Preset>,
    /// Name of the preset last applied/saved; None = Default.
    current_preset: Option<String>,
    new_preset_name: String,
    advanced_open: bool,
    /// Whether the advanced section is drawn this frame: only once the window
    /// resize has landed, to avoid a one-frame layout jump.
    adv_shown: bool,
    /// Last settings snapshot written to disk, to save only on change.
    saved: String,
    /// Window height actually applied, to resize only on change.
    applied_height: Option<i32>,
    /// Window level actually applied, to send the viewport command on change.
    applied_on_top: Option<bool>,
    /// When the current run began (for the duration countdown display).
    run_started: Option<Instant>,
}

/// A saved snapshot of the clicking settings (not hotkey / window options).
#[derive(Clone, PartialEq)]
struct Preset {
    name: String,
    mode: u8,
    button: u8,
    interval_ms: u64,
    repeat: u8,
    times: u64,
    dur_h: u32,
    dur_m: u32,
    dur_s: u32,
}

fn config_path() -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("automouse");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("config.ini"))
}

fn parse_fkey(s: &str) -> Option<usize> {
    let n: usize = s.strip_prefix('F')?.parse().ok()?;
    if (1..=12).contains(&n) {
        Some(n - 1)
    } else {
        None
    }
}

/// Two mutually exclusive choices as full-width side-by-side toggle buttons.
fn selector(ui: &mut egui::Ui, value: &mut u8, options: [(u8, &str); 2]) {
    let w = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
    ui.horizontal(|ui| {
        for (v, label) in options {
            if ui
                .add_sized([w, 26.0], egui::SelectableLabel::new(*value == v, label))
                .clicked()
            {
                *value = v;
            }
        }
    });
}

impl App {
    fn new(shared: Arc<Shared>) -> Self {
        let mut app = App {
            shared,
            mode: MODE_CLICK,
            button: BUTTON_LEFT,
            interval_ms: 100,
            repeat: REPEAT_FOREVER,
            times: 100,
            dur_h: 0,
            dur_m: 1,
            dur_s: 0,
            hotkey_index: 8, // F9
            on_top: TOP_OFF,
            presets: Vec::new(),
            current_preset: None,
            new_preset_name: String::new(),
            advanced_open: false,
            adv_shown: false,
            saved: String::new(),
            applied_height: None,
            applied_on_top: None,
            run_started: None,
        };
        app.load_settings();

        if let Some(c) = &app.current_preset {
            if !app.presets.iter().any(|p| &p.name == c) {
                app.current_preset = None;
            }
        }

        app.saved = app.settings_string();
        app
    }

    fn load_settings(&mut self) {
        let Some(path) = config_path() else { return };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if let Some(name) = k.strip_prefix("preset.") {
                let f: Vec<u64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                if f.len() == 8 && !name.is_empty() {
                    self.presets.push(Preset {
                        name: name.to_string(),
                        mode: (f[0] as u8).min(1),
                        button: (f[1] as u8).min(1),
                        interval_ms: f[2].clamp(1, 600_000),
                        repeat: (f[3] as u8).min(2),
                        times: f[4].clamp(1, 1_000_000),
                        dur_h: (f[5] as u32).min(99),
                        dur_m: (f[6] as u32).min(59),
                        dur_s: (f[7] as u32).min(59),
                    });
                }
                continue;
            }
            match k {
                "mode" => self.mode = if v == "hold" { MODE_HOLD } else { MODE_CLICK },
                "button" => {
                    self.button = if v == "right" {
                        BUTTON_RIGHT
                    } else {
                        BUTTON_LEFT
                    }
                }
                "interval_ms" => {
                    if let Ok(n) = v.parse() {
                        self.interval_ms = n;
                    }
                }
                // "forever" is the legacy pre-duration key for this setting.
                "forever" => {
                    self.repeat = if v == "1" {
                        REPEAT_FOREVER
                    } else {
                        REPEAT_TIMES
                    }
                }
                "repeat" => {
                    self.repeat = match v {
                        "times" => REPEAT_TIMES,
                        "duration" => REPEAT_DURATION,
                        _ => REPEAT_FOREVER,
                    }
                }
                "times" => {
                    if let Ok(n) = v.parse() {
                        self.times = n;
                    }
                }
                "duration" => {
                    let mut parts = v.splitn(3, ':').map(|p| p.parse::<u32>().unwrap_or(0));
                    self.dur_h = parts.next().unwrap_or(0).min(99);
                    self.dur_m = parts.next().unwrap_or(0).min(59);
                    self.dur_s = parts.next().unwrap_or(0).min(59);
                }
                "hotkey" => {
                    if let Some(i) = parse_fkey(v) {
                        self.hotkey_index = i;
                    }
                }
                "ontop" => {
                    self.on_top = match v {
                        "1" => TOP_ALWAYS,
                        "2" => TOP_WHILE_RUNNING,
                        _ => TOP_OFF,
                    }
                }
                "current" => {
                    if !v.is_empty() {
                        self.current_preset = Some(v.to_string());
                    }
                }
                _ => {}
            }
        }
        self.interval_ms = self.interval_ms.clamp(1, 600_000);
        self.times = self.times.clamp(1, 1_000_000);
    }

    fn settings_string(&self) -> String {
        let mut s = format!(
            "mode={}\nbutton={}\ninterval_ms={}\nrepeat={}\ntimes={}\nduration={:02}:{:02}:{:02}\nhotkey={}\nontop={}\n",
            if self.mode == MODE_HOLD { "hold" } else { "click" },
            if self.button == BUTTON_RIGHT { "right" } else { "left" },
            self.interval_ms,
            match self.repeat {
                REPEAT_TIMES => "times",
                REPEAT_DURATION => "duration",
                _ => "forever",
            },
            self.times,
            self.dur_h,
            self.dur_m,
            self.dur_s,
            self.hotkey_name(),
            self.on_top,
        );
        s.push_str(&format!(
            "current={}\n",
            self.current_preset.as_deref().unwrap_or("")
        ));
        for p in &self.presets {
            s.push_str(&format!(
                "preset.{}={},{},{},{},{},{},{},{}\n",
                p.name,
                p.mode,
                p.button,
                p.interval_ms,
                p.repeat,
                p.times,
                p.dur_h,
                p.dur_m,
                p.dur_s
            ));
        }
        s
    }

    fn capture_preset(&self, name: String) -> Preset {
        Preset {
            name,
            mode: self.mode,
            button: self.button,
            interval_ms: self.interval_ms,
            repeat: self.repeat,
            times: self.times,
            dur_h: self.dur_h,
            dur_m: self.dur_m,
            dur_s: self.dur_s,
        }
    }

    fn apply_preset(&mut self, p: &Preset) {
        self.mode = p.mode;
        self.button = p.button;
        self.interval_ms = p.interval_ms;
        self.repeat = p.repeat;
        self.times = p.times;
        self.dur_h = p.dur_h;
        self.dur_m = p.dur_m;
        self.dur_s = p.dur_s;
    }

    fn apply_default(&mut self) {
        self.apply_preset(&Preset {
            name: String::new(),
            mode: MODE_CLICK,
            button: BUTTON_LEFT,
            interval_ms: 100,
            repeat: REPEAT_FOREVER,
            times: 100,
            dur_h: 0,
            dur_m: 1,
            dur_s: 0,
        });
    }

    /// Height of the expanded advanced section: preset rows (capped by the
    /// scroll area) plus the name+add row.
    fn adv_height(&self) -> f32 {
        34.0 + (22.0 * (self.presets.len() + 1) as f32).min(80.0)
    }

    fn duration_ms(&self) -> u64 {
        (u64::from(self.dur_h) * 3600 + u64::from(self.dur_m) * 60 + u64::from(self.dur_s)) * 1000
    }

    fn hotkey_name(&self) -> String {
        format!("F{}", self.hotkey_index + 1)
    }

    fn push_settings(&self) {
        let s = &self.shared;
        s.mode.store(self.mode, Ordering::Relaxed);
        s.button.store(self.button, Ordering::Relaxed);
        s.interval_ms
            .store(self.interval_ms.max(1), Ordering::Relaxed);
        let (times, duration_ms) = match self.repeat {
            REPEAT_TIMES => (self.times.max(1), 0),
            REPEAT_DURATION => (0, self.duration_ms()),
            _ => (0, 0),
        };
        s.times.store(times, Ordering::Relaxed);
        s.duration_ms.store(duration_ms, Ordering::Relaxed);
        s.hotkey_vk
            .store(VK_F1 + self.hotkey_index as u32, Ordering::Relaxed);
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        selector(
            ui,
            &mut self.mode,
            [(MODE_CLICK, "Autoclick"), (MODE_HOLD, "Hold")],
        );
        selector(
            ui,
            &mut self.button,
            [(BUTTON_LEFT, "Left button"), (BUTTON_RIGHT, "Right button")],
        );
        ui.add_space(6.0);

        // Interval and click-count only apply to Autoclick; Forever and
        // Duration apply to both modes. "N times" makes no sense for Hold, so
        // coerce it away when switching.
        let click_mode = self.mode == MODE_CLICK;
        if !click_mode && self.repeat == REPEAT_TIMES {
            self.repeat = REPEAT_FOREVER;
        }
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("options")
                .num_columns(2)
                .spacing([18.0, 8.0])
                .show(ui, |ui| {
                    ui.add_enabled_ui(click_mode, |ui| ui.label("Interval"));
                    ui.add_enabled_ui(click_mode, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.interval_ms)
                                    .speed(5)
                                    .range(1..=600_000),
                            );
                            ui.weak("ms");
                        });
                    });
                    ui.end_row();

                    ui.label(if click_mode { "Repeat" } else { "Hold" });
                    ui.vertical(|ui| {
                        ui.radio_value(&mut self.repeat, REPEAT_FOREVER, "Forever");
                        ui.add_enabled_ui(click_mode, |ui| {
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.repeat, REPEAT_TIMES, "");
                                ui.add_enabled(
                                    self.repeat == REPEAT_TIMES,
                                    egui::DragValue::new(&mut self.times)
                                        .speed(1)
                                        .range(1..=1_000_000),
                                );
                                ui.weak("times");
                            });
                        });
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.repeat, REPEAT_DURATION, "");
                            ui.add_enabled_ui(self.repeat == REPEAT_DURATION, |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let pad = |n: f64, _| format!("{:02}", n as u32);
                                ui.add(
                                    egui::DragValue::new(&mut self.dur_h)
                                        .range(0..=99)
                                        .custom_formatter(pad),
                                );
                                ui.weak(":");
                                ui.add(
                                    egui::DragValue::new(&mut self.dur_m)
                                        .range(0..=59)
                                        .custom_formatter(pad),
                                );
                                ui.weak(":");
                                ui.add(
                                    egui::DragValue::new(&mut self.dur_s)
                                        .range(0..=59)
                                        .custom_formatter(pad),
                                );
                                ui.add_space(4.0);
                                ui.weak("hh:mm:ss");
                            });
                        });
                    });
                    ui.end_row();
                });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Hotkey");
            egui::ComboBox::from_id_salt("hotkey")
                .selected_text(self.hotkey_name())
                .show_ui(ui, |ui| {
                    for i in 0..12 {
                        ui.selectable_value(&mut self.hotkey_index, i, format!("F{}", i + 1));
                    }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("ontop")
                    .selected_text(match self.on_top {
                        TOP_ALWAYS => "Always",
                        TOP_WHILE_RUNNING => "Running",
                        _ => "Never",
                    })
                    .width(88.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.on_top, TOP_OFF, "Never");
                        ui.selectable_value(&mut self.on_top, TOP_ALWAYS, "Always");
                        ui.selectable_value(&mut self.on_top, TOP_WHILE_RUNNING, "While running");
                    });
                ui.label("On top");
            });
        });

        self.push_settings();

        // Spacer pushing the action area to the bottom edge of the window.
        // Clamped so transition frames (window mid-resize) don't shift layout.
        let status_h = if self.shared.hotkey_ok.load(Ordering::Relaxed) {
            16.0
        } else {
            32.0
        };
        let needed = 34.0
            + 4.0
            + status_h
            + 10.0
            + 6.0
            + 26.0
            + if self.adv_shown {
                self.adv_height()
            } else {
                0.0
            };
        ui.add_space((ui.available_height() - needed).clamp(4.0, 30.0));

        if ui
            .add_sized(
                [ui.available_width(), 34.0],
                egui::Button::new(
                    egui::RichText::new("Start")
                        .size(16.0)
                        .color(egui::Color32::WHITE),
                )
                .fill(ACCENT),
            )
            .clicked()
        {
            self.shared.active.store(true, Ordering::Relaxed);
        }
        ui.add_space(4.0);
        if self.shared.hotkey_ok.load(Ordering::Relaxed) {
            ui.weak(format!("Idle. Press {} to start/stop", self.hotkey_name()));
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(235, 90, 90),
                format!(
                    "{} is taken by another app (autoclicker already running?). Close it or pick another key",
                    self.hotkey_name()
                ),
            );
        }
        ui.add_space(10.0);
        ui.separator();
        // Full-width hit area so the toggle is easy to click, not just the glyphs.
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let color = if resp.hovered() {
            ACCENT_LIGHT
        } else {
            ui.visuals().weak_text_color()
        };
        ui.painter().text(
            rect.left_center() + egui::vec2(0.0, 2.0),
            egui::Align2::LEFT_CENTER,
            if self.advanced_open {
                "⏷ advanced"
            } else {
                "⏵ advanced"
            },
            egui::FontId::proportional(13.0),
            color,
        );
        if resp.clicked() {
            self.advanced_open = !self.advanced_open;
        }
        if self.adv_shown {
            self.presets_ui(ui);
        }
    }

    fn presets_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        let mut apply: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut overwrite: Option<usize> = None;
        egui::ScrollArea::vertical()
            .max_height(80.0)
            .show(ui, |ui| {
                if ui
                    .add(egui::SelectableLabel::new(
                        self.current_preset.is_none(),
                        "Default",
                    ))
                    .clicked()
                {
                    apply = Some(usize::MAX);
                }
                for (i, p) in self.presets.iter().enumerate() {
                    let is_current = self.current_preset.as_deref() == Some(p.name.as_str());
                    let dirty = is_current && self.capture_preset(p.name.clone()) != *p;
                    ui.horizontal(|ui| {
                        let label = if dirty {
                            format!("{} •", p.name)
                        } else {
                            p.name.clone()
                        };
                        if ui
                            .add(egui::SelectableLabel::new(is_current, label))
                            .clicked()
                        {
                            apply = Some(i);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✖").clicked() {
                                delete = Some(i);
                            }
                            if dirty
                                && ui
                                    .small_button("save")
                                    .on_hover_text(
                                        "Overwrite this preset with the current settings",
                                    )
                                    .clicked()
                            {
                                overwrite = Some(i);
                            }
                        });
                    });
                }
            });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_preset_name)
                    .hint_text("preset name")
                    .desired_width(ui.available_width() - 30.0),
            );
            if ui.button("+").clicked() {
                let name: String = self
                    .new_preset_name
                    .trim()
                    .chars()
                    .filter(|c| !matches!(c, '=' | ',' | '\n' | '\r'))
                    .take(24)
                    .collect();
                let name = if name.is_empty() {
                    format!("Preset {}", self.presets.len() + 1)
                } else {
                    name
                };
                let p = self.capture_preset(name);
                self.current_preset = Some(p.name.clone());
                if let Some(existing) = self.presets.iter_mut().find(|q| q.name == p.name) {
                    *existing = p;
                } else {
                    self.presets.push(p);
                }
                self.new_preset_name.clear();
            }
        });
        match apply {
            Some(usize::MAX) => {
                self.apply_default();
                self.current_preset = None;
            }
            Some(i) => {
                let p = self.presets[i].clone();
                self.apply_preset(&p);
                self.current_preset = Some(p.name);
            }
            None => {}
        }
        if let Some(i) = overwrite {
            self.presets[i] = self.capture_preset(self.presets[i].name.clone());
        }
        if let Some(i) = delete {
            if self.current_preset.as_deref() == Some(self.presets[i].name.as_str()) {
                self.current_preset = None;
            }
            self.presets.remove(i);
        }
    }

    /// Remaining-time line shown while a duration-limited run is active.
    fn countdown_ui(&self, ui: &mut egui::Ui) {
        if self.repeat != REPEAT_DURATION {
            return;
        }
        let Some(start) = self.run_started else {
            return;
        };
        let left = Duration::from_millis(self.duration_ms()).saturating_sub(start.elapsed());
        let s = left.as_secs();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!(
                "{:02}:{:02}:{:02} left",
                s / 3600,
                s % 3600 / 60,
                s % 60
            ))
            .monospace(),
        );
    }

    fn running_ui(&mut self, ui: &mut egui::Ui) {
        // Stop lives at the TOP while Start lives at the BOTTOM, so the
        // cursor (still parked where Start was clicked) is never over
        // Stop when our own synthetic clicks land on this window.
        if ui
            .add_sized(
                [ui.available_width(), 34.0],
                egui::Button::new(egui::RichText::new("Stop").size(16.0)),
            )
            .clicked()
        {
            self.shared.active.store(false, Ordering::Relaxed);
        }

        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            if self.mode == MODE_HOLD {
                let b = if self.button == BUTTON_RIGHT {
                    "right"
                } else {
                    "left"
                };
                ui.colored_label(
                    ACCENT_LIGHT,
                    egui::RichText::new(format!("Holding {b} button"))
                        .size(18.0)
                        .strong(),
                );
                self.countdown_ui(ui);
            } else {
                ui.colored_label(
                    ACCENT_LIGHT,
                    egui::RichText::new("Clicking").size(18.0).strong(),
                );
                ui.add_space(6.0);
                let n = self.shared.clicks_done.load(Ordering::Relaxed);
                let total = if self.repeat == REPEAT_TIMES {
                    format!(" / {}", self.times)
                } else {
                    String::new()
                };
                ui.label(
                    egui::RichText::new(format!("{n}{total}"))
                        .size(28.0)
                        .monospace(),
                );
                ui.weak("clicks");
                self.countdown_ui(ui);
            }
            ui.add_space(12.0);
            ui.weak(format!("press {} to stop", self.hotkey_name()));
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let running = self.shared.active.load(Ordering::Relaxed);

        // Track when the current run began (drives the countdown display).
        if running {
            if self.run_started.is_none() {
                self.run_started = Some(Instant::now());
            }
        } else {
            self.run_started = None;
        }

        // Grow the fixed-size window while the advanced section is open, and
        // only draw that section once the resize has landed (otherwise the
        // spacer collapses for one frame and the whole layout jumps).
        let adv_h = self.adv_height();
        let want_h: i32 = if self.advanced_open && !running {
            BASE_H as i32 + adv_h as i32
        } else {
            BASE_H as i32
        };
        if self.applied_height != Some(want_h) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                BASE_W,
                want_h as f32,
            )));
            self.applied_height = Some(want_h);
        }
        self.adv_shown = self.advanced_open && ctx.screen_rect().height() >= BASE_H - 1.0 + adv_h;

        let want_top = match self.on_top {
            TOP_ALWAYS => true,
            TOP_WHILE_RUNNING => running,
            _ => false,
        };
        if self.applied_on_top != Some(want_top) {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if want_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
            self.applied_on_top = Some(want_top);
        }

        // No bottom margin: the "advanced" area sits flush with the window edge.
        let frame = egui::Frame::central_panel(&ctx.style()).inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 8,
            bottom: 5,
        });
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            if running {
                self.running_ui(ui);
            } else {
                self.settings_ui(ui);
            }
        });

        // Persist settings whenever they change.
        let snapshot = self.settings_string();
        if snapshot != self.saved {
            if let Some(path) = config_path() {
                let _ = std::fs::write(path, &snapshot);
            }
            self.saved = snapshot;
        }

        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

/// Purple accent for everything egui colors by default (selected segments,
/// radio marks, text selection, ...).
fn apply_theme(ctx: &egui::Context) {
    ctx.style_mut(|s| {
        let v = &mut s.visuals;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke.color = egui::Color32::WHITE;
        v.hyperlink_color = ACCENT_LIGHT;
        v.widgets.hovered.bg_stroke.color = ACCENT_LIGHT;
        v.widgets.active.bg_stroke.color = ACCENT_LIGHT;
    });
}

fn app_icon() -> egui::IconData {
    egui::IconData {
        rgba: include_bytes!("../icons/app64.rgba").to_vec(),
        width: 64,
        height: 64,
    }
}

fn main() -> eframe::Result {
    // 1 ms system timer resolution; without it Sleep() rounds up to ~16 ms
    // (worse when throttled), wrecking short click intervals.
    unsafe {
        timeBeginPeriod(1);
    }

    let shared = Arc::new(Shared::new());

    {
        let s = shared.clone();
        std::thread::spawn(move || worker_thread(s));
    }
    {
        let s = shared.clone();
        std::thread::spawn(move || hotkey_thread(s));
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([BASE_W, BASE_H])
            .with_resizable(false)
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "AutoMouse",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(App::new(shared)))
        }),
    )
}
