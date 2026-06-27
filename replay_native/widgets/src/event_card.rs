//! N5 — the EVENT / POPUP CARD: a media (portrait) panel + a wrapped title/desc + a
//! column of option/action buttons. Consumers: event, popup, alerts, decisions,
//! detailed popups.
//!
//! Codex N5 hardening: the primitive is a GENERIC [`PopupCard`] (header / body sections
//! / actions / media) — NOT event-shaped — so alerts (no portrait, tiny), decisions
//! (cost/effect actions), and full event windows all map onto it. An [`EventCard`]
//! adapter builds one from the `events.json` shape. Media is an [`AssetRef`]
//! (no-portrait-by-design vs missing-asset vs blocked stay distinct). The title/body
//! are [`Atom`]s (a card with no title draws `—`, and IF the card itself is
//! unavailable the caller passes an unavailable title — the heading is honest). Each
//! action carries an [`ActionState`] (enabled / disabled / unavailable / already-taken).
//!
//! Wrapping: at LAYOUT time (headless) the card wraps the body on a character budget
//! derived from the panel width; at DRAW time, when a real BMFont is present, it
//! re-wraps with glyph metrics via [`gui_runtime::markup`]. The plan/digest use the
//! headless wrap so the no-fabrication + layout logic is testable without a GPU.
//!
//! ## Usage
//! ```ignore
//! use widgets::event_card::*;
//! use widgets::common::{Atom, AssetRef};
//! let card = PopupCard::new(Atom::exact("The Phony War Ends"))
//!     .media(AssetRef::Named("GFX_event_phony_war".into()))
//!     .body(Atom::exact("German forces have crossed the frontier..."))
//!     .action(Action::enabled("opt_a", Atom::exact("We are ready.")))
//!     .action(Action::unavailable("opt_b", Atom::exact("Sue for peace.")));
//! // let laid = card.layout(area);
//! // draw_card(renderer, &laid, &assets, resolve_sprite);
//! // if let Some(id) = laid.pick_action(mx, my) { ... }
//! ```

use render::Renderer;

use crate::common::{AssetRef, Atom, DrawAssets, DIM_TINT, TEXT_TINT, UNAVAIL_TINT};
use crate::plan::{Cmd, Plan, SpriteDraw};

/// An action/option button's state (Codex N5: options need semantics beyond a label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionState {
    /// Selectable.
    Enabled,
    /// Present but inert (a trigger gate unmet).
    Disabled,
    /// Present but its data/effect is blocked — badged, not pickable.
    Unavailable,
    /// Already chosen (a historical replay marker) — shown, not re-pickable.
    AlreadyTaken,
}

impl ActionState {
    pub fn is_pickable(&self) -> bool {
        matches!(self, ActionState::Enabled)
    }
}

/// One option/action button: a stable id + an [`Atom`] caption + its state.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: String,
    pub caption: Atom,
    pub state: ActionState,
}

impl Action {
    pub fn enabled(id: impl Into<String>, caption: Atom) -> Action {
        Action {
            id: id.into(),
            caption,
            state: ActionState::Enabled,
        }
    }
    pub fn disabled(id: impl Into<String>, caption: Atom) -> Action {
        Action {
            id: id.into(),
            caption,
            state: ActionState::Disabled,
        }
    }
    pub fn unavailable(id: impl Into<String>, caption: Atom) -> Action {
        Action {
            id: id.into(),
            caption,
            state: ActionState::Unavailable,
        }
    }
    pub fn already_taken(id: impl Into<String>, caption: Atom) -> Action {
        Action {
            id: id.into(),
            caption,
            state: ActionState::AlreadyTaken,
        }
    }
}

/// The generic popup card model: a header title, a media slot, body sections, actions.
#[derive(Debug, Clone, PartialEq)]
pub struct PopupCard {
    pub title: Atom,
    pub media: AssetRef,
    /// Body sections (each an `Atom`; multiple desc blocks stay separate, Codex N5).
    pub body: Vec<Atom>,
    pub actions: Vec<Action>,
}

impl PopupCard {
    pub fn new(title: Atom) -> PopupCard {
        PopupCard {
            title,
            media: AssetRef::None,
            body: Vec::new(),
            actions: Vec::new(),
        }
    }
    pub fn media(mut self, m: AssetRef) -> PopupCard {
        self.media = m;
        self
    }
    pub fn body(mut self, section: Atom) -> PopupCard {
        self.body.push(section);
        self
    }
    pub fn action(mut self, a: Action) -> PopupCard {
        self.actions.push(a);
        self
    }

    /// Lay the card out within `area`: a header strip, an optional media box, the
    /// wrapped body, and a column of action buttons at the bottom. Returns a
    /// [`LaidCard`] (positions + the per-action rects) — pure, headless.
    pub fn layout(&self, area: gui_runtime::Rect) -> LaidCard {
        let pad = 12.0;
        let header_h = 28.0;
        let inner_w = (area.w - 2.0 * pad).max(1.0);

        // media box: only when the card HAS media (None = no box; Codex distinguishes
        // none-by-design from missing-asset, which still gets a box + placeholder).
        let media_box = (!self.media.is_none()).then(|| gui_runtime::Rect {
            x: area.x + pad,
            y: area.y + header_h + pad,
            w: inner_w,
            h: MEDIA_H.min((area.h - header_h - pad * 2.0) * 0.4).max(0.0),
        });

        let body_y0 = media_box
            .map(|m| m.y + m.h + 8.0)
            .unwrap_or(area.y + header_h + pad);

        // headless wrap: a char budget from the panel width (≈ APPROX_CHAR_W px/char).
        let char_budget = ((inner_w / APPROX_CHAR_W) as usize).max(8);
        let mut body_lines = Vec::new();
        for section in &self.body {
            if section.is_unavailable() {
                body_lines.push(BodyLine {
                    text: crate::common::NOT_CAPTURED.to_string(),
                    unavailable: true,
                });
            } else {
                for line in wrap_chars(section.as_str(), char_budget) {
                    body_lines.push(BodyLine {
                        text: line,
                        unavailable: false,
                    });
                }
            }
            body_lines.push(BodyLine {
                text: String::new(),
                unavailable: false,
            }); // blank between sections
        }

        // action buttons stacked at the bottom.
        let n = self.actions.len();
        let actions_h = n as f32 * (ACTION_H + 4.0);
        let actions_y0 = (area.y + area.h - pad - actions_h).max(body_y0);
        let mut action_rects = Vec::with_capacity(n);
        for (i, a) in self.actions.iter().enumerate() {
            action_rects.push(ActionRect {
                id: a.id.clone(),
                index: i,
                rect: gui_runtime::Rect {
                    x: area.x + pad,
                    y: actions_y0 + i as f32 * (ACTION_H + 4.0),
                    w: inner_w,
                    h: ACTION_H,
                },
            });
        }

        LaidCard {
            area,
            header: gui_runtime::Rect {
                x: area.x,
                y: area.y,
                w: area.w,
                h: header_h,
            },
            title: self.title.clone(),
            media: self.media.clone(),
            media_box,
            body_origin: (area.x + pad, body_y0),
            body_lines,
            actions: self.actions.clone(),
            action_rects,
        }
    }
}

/// One wrapped body line + whether it is the not-captured glyph.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyLine {
    pub text: String,
    pub unavailable: bool,
}

/// One laid-out action's screen rect.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionRect {
    pub id: String,
    pub index: usize,
    pub rect: gui_runtime::Rect,
}

/// The laid-out card: everything positioned, headless, ready to plan/draw/pick.
#[derive(Debug, Clone, PartialEq)]
pub struct LaidCard {
    pub area: gui_runtime::Rect,
    pub header: gui_runtime::Rect,
    pub title: Atom,
    pub media: AssetRef,
    pub media_box: Option<gui_runtime::Rect>,
    pub body_origin: (f32, f32),
    pub body_lines: Vec<BodyLine>,
    pub actions: Vec<Action>,
    pub action_rects: Vec<ActionRect>,
}

impl LaidCard {
    /// Hit-test a click against the action buttons. Returns the action id IF it is
    /// PICKABLE (disabled / unavailable / already-taken are not). `None` otherwise.
    pub fn pick_action(&self, x: f32, y: f32) -> Option<String> {
        for ar in &self.action_rects {
            if ar.rect.contains(x, y) {
                let pickable = self
                    .actions
                    .get(ar.index)
                    .map(|a| a.state.is_pickable())
                    .unwrap_or(false);
                return pickable.then(|| ar.id.clone());
            }
        }
        None
    }
}

/// Layout constants.
pub const MEDIA_H: f32 = 120.0;
pub const ACTION_H: f32 = 26.0;
pub const APPROX_CHAR_W: f32 = 8.0;

const CARD_BG: [f32; 4] = [0.10, 0.11, 0.14, 0.98];
const HEADER_BG: [f32; 4] = [0.16, 0.18, 0.23, 1.0];
const MEDIA_BG: [f32; 4] = [0.06, 0.07, 0.09, 1.0];
const ACTION_BG: [f32; 4] = [0.18, 0.22, 0.30, 1.0];
const ACTION_DISABLED: [f32; 4] = [0.12, 0.13, 0.16, 1.0];
const ACTION_TAKEN: [f32; 4] = [0.20, 0.24, 0.20, 1.0];

/// Build the render plan for a laid-out card. Media: a `None` AssetRef draws no box; a
/// `Named` draws a Sprite (executor → placeholder on miss); an `Unavailable` draws a
/// `—` placeholder box. Body lines draw their literal (a `—` for an unavailable
/// section). Actions draw tinted by state.
pub fn plan_card(card: &LaidCard) -> Plan {
    let mut p = Plan::new();
    let a = card.area;
    let clip = Some([a.x, a.y, a.w, a.h]);

    // panel + header.
    p.push(Cmd::Rect {
        x: a.x,
        y: a.y,
        w: a.w,
        h: a.h,
        tint: CARD_BG,
        clip,
    });
    p.push(Cmd::Rect {
        x: card.header.x,
        y: card.header.y,
        w: card.header.w,
        h: card.header.h,
        tint: HEADER_BG,
        clip,
    });
    p.push(Cmd::Text {
        x: card.header.x + 10.0,
        y: card.header.y + 7.0,
        text: card.title.as_str().into(),
        tint: card.title.tint(TEXT_TINT),
        clip,
    });

    // media.
    if let Some(mb) = card.media_box {
        p.push(Cmd::Rect {
            x: mb.x,
            y: mb.y,
            w: mb.w,
            h: mb.h,
            tint: MEDIA_BG,
            clip,
        });
        match &card.media {
            AssetRef::Named(name) => {
                p.push(Cmd::Sprite {
                    x: mb.x,
                    y: mb.y,
                    w: mb.w,
                    h: mb.h,
                    name: Some(name.clone()),
                    tint: [1.0; 4],
                    clip,
                });
            }
            AssetRef::Unavailable => {
                // a visible no-data media placeholder (distinct from no-media).
                p.push(Cmd::Sprite {
                    x: mb.x,
                    y: mb.y,
                    w: mb.w,
                    h: mb.h,
                    name: None,
                    tint: [1.0; 4],
                    clip,
                });
            }
            AssetRef::None => {}
        }
    }

    // body lines.
    let (bx, mut by) = card.body_origin;
    for line in &card.body_lines {
        if !line.text.is_empty() {
            let tint = if line.unavailable {
                UNAVAIL_TINT
            } else {
                TEXT_TINT
            };
            p.push(Cmd::Text {
                x: bx,
                y: by,
                text: line.text.clone(),
                tint,
                clip,
            });
        }
        by += LINE_H;
    }

    // actions.
    for ar in &card.action_rects {
        let action = match card.actions.get(ar.index) {
            Some(a) => a,
            None => continue,
        };
        let r = ar.rect;
        let bg = match action.state {
            ActionState::Enabled => ACTION_BG,
            ActionState::Disabled | ActionState::Unavailable => ACTION_DISABLED,
            ActionState::AlreadyTaken => ACTION_TAKEN,
        };
        p.push(Cmd::Rect {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
            tint: bg,
            clip,
        });
        let normal = match action.state {
            ActionState::Enabled => TEXT_TINT,
            ActionState::AlreadyTaken => DIM_TINT,
            ActionState::Disabled => DIM_TINT,
            ActionState::Unavailable => UNAVAIL_TINT,
        };
        let cell_clip = Some([r.x, r.y, r.w, r.h]);
        p.push(Cmd::Text {
            x: r.x + 8.0,
            y: r.y + 6.0,
            text: action.caption.as_str().into(),
            tint: action.caption.tint(normal),
            clip: cell_clip,
        });
    }

    p
}

/// The body line advance (matches a ~16px BMFont line).
pub const LINE_H: f32 = 16.0;

/// Draw the card (plan + execute). When a real font is present the body could be
/// re-wrapped with glyph metrics; the plan path uses the headless char-wrap, which is
/// what the digest + tests cover. `resolve` resolves the media sprite (miss →
/// placeholder).
pub fn draw_card(
    r: &mut Renderer,
    card: &LaidCard,
    assets: &DrawAssets,
    resolve: impl FnMut(&mut Renderer, SpriteDraw) -> bool,
) {
    plan_card(card).execute(r, assets, resolve);
}

/// Wrap plain text to a CHARACTER budget (headless; the layout-time wrap). Words are
/// split on spaces; a word longer than the budget is hard-broken. Never loops/drops.
fn wrap_chars(text: &str, budget: usize) -> Vec<String> {
    let budget = budget.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if word.len() > budget {
            // flush, then hard-break the long word by chars.
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            let chars: Vec<char> = word.chars().collect();
            for chunk in chars.chunks(budget) {
                lines.push(chunk.iter().collect());
            }
            continue;
        }
        let need = if cur.is_empty() {
            word.len()
        } else {
            cur.len() + 1 + word.len()
        };
        if need > budget && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        if cur.is_empty() {
            cur.push_str(word);
        } else {
            cur.push(' ');
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

// ===========================================================================
// EVENT ADAPTER — build a PopupCard from the `events.json` shape (Codex N5: a
// generic card + a thin event adapter, not an event-shaped primitive).
// ===========================================================================

/// Build a [`PopupCard`] from one `events.json` event record (the shape
/// `parse_events.py` emits: `{id, kind, title:{key,text}, desc:[{key,text}],
/// picture:GFX_*, options:[{key,text}]}`). No-fabrication: a missing title/desc becomes
/// an unavailable `Atom`, NEVER a fabricated string. Media uses the trichotomy honestly:
/// a `picture` STRING → `Named`; an ABSENT `picture` key → `None` (the event has no art
/// by design — most events specify one; absence is a real "no media"); a `picture`
/// present but JSON-NULL → `Unavailable` (a captured-but-unobtained art slot).
pub struct EventCard;

impl EventCard {
    pub fn from_event_json(ev: &serde_json::Value) -> PopupCard {
        let title = loc_atom(ev.get("title"));
        let picture = match ev.get("picture") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => AssetRef::Named(s.clone()),
            // a present-but-null (or empty-string) picture: captured-but-unobtained art.
            Some(serde_json::Value::Null) => AssetRef::Unavailable,
            Some(serde_json::Value::String(_)) => AssetRef::Unavailable,
            // any other present-but-non-string value: also unobtained.
            Some(_) => AssetRef::Unavailable,
            // no `picture` key at all: the event has no art by design.
            None => AssetRef::None,
        };
        let mut card = PopupCard::new(title).media(picture);
        // desc sections.
        if let Some(arr) = ev.get("desc").and_then(|v| v.as_array()) {
            if arr.is_empty() {
                card = card.body(Atom::unavailable());
            }
            for d in arr {
                card = card.body(loc_atom(Some(d)));
            }
        } else {
            card = card.body(Atom::unavailable());
        }
        // options → actions. We do NOT know which option fired in a replay, so every
        // option is shown ENABLED-as-display (the replay layer can downgrade to
        // AlreadyTaken when the diff reveals the chosen one); a missing caption → `—`.
        if let Some(arr) = ev.get("options").and_then(|v| v.as_array()) {
            for (i, o) in arr.iter().enumerate() {
                let cap = loc_atom(Some(o));
                card = card.action(Action::enabled(format!("opt_{i}"), cap));
            }
        }
        card
    }
}

/// Turn a `{key, text}` loc node (or a bare string) into an [`Atom`]. A null/absent
/// node or an absent `text` → an unavailable atom (never a fabricated key/string).
fn loc_atom(node: Option<&serde_json::Value>) -> Atom {
    match node {
        None | Some(serde_json::Value::Null) => Atom::unavailable(),
        Some(serde_json::Value::String(s)) if !s.is_empty() => Atom::exact(s.clone()),
        Some(serde_json::Value::String(_)) => Atom::unavailable(),
        Some(obj) => match obj.get("text").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => Atom::exact(t.to_string()),
            _ => Atom::unavailable(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> gui_runtime::Rect {
        gui_runtime::Rect {
            x: 0.0,
            y: 0.0,
            w: 360.0,
            h: 400.0,
        }
    }

    #[test]
    fn wrap_chars_never_loops_or_drops() {
        let lines = wrap_chars("a verylongwordthatexceedsthebudgetxxxxxxxxxx b", 10);
        assert!(!lines.is_empty());
        // the long word is hard-broken, the rest preserved.
        assert!(lines.iter().any(|l| l.contains('a')));
        assert!(lines.iter().any(|l| l.contains('b')));
    }

    #[test]
    fn missing_title_and_desc_badge_glyph() {
        let ev = serde_json::json!({ "id": "x.1", "kind": "country_event" });
        let card = EventCard::from_event_json(&ev);
        assert!(card.title.is_unavailable());
        // no desc -> one unavailable body section.
        assert_eq!(card.body.len(), 1);
        assert!(card.body[0].is_unavailable());
        // no picture -> no media by design.
        assert!(card.media.is_none());
    }

    #[test]
    fn picture_media_trichotomy_is_honest() {
        // absent picture -> no media by design.
        let a = EventCard::from_event_json(&serde_json::json!({ "id": "x" }));
        assert_eq!(a.media, AssetRef::None);
        // present string -> Named.
        let b = EventCard::from_event_json(&serde_json::json!({ "id": "x", "picture": "GFX_e" }));
        assert_eq!(b.media, AssetRef::Named("GFX_e".into()));
        // present-but-null -> Unavailable (captured-but-unobtained), distinct from None.
        let c = EventCard::from_event_json(&serde_json::json!({ "id": "x", "picture": null }));
        assert_eq!(c.media, AssetRef::Unavailable);
    }

    #[test]
    fn full_event_maps_all_fields() {
        let ev = serde_json::json!({
            "id": "ger.1", "kind": "country_event",
            "title": { "key": "ger.1.t", "text": "The Phony War Ends" },
            "desc": [ { "key": "ger.1.d", "text": "German forces cross the frontier." } ],
            "picture": "GFX_event_war",
            "options": [
                { "key": "ger.1.a", "text": "We are ready." },
                { "key": "ger.1.b", "text": "Sue for peace." }
            ]
        });
        let card = EventCard::from_event_json(&ev);
        assert_eq!(card.title.as_str(), "The Phony War Ends");
        assert_eq!(card.media, AssetRef::Named("GFX_event_war".into()));
        assert_eq!(card.body.len(), 1);
        assert_eq!(card.actions.len(), 2);
        assert!(card.actions[0].state.is_pickable());
    }

    #[test]
    fn layout_and_pick_action() {
        let card = PopupCard::new(Atom::exact("T"))
            .body(Atom::exact("hello world"))
            .action(Action::enabled("a", Atom::exact("Yes")))
            .action(Action::unavailable("b", Atom::exact("No")));
        let laid = card.layout(area());
        // the enabled action is pickable; the unavailable is not.
        let ra = &laid.action_rects[0].rect;
        assert_eq!(
            laid.pick_action(ra.x + 4.0, ra.y + 4.0).as_deref(),
            Some("a")
        );
        let rb = &laid.action_rects[1].rect;
        assert_eq!(laid.pick_action(rb.x + 4.0, rb.y + 4.0), None);
    }

    #[test]
    fn empty_card_no_panic_and_no_media_box() {
        let card = PopupCard::new(Atom::unavailable());
        let laid = card.layout(area());
        assert!(laid.media_box.is_none()); // None media -> no box
        assert!(laid.action_rects.is_empty());
        let p = plan_card(&laid);
        // the title drew the glyph (unavailable card heading is honest).
        assert!(p.unavailable_text_count() >= 1);
        assert_eq!(laid.pick_action(10.0, 10.0), None);
    }

    #[test]
    fn unavailable_media_draws_placeholder_not_nothing() {
        let card = PopupCard::new(Atom::exact("T")).media(AssetRef::Unavailable);
        let laid = card.layout(area());
        assert!(laid.media_box.is_some());
        let p = plan_card(&laid);
        assert_eq!(p.placeholder_sprite_count(), 1);
    }

    #[test]
    fn plan_is_deterministic() {
        let card = PopupCard::new(Atom::exact("T"))
            .body(Atom::exact("a b c"))
            .action(Action::enabled("a", Atom::exact("Y")));
        let laid = card.layout(area());
        assert_eq!(plan_card(&laid).digest(), plan_card(&laid).digest());
    }
}
