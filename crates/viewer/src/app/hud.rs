use crate::controls::{Device, Frame, flight, walker};
use dashboard_core::protocol::{CameraMode, GamepadButton};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2};
use std::time::{Duration, Instant};

const CARD_AUTO_HIDE: Duration = Duration::from_secs(3);
const PILL_HOLD: Duration = Duration::from_millis(1200);
const FADE_IN: f32 = 0.12;
const FADE_OUT: f32 = 0.3;

const GLASS_FILL: [u8; 4] = [14, 16, 20, 204];
const BORDER_ALPHA: u8 = 28;
const DIVIDER_ALPHA: u8 = 18;
const KEY_FILL_ALPHA: u8 = 18;
const KEY_BORDER_ALPHA: u8 = 46;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AircraftHud {
    Free,
    Loading(String),
    Ready(String),
    Refused(String),
}

#[derive(Clone, Debug)]
pub(super) enum ActiveHud {
    Walk {
        view: walker::View,
        gait: walker::Gait,
        scale: f32,
        scale_limit: Option<bool>,
        figure_missing: bool,
    },
    FlyFree {
        speed: f32,
        speed_limit: Option<bool>,
        aircraft: AircraftHud,
    },
    FlyAircraft {
        title: String,
        view: flight::View,
    },
}

pub(super) enum HudState {
    StartPick {
        mode: CameraMode,
        pad_connected: bool,
    },
    Active {
        active: ActiveHud,
        captured: bool,
        device: Device,
    },
}

#[derive(Clone, Debug)]
struct Pill {
    text: String,
    warning: bool,
    shown_at: Instant,
    hold_until: Instant,
}

#[derive(Clone, Debug)]
struct KeyNames {
    w: String,
    a: String,
    s: String,
    d: String,
    equals: String,
    minus: String,
    zero: String,
    v: String,
    c: String,
}

impl KeyNames {
    fn detect() -> Self {
        Self {
            w: scan_label(0x11, "W"),
            a: scan_label(0x1e, "A"),
            s: scan_label(0x1f, "S"),
            d: scan_label(0x20, "D"),
            equals: scan_label(0x0d, "="),
            minus: scan_label(0x0c, "−"),
            zero: scan_label(0x0b, "0"),
            v: scan_label(0x2f, "V"),
            c: scan_label(0x2e, "C"),
        }
    }
}

#[cfg(windows)]
fn scan_label(scan: u32, fallback: &str) -> String {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyNameTextW;
    let mut buffer = [0u16; 32];
    let lparam = (scan << 16) as i32;
    // SAFETY: `buffer` is a valid writable UTF-16 slice and its length fits in `i32`.
    let written = unsafe { GetKeyNameTextW(lparam, buffer.as_mut_ptr(), buffer.len() as i32) };
    if written > 0 && (written as usize) <= buffer.len() {
        let name = String::from_utf16_lossy(&buffer[..written as usize])
            .trim()
            .to_owned();
        if !name.is_empty() && name.chars().count() <= 4 {
            return if name == "-" {
                "−".to_owned()
            } else {
                name
            };
        }
    }
    fallback.to_owned()
}

#[cfg(not(windows))]
fn scan_label(_scan: u32, fallback: &str) -> String {
    fallback.to_owned()
}

#[derive(Default)]
pub(super) struct Hud {
    card_open: bool,
    card_dismiss_at: Option<Instant>,
    pill: Option<Pill>,
    was_captured: bool,
    last_walk_view: Option<walker::View>,
    last_walk_gait: Option<walker::Gait>,
    last_walk_scale: Option<f32>,
    last_fly_view: Option<flight::View>,
    last_free_speed: Option<f32>,
    last_aircraft: Option<AircraftHud>,
    key_names: Option<KeyNames>,
}

impl Hud {
    pub(super) fn enter_mode(&mut self) {
        self.card_open = true;
        self.card_dismiss_at = None;
        self.pill = None;
        self.was_captured = false;
        self.last_walk_view = None;
        self.last_walk_gait = None;
        self.last_walk_scale = None;
        self.last_fly_view = None;
        self.last_free_speed = None;
        self.last_aircraft = None;
    }

    pub(super) fn leave(&mut self) {
        let key_names = self.key_names.take();
        *self = Self {
            key_names,
            ..Self::default()
        };
    }

    fn show_pill(&mut self, text: String, warning: bool, now: Instant) {
        let shown_at = self
            .pill
            .as_ref()
            .filter(|pill| now < pill.hold_until + Duration::from_secs_f32(FADE_OUT))
            .map_or(now, |pill| pill.shown_at);
        self.pill = Some(Pill {
            text,
            warning,
            shown_at,
            hold_until: now + PILL_HOLD,
        });
    }

    pub(super) fn observe(&mut self, active: &ActiveHud, controls: &Frame, now: Instant) {
        if self.was_captured && !controls.captured {
            self.card_open = true;
            self.card_dismiss_at = None;
        }
        self.was_captured = controls.captured;
        if controls.pad.pressed(GamepadButton::Start) {
            self.card_open = !self.card_open;
            self.card_dismiss_at = None;
        }
        if self.card_open {
            if self.card_dismiss_at.is_none() && controls.moved_or_looked() {
                self.card_dismiss_at = Some(now + CARD_AUTO_HIDE);
            }
            if self.card_dismiss_at.is_some_and(|at| now >= at) {
                self.card_open = false;
                self.card_dismiss_at = None;
            }
        }
        match active {
            ActiveHud::Walk {
                view,
                gait,
                scale,
                scale_limit,
                figure_missing,
            } => {
                self.last_fly_view = None;
                self.last_free_speed = None;
                self.last_aircraft = None;
                if let Some(previous) = self.last_walk_scale
                    && ((scale - previous).abs() / previous.max(1e-5)) > 1e-3
                {
                    let suffix = match scale_limit {
                        Some(false) => " · smallest",
                        Some(true) => " · largest",
                        None => "",
                    };
                    self.show_pill(
                        format!("Size {}{suffix}", format_multiplier(*scale)),
                        false,
                        now,
                    );
                }
                if let Some(previous) = self.last_walk_view
                    && previous != *view
                {
                    match view {
                        walker::View::First => self.show_pill("First person".into(), false, now),
                        walker::View::Third if *figure_missing => {
                            self.show_pill("Third person · figure not drawn".into(), true, now)
                        }
                        walker::View::Third => self.show_pill("Third person".into(), false, now),
                    }
                }
                if let Some(previous) = self.last_walk_gait {
                    let was_flying = previous == walker::Gait::Flying;
                    let now_flying = *gait == walker::Gait::Flying;
                    if !was_flying && now_flying {
                        self.show_pill("Flying".into(), false, now);
                    } else if was_flying && !now_flying {
                        self.show_pill("Walking".into(), false, now);
                    }
                }
                self.last_walk_scale = Some(*scale);
                self.last_walk_view = Some(*view);
                self.last_walk_gait = Some(*gait);
            }
            ActiveHud::FlyFree {
                speed,
                speed_limit,
                aircraft,
            } => {
                self.last_walk_view = None;
                self.last_walk_gait = None;
                self.last_walk_scale = None;
                self.last_fly_view = None;
                if let Some(previous) = self.last_free_speed
                    && ((speed - previous).abs() / previous.max(1e-5)) > 1e-3
                {
                    let suffix = match speed_limit {
                        Some(false) => " · slowest",
                        Some(true) => " · fastest",
                        None => "",
                    };
                    self.show_pill(
                        format!("Speed {}{suffix}", format_multiplier(*speed)),
                        false,
                        now,
                    );
                }
                if self.last_aircraft.as_ref() != Some(aircraft) {
                    match (self.last_aircraft.as_ref(), aircraft) {
                        (Some(_), AircraftHud::Free) => {
                            self.show_pill("Free camera".into(), false, now);
                        }
                        (_, AircraftHud::Refused(title)) => {
                            self.show_pill(
                                format!("{title} can’t be flown · free camera"),
                                true,
                                now,
                            );
                        }
                        _ => {}
                    }
                }
                self.last_free_speed = Some(*speed);
                self.last_aircraft = Some(aircraft.clone());
            }
            ActiveHud::FlyAircraft { title, view } => {
                self.last_walk_view = None;
                self.last_walk_gait = None;
                self.last_walk_scale = None;
                self.last_free_speed = None;
                let badge = AircraftHud::Ready(title.clone());
                if self.last_aircraft.as_ref() != Some(&badge) {
                    if self.last_aircraft.is_some() {
                        self.show_pill(format!("Flying {title}"), false, now);
                    }
                    self.last_aircraft = Some(badge);
                }
                if let Some(previous) = self.last_fly_view
                    && previous != *view
                {
                    let label = match view {
                        flight::View::Chase => "Chase view",
                        flight::View::Nose => "Nose view",
                    };
                    self.show_pill(label.into(), false, now);
                }
                self.last_fly_view = Some(*view);
            }
        }
    }

    pub(super) fn paint(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        points: f32,
        state: &HudState,
        now: Instant,
    ) {
        if rect.width() < 80.0 || rect.height() < 60.0 {
            return;
        }
        let painter = ui.painter().with_clip_rect(rect);
        let p = points.max(0.5);
        match state {
            HudState::StartPick {
                mode,
                pad_connected,
            } => {
                if *pad_connected {
                    paint_start_ring(&painter, rect.center(), p);
                }
                let kind = match mode {
                    CameraMode::Walk => "walk",
                    _ => "flight",
                };
                let text = if *pad_connected {
                    format!(
                        "Click the map where the {kind} starts, or press A to start at the ring. \
                         Esc or B goes back to the map."
                    )
                } else {
                    format!("Click the map where the {kind} starts. Esc goes back to the map.")
                };
                let bottom = rect.bottom() - 20.0 * p;
                paint_pill(&painter, rect, bottom, &text, false, 1.0, p);
            }
            HudState::Active {
                active,
                captured,
                device,
            } => {
                let mut bottom = rect.bottom() - 20.0 * p;
                let brief = self.pill.as_ref().and_then(|pill| {
                    let alpha = pill_alpha(pill, now);
                    (alpha > 0.0).then_some((pill.text.as_str(), pill.warning, alpha))
                });
                let pill_height = if let Some((text, warning, alpha)) = brief {
                    let h = paint_pill(&painter, rect, bottom, text, warning, alpha, p);
                    ui.ctx().request_repaint();
                    h
                } else {
                    self.pill = None;
                    if !self.card_open && !captured && *device == Device::Keyboard {
                        paint_pill(
                            &painter,
                            rect,
                            bottom,
                            "Click the map to look around",
                            false,
                            1.0,
                            p,
                        )
                    } else {
                        0.0
                    }
                };
                if pill_height > 0.0 {
                    bottom -= pill_height + 8.0 * p;
                }
                if self.card_open {
                    let names = self.key_names.get_or_insert_with(KeyNames::detect).clone();
                    paint_card(&painter, rect, bottom, active, *captured, *device, &names, p);
                    if self.card_dismiss_at.is_some() {
                        ui.ctx().request_repaint();
                    }
                }
            }
        }
    }
}

fn pill_alpha(pill: &Pill, now: Instant) -> f32 {
    if now < pill.hold_until {
        let elapsed = now.saturating_duration_since(pill.shown_at).as_secs_f32();
        (elapsed / FADE_IN).clamp(0.0, 1.0)
    } else {
        let elapsed = now.saturating_duration_since(pill.hold_until).as_secs_f32();
        (1.0 - elapsed / FADE_OUT).clamp(0.0, 1.0)
    }
}

fn rgba(base: [u8; 4], alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        base[0],
        base[1],
        base[2],
        (f32::from(base[3]) * alpha).round() as u8,
    )
}

fn white(a: u8, alpha: f32) -> Color32 {
    Color32::from_white_alpha((f32::from(a) * alpha).round() as u8)
}

fn paint_start_ring(painter: &egui::Painter, center: Pos2, p: f32) {
    let radius = 9.0 * p;
    painter.circle_stroke(
        center,
        radius + 1.0 * p,
        Stroke::new(1.0 * p, Color32::from_black_alpha(190)),
    );
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(2.0 * p, Color32::from_white_alpha(200)),
    );
    painter.circle_filled(center, 1.5 * p, Color32::from_white_alpha(230));
}

fn paint_pill(
    painter: &egui::Painter,
    bounds: Rect,
    bottom: f32,
    text: &str,
    warning: bool,
    alpha: f32,
    p: f32,
) -> f32 {
    let font = FontId::proportional(12.0 * p);
    let max_text = (bounds.width() - 48.0 * p).max(120.0 * p);
    let galley = painter.layout(text.to_owned(), font, white(235, alpha), max_text);
    let dot_space = if warning { 12.0 * p } else { 0.0 };
    let pad_x = 10.0 * p;
    let pad_y = 5.0 * p;
    let width = (galley.size().x + dot_space + pad_x * 2.0).min(bounds.width() - 16.0 * p);
    let height = (galley.size().y + pad_y * 2.0).max(24.0 * p);
    let rect = Rect::from_min_size(
        Pos2::new(bounds.center().x - width * 0.5, bottom - height),
        Vec2::new(width, height),
    );
    painter.rect(
        rect,
        CornerRadius::same((7.0 * p).round() as u8),
        rgba(GLASS_FILL, alpha),
        Stroke::new(1.0 * p, white(BORDER_ALPHA, alpha)),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + pad_x;
    if warning {
        painter.circle_filled(
            Pos2::new(x + 3.0 * p, rect.center().y),
            3.0 * p,
            Color32::from_rgba_unmultiplied(255, 255, 0, (255.0 * alpha).round() as u8),
        );
        x += dot_space;
    }
    let text_pos = Pos2::new(x, rect.center().y - galley.size().y * 0.5);
    painter.galley(text_pos, galley, white(235, alpha));
    height
}

#[derive(Clone)]
enum Token {
    Key(String),
    Pad(String),
    Note(String),
}

struct Row {
    tokens: Vec<Token>,
    label: String,
}

fn k(text: impl Into<String>) -> Token {
    Token::Key(text.into())
}

fn b(text: impl Into<String>) -> Token {
    Token::Pad(text.into())
}

fn note(text: impl Into<String>) -> Token {
    Token::Note(text.into())
}

fn row(tokens: Vec<Token>, label: impl Into<String>) -> Row {
    Row {
        tokens,
        label: label.into(),
    }
}

fn paint_card(
    painter: &egui::Painter,
    bounds: Rect,
    bottom: f32,
    active: &ActiveHud,
    captured: bool,
    device: Device,
    names: &KeyNames,
    p: f32,
) {
    let (title, subtitle, metric, warn_sub, rows) = card_content(active, device, names);
    let footer = match (device, captured) {
        (Device::Controller, _) => "Menu shows or hides this · View next camera",
        (Device::Keyboard, true) => "Esc frees the mouse and shows this · F12 next camera",
        (Device::Keyboard, false) => "Click the map to look around · Esc frees the mouse · F12 next camera",
    };

    let two_cols = bounds.width() >= 520.0 * p;
    let pad = 12.0 * p;
    let col_gap = 24.0 * p;
    let row_gap = 6.0 * p;
    let row_h = 20.0 * p;
    let key_col = 96.0 * p;
    let col_w = key_col + 128.0 * p;
    let inner_w = if two_cols {
        (col_w * 2.0 + col_gap).min(560.0 * p - pad * 2.0)
    } else {
        col_w
    }
    .min((bounds.width() - 24.0 * p - pad * 2.0).max(180.0 * p));
    let card_w = inner_w + pad * 2.0;

    let row_count = if two_cols {
        rows.len().div_ceil(2)
    } else {
        rows.len()
    };
    let head_h = 20.0 * p;
    let grid_h = if row_count == 0 {
        0.0
    } else {
        row_count as f32 * row_h + (row_count - 1) as f32 * row_gap
    };
    let foot_font = FontId::proportional(11.0 * p);
    let foot_galley = painter.layout(footer.to_owned(), foot_font, white(150, 1.0), inner_w);
    let foot_h = foot_galley.size().y;
    let card_h = pad + head_h + 8.0 * p + grid_h + 9.0 * p + 8.0 * p + foot_h + pad;

    let rect = Rect::from_min_size(
        Pos2::new(bounds.center().x - card_w * 0.5, bottom - card_h),
        Vec2::new(card_w, card_h),
    );
    painter.rect(
        rect,
        CornerRadius::same((10.0 * p).round() as u8),
        rgba(GLASS_FILL, 1.0),
        Stroke::new(1.0 * p, white(BORDER_ALPHA, 1.0)),
        StrokeKind::Inside,
    );

    let left = rect.left() + pad;
    let mut y = rect.top() + pad;
    let title_galley = painter.layout_no_wrap(title, FontId::proportional(13.0 * p), white(235, 1.0));
    let title_w = title_galley.size().x;
    painter.galley(Pos2::new(left, y), title_galley, white(235, 1.0));

    if !subtitle.is_empty() {
        let mut sx = left + title_w + 6.0 * p;
        if warn_sub {
            painter.circle_filled(
                Pos2::new(sx + 3.0 * p, y + 7.5 * p),
                3.0 * p,
                Color32::YELLOW,
            );
            sx += 10.0 * p;
        }
        let sub_galley = painter.layout_no_wrap(
            subtitle,
            FontId::proportional(12.0 * p),
            white(150, 1.0),
        );
        painter.galley(Pos2::new(sx, y + 1.0 * p), sub_galley, white(150, 1.0));
    }

    if let Some(metric) = metric {
        let m_galley = painter.layout_no_wrap(
            metric,
            FontId::proportional(12.0 * p),
            white(175, 1.0),
        );
        let mx = rect.right() - pad - m_galley.size().x;
        painter.galley(Pos2::new(mx, y + 1.0 * p), m_galley, white(175, 1.0));
    }

    y += head_h + 8.0 * p;
    let slot_w = if two_cols {
        (inner_w - col_gap) * 0.5
    } else {
        inner_w
    };
    for (index, item) in rows.iter().enumerate() {
        let (col, r) = if two_cols {
            (index % 2, index / 2)
        } else {
            (0, index)
        };
        let rx = left + col as f32 * (slot_w + col_gap);
        let ry = y + r as f32 * (row_h + row_gap);
        paint_row(painter, rx, ry, row_h, key_col, item, p);
    }

    y += grid_h + 9.0 * p;
    painter.line_segment(
        [Pos2::new(left, y), Pos2::new(rect.right() - pad, y)],
        Stroke::new(1.0 * p, white(DIVIDER_ALPHA, 1.0)),
    );
    y += 8.0 * p;
    painter.galley(Pos2::new(left, y), foot_galley, white(150, 1.0));
}

fn paint_row(
    painter: &egui::Painter,
    x: f32,
    y: f32,
    row_h: f32,
    key_col: f32,
    item: &Row,
    p: f32,
) {
    let mut cx = x;
    let cy = y + row_h * 0.5;
    let key_h = 18.0 * p;
    let key_font = FontId::proportional(11.0 * p);
    for token in &item.tokens {
        match token {
            Token::Key(text) => {
                let galley = painter.layout_no_wrap(text.clone(), key_font.clone(), white(235, 1.0));
                let w = (galley.size().x + 9.0 * p).max(20.0 * p);
                let cap = Rect::from_min_size(Pos2::new(cx, cy - key_h * 0.5), Vec2::new(w, key_h));
                painter.rect(
                    cap,
                    CornerRadius::same((4.5 * p).round() as u8),
                    white(KEY_FILL_ALPHA, 1.0),
                    Stroke::new(1.0 * p, white(KEY_BORDER_ALPHA, 1.0)),
                    StrokeKind::Inside,
                );
                painter.galley(
                    Pos2::new(cap.center().x - galley.size().x * 0.5, cy - galley.size().y * 0.5),
                    galley,
                    white(235, 1.0),
                );
                cx += w + 3.0 * p;
            }
            Token::Pad(text) => {
                let galley = painter.layout_no_wrap(text.clone(), key_font.clone(), white(235, 1.0));
                let w = (galley.size().x + 8.0 * p).max(key_h);
                let cap = Rect::from_min_size(Pos2::new(cx, cy - key_h * 0.5), Vec2::new(w, key_h));
                let radius = (key_h * 0.5).round() as u8;
                painter.rect(
                    cap,
                    CornerRadius::same(radius),
                    white(KEY_FILL_ALPHA, 1.0),
                    Stroke::new(1.0 * p, white(KEY_BORDER_ALPHA, 1.0)),
                    StrokeKind::Inside,
                );
                painter.galley(
                    Pos2::new(cap.center().x - galley.size().x * 0.5, cy - galley.size().y * 0.5),
                    galley,
                    white(235, 1.0),
                );
                cx += w + 3.0 * p;
            }
            Token::Note(text) => {
                let galley = painter.layout_no_wrap(text.clone(), key_font.clone(), white(155, 1.0));
                let w = galley.size().x;
                painter.galley(Pos2::new(cx + 2.0 * p, cy - galley.size().y * 0.5), galley, white(155, 1.0));
                cx += w + 5.0 * p;
            }
        }
    }
    let label_x = (x + key_col).max(cx + 4.0 * p);
    painter.text(
        Pos2::new(label_x, cy),
        Align2::LEFT_CENTER,
        &item.label,
        FontId::proportional(12.0 * p),
        white(225, 1.0),
    );
}

fn card_content(
    active: &ActiveHud,
    device: Device,
    names: &KeyNames,
) -> (String, String, Option<String>, bool, Vec<Row>) {
    match active {
        ActiveHud::Walk {
            view,
            gait,
            scale,
            figure_missing,
            ..
        } => {
            let view_name = match view {
                walker::View::First => "First person",
                walker::View::Third if *figure_missing => "Third person · figure not drawn",
                walker::View::Third => "Third person",
            };
            let warn = *view == walker::View::Third && *figure_missing;
            let subtitle = if *gait == walker::Gait::Flying {
                format!("· {view_name} · Flying")
            } else {
                format!("· {view_name}")
            };
            let toggle_label = match view {
                walker::View::First => "Third person",
                walker::View::Third => "First person",
            };
            let metric = Some(format!("Size {}", format_multiplier(*scale)));
            let mut rows = match device {
                Device::Keyboard => vec![
                    row(vec![k(&names.w), k(&names.a), k(&names.s), k(&names.d)], "Move"),
                    row(vec![k("Space")], "Jump"),
                    row(vec![k("Ctrl")], "Sprint"),
                    row(vec![k("Space"), k("Space")], "Fly / stop flying"),
                    row(vec![k(&names.equals), k(&names.minus)], "Grow / shrink"),
                    row(vec![k("Space"), k("Shift")], "Up / down, flying"),
                    row(vec![k(&names.zero)], "Normal size"),
                    row(vec![k(&names.v)], toggle_label),
                ],
                Device::Controller => vec![
                    row(vec![b("LS")], "Move · press to sprint"),
                    row(vec![b("A")], "Jump"),
                    row(vec![b("RS")], "Look · press to recentre"),
                    row(vec![b("A"), b("A")], "Fly / stop flying"),
                    row(vec![k("D-pad ↑↓")], "Grow / shrink"),
                    row(vec![b("A"), b("B"), k("RT"), k("LT")], "Up / down, flying"),
                    row(vec![b("X")], "Normal size"),
                    row(vec![b("Y")], toggle_label),
                ],
            };
            if *view == walker::View::Third {
                match device {
                    Device::Keyboard => rows.push(row(vec![k("Wheel")], "Camera distance")),
                    Device::Controller => rows.push(row(vec![k("LB"), k("RB")], "Camera distance")),
                }
            }
            ("Walk".into(), subtitle, metric, warn, rows)
        }
        ActiveHud::FlyFree {
            speed, aircraft, ..
        } => {
            let (subtitle, warn) = match aircraft {
                AircraftHud::Free => ("· Free camera".to_owned(), false),
                AircraftHud::Loading(title) => (format!("· Free camera · loading {title}"), false),
                AircraftHud::Refused(title) => {
                    (format!("· Free camera · {title} can’t be flown"), true)
                }
                AircraftHud::Ready(_) => ("· Free camera".to_owned(), false),
            };
            let metric = Some(format!("Speed {}", format_multiplier(*speed)));
            let rows = match device {
                Device::Keyboard => vec![
                    row(vec![k(&names.w), k(&names.a), k(&names.s), k(&names.d)], "Move"),
                    row(vec![k("Space")], "Up"),
                    row(vec![k("Ctrl")], "Boost"),
                    row(vec![k("Shift")], "Down"),
                    row(vec![k(&names.equals), k(&names.minus), note("Wheel")], "Speed"),
                    row(vec![k(&names.zero)], "Starting speed"),
                ],
                Device::Controller => vec![
                    row(vec![b("LS")], "Move · press to boost"),
                    row(vec![b("A"), k("RT")], "Up"),
                    row(vec![b("RS")], "Look · press to level"),
                    row(vec![b("B"), k("LT")], "Down"),
                    row(vec![k("D-pad ↑↓")], "Speed"),
                    row(vec![b("X")], "Starting speed"),
                ],
            };
            ("Fly".into(), subtitle, metric, warn, rows)
        }
        ActiveHud::FlyAircraft { title, view } => {
            let (view_label, next_view) = match view {
                flight::View::Chase => ("Chase view", "Nose view"),
                flight::View::Nose => ("Nose view", "Chase view"),
            };
            let subtitle = format!("· {title} · {view_label}");
            let rows = match device {
                Device::Keyboard => vec![
                    row(vec![k(&names.w), k(&names.s)], "Throttle"),
                    row(vec![k("Mouse"), k("←"), k("→"), k("↑"), k("↓")], "Steer"),
                    row(vec![k(&names.a), k(&names.d)], "Turn"),
                    row(vec![k(&names.c), note("hold")], "Look around"),
                    row(vec![k("Ctrl")], "Boost"),
                    row(vec![k(&names.v)], next_view),
                ],
                Device::Controller => vec![
                    row(vec![k("RT"), k("LT")], "Throttle"),
                    row(vec![b("LS")], "Steer · press to boost"),
                    row(vec![b("RS")], "Look around"),
                    row(vec![b("A")], "Boost"),
                    row(vec![b("Y")], next_view),
                ],
            };
            ("Fly".into(), subtitle, None, false, rows)
        }
    }
}

pub(super) fn format_multiplier(value: f32) -> String {
    if !value.is_finite() || value <= 0.0 {
        return "×1.0".into();
    }
    if value >= 10.0 {
        format!("×{value:.0}")
    } else if value >= 1.0 {
        format!("×{value:.1}")
    } else {
        let digits = (-value.log10().floor()) as usize + 1;
        format!("×{value:.prec$}", prec = digits.min(6))
    }
}

pub(super) fn model_label(name: &str) -> String {
    let mut base = name.trim();
    loop {
        let lower = base.to_ascii_lowercase();
        let next = if lower.ends_with("_entity") || lower.ends_with("-entity") {
            &base[..base.len() - 7]
        } else if lower.ends_with("_mesh") || lower.ends_with("-mesh") {
            &base[..base.len() - 5]
        } else {
            break;
        };
        base = next;
    }
    let words: Vec<&str> = base
        .split(|ch: char| ch == '_' || ch == '-' || ch == '.' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    if words.is_empty() {
        return "Aircraft".into();
    }
    let joined = words.join(" ");
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => "Aircraft".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipliers_format_as_integer_one_decimal_or_two_significant_figures() {
        assert_eq!(format_multiplier(140.4), "×140");
        assert_eq!(format_multiplier(10.0), "×10");
        assert_eq!(format_multiplier(2.5), "×2.5");
        assert_eq!(format_multiplier(1.0), "×1.0");
        assert_eq!(format_multiplier(0.25), "×0.25");
        assert_eq!(format_multiplier(0.0123), "×0.012");
    }

    #[test]
    fn model_labels_strip_entity_and_mesh_suffixes() {
        assert_eq!(
            model_label("ENG_fighter_equipment_1_entity"),
            "ENG fighter equipment 1"
        );
        assert_eq!(model_label("jet_mesh"), "Jet");
        assert_eq!(model_label("_entity"), "Aircraft");
    }
}
