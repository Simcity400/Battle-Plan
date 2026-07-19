//! Exact data-driven tooltip composition laws.

use crate::{GameUiError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ScreenRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains_half_open(self, point: (i32, i32)) -> bool {
        self.width > 0
            && self.height > 0
            && point.0 >= self.x
            && point.1 >= self.y
            && point.0 < self.x.saturating_add(self.width)
            && point.1 < self.y.saturating_add(self.height)
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self
            .x
            .saturating_add(self.width)
            .min(other.x.saturating_add(other.width));
        let y1 = self
            .y
            .saturating_add(self.height)
            .min(other.y.saturating_add(other.height));
        (x1 > x0 && y1 > y0).then(|| Self::new(x0, y0, x1 - x0, y1 - y0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TooltipOwner {
    pub element_path: String,
    pub repeat_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitSurface {
    pub rect: ScreenRect,
    pub clip_rect: ScreenRect,
    pub draw_list_ordinal: u32,
    pub z_order: i32,
    pub item_ordinal: u32,
    /// An ownerless winning surface intentionally occludes lower owners.
    pub owner: Option<TooltipOwner>,
}

/// Later draw list, higher z, then later item wins. Hit geometry is the
/// half-open intersection of the item and its clip rectangle.
pub fn frontmost_owner(surfaces: &[HitSurface], point: (i32, i32)) -> Option<&TooltipOwner> {
    surfaces
        .iter()
        .filter(|surface| {
            surface
                .rect
                .intersection(surface.clip_rect)
                .is_some_and(|visible| visible.contains_half_open(point))
        })
        .max_by_key(|surface| {
            (
                surface.draw_list_ordinal,
                surface.z_order,
                surface.item_ordinal,
            )
        })
        .and_then(|surface| surface.owner.as_ref())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooltipTimingLaw {
    pub show_delay_us: u64,
    pub hide_delay_us: u64,
    pub delayed_delay_us: u64,
}

impl TooltipTimingLaw {
    pub fn from_graphics_defines(source: &str) -> Result<Self> {
        Ok(Self {
            show_delay_us: parse_seconds(source, "TOOLTIP_SHOW_DELAY")?,
            hide_delay_us: parse_seconds(source, "TOOLTIP_HIDE_DELAY")?,
            delayed_delay_us: parse_seconds(source, "TOOLTIP_DELAYED_DELAY")?,
        })
    }

    fn threshold(self, delayed: bool) -> u64 {
        if delayed {
            self.delayed_delay_us
        } else {
            self.show_delay_us
        }
    }
}

fn parse_seconds(source: &str, key: &str) -> Result<u64> {
    let mut found = source.lines().filter_map(|line| {
        let code = line.split_once("--").map_or(line, |(code, _)| code);
        let (lhs, rhs) = code.split_once('=')?;
        (lhs.trim() == key).then(|| rhs.trim().trim_end_matches(',').trim())
    });
    let raw = found
        .next()
        .ok_or_else(|| GameUiError::missing(format!("missing graphics define {key}")))?;
    if found.next().is_some() {
        return Err(GameUiError::parse(format!(
            "duplicate graphics define {key}"
        )));
    }
    let seconds: f64 = raw
        .parse()
        .map_err(|_| GameUiError::parse(format!("invalid {key} value")))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(GameUiError::parse(format!("invalid {key} seconds")));
    }
    Ok((seconds * 1_000_000.0).round() as u64)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverCandidate {
    pub owner: TooltipOwner,
    pub delayed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipLifecycle {
    timing: TooltipTimingLaw,
    candidate: Option<HoverCandidate>,
    candidate_elapsed_us: u64,
    visible: Option<TooltipOwner>,
    leave_elapsed_us: u64,
}

impl TooltipLifecycle {
    pub fn new(timing: TooltipTimingLaw) -> Self {
        Self {
            timing,
            candidate: None,
            candidate_elapsed_us: 0,
            visible: None,
            leave_elapsed_us: 0,
        }
    }

    pub fn visible_owner(&self) -> Option<&TooltipOwner> {
        self.visible.as_ref()
    }

    pub fn advance(&mut self, next: Option<HoverCandidate>, delta_us: u64) {
        if next != self.candidate {
            self.candidate = next;
            self.candidate_elapsed_us = 0;
        } else if self.candidate.is_some() {
            self.candidate_elapsed_us = self.candidate_elapsed_us.saturating_add(delta_us);
        }
        if let Some(candidate) = &self.candidate {
            self.leave_elapsed_us = 0;
            if self.candidate_elapsed_us >= self.timing.threshold(candidate.delayed) {
                self.visible = Some(candidate.owner.clone());
            }
        } else if self.visible.is_some() {
            self.leave_elapsed_us = self.leave_elapsed_us.saturating_add(delta_us);
            if self.leave_elapsed_us >= self.timing.hide_delay_us {
                self.visible = None;
                self.leave_elapsed_us = 0;
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlacementEvidence {
    ExtractedEngineRect { producer_record: String },
    RenderDocOracle { capture_sha256: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPlacement {
    pub screen_rect: ScreenRect,
    pub evidence: PlacementEvidence,
}

impl ResolvedPlacement {
    /// Preserve engine origin and size; only scissor to the viewport. Unknown
    /// cursor placement is never replaced by a guessed offset or clamp.
    pub fn clipped_to(&self, viewport: ScreenRect) -> Result<(ScreenRect, ScreenRect)> {
        if self.screen_rect.width <= 0 || self.screen_rect.height <= 0 {
            return Err(GameUiError::parse("tooltip screen rectangle is empty"));
        }
        let clipped = self
            .screen_rect
            .intersection(viewport)
            .ok_or_else(|| GameUiError::missing("tooltip is fully outside viewport"))?;
        Ok((self.screen_rect, clipped))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowAvailability {
    Resolved,
    NotApplicable,
    DataBlocked { field: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TooltipLineKind {
    Header,
    Description,
    Value,
    Breakdown,
    Scripted,
    Spacer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipRow {
    pub source_ordinal: u32,
    pub line_kind: TooltipLineKind,
    pub source_identity: String,
    pub evaluation_context: String,
    pub availability: RowAvailability,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipPayload {
    pub owner: TooltipOwner,
    pub rows: Vec<TooltipRow>,
}

impl TooltipPayload {
    /// Validate in producer order. Replay neither sorts nor evaluates rows.
    pub fn validate(&self) -> Result<()> {
        let mut prior = None;
        for row in &self.rows {
            if prior.is_some_and(|value| row.source_ordinal <= value) {
                return Err(GameUiError::parse(
                    "tooltip row ordinals must be unique and strictly increasing",
                ));
            }
            prior = Some(row.source_ordinal);
            match &row.availability {
                RowAvailability::Resolved if row.text.is_empty() => {
                    return Err(GameUiError::parse("resolved tooltip row has empty text"))
                }
                RowAvailability::NotApplicable if !row.text.is_empty() => {
                    return Err(GameUiError::parse(
                        "not-applicable tooltip row carries text",
                    ))
                }
                RowAvailability::DataBlocked { field } => {
                    return Err(GameUiError::missing(format!(
                        "tooltip upstream field is DATA_BLOCKED: {field}"
                    )))
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TooltipDrawCommand {
    NineSlice {
        rect: ScreenRect,
        clip_rect: ScreenRect,
        texture: String,
        border_px: i32,
    },
    Row {
        source_ordinal: u32,
        text: String,
        line_kind: TooltipLineKind,
    },
}

/// The returned tooltip family is appended after ordinary UI draw families,
/// matching the captured offscreen-tooltip then screen-composite ordering.
pub fn compose_tooltip(
    placement: &ResolvedPlacement,
    viewport: ScreenRect,
    payload: &TooltipPayload,
) -> Result<Vec<TooltipDrawCommand>> {
    payload.validate()?;
    let (rect, clip_rect) = placement.clipped_to(viewport)?;
    let mut commands = vec![TooltipDrawCommand::NineSlice {
        rect,
        clip_rect,
        texture: "gfx/interface/tiles/tiles_dialog.dds".into(),
        border_px: 18,
    }];
    commands.extend(payload.rows.iter().filter_map(|row| {
        matches!(row.availability, RowAvailability::Resolved).then(|| TooltipDrawCommand::Row {
            source_ordinal: row.source_ordinal,
            text: row.text.clone(),
            line_kind: row.line_kind.clone(),
        })
    }));
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(name: &str) -> TooltipOwner {
        TooltipOwner {
            element_path: name.into(),
            repeat_index: 0,
        }
    }
    fn timing() -> TooltipTimingLaw {
        TooltipTimingLaw {
            show_delay_us: 50_000,
            hide_delay_us: 50_000,
            delayed_delay_us: 1_000_000,
        }
    }

    #[test]
    fn exact_timing_and_lifecycle_boundaries() {
        let source =
            "TOOLTIP_DELAYED_DELAY = 1\nTOOLTIP_SHOW_DELAY = 0.05\nTOOLTIP_HIDE_DELAY = 0.05\n";
        assert_eq!(
            TooltipTimingLaw::from_graphics_defines(source).unwrap(),
            timing()
        );
        let mut life = TooltipLifecycle::new(timing());
        let a = HoverCandidate {
            owner: owner("a"),
            delayed: false,
        };
        life.advance(Some(a.clone()), 9_000_000); // entry resets clock
        life.advance(Some(a), 49_999);
        assert_eq!(life.visible_owner(), None);
        life.advance(
            Some(HoverCandidate {
                owner: owner("a"),
                delayed: false,
            }),
            1,
        );
        assert_eq!(life.visible_owner(), Some(&owner("a")));
        let b = HoverCandidate {
            owner: owner("b"),
            delayed: true,
        };
        life.advance(Some(b.clone()), 9_000_000); // owner change resets
        life.advance(Some(b.clone()), 999_999);
        assert_eq!(life.visible_owner(), Some(&owner("a")));
        life.advance(Some(b), 1);
        assert_eq!(life.visible_owner(), Some(&owner("b")));
        life.advance(None, 0);
        life.advance(None, 49_999);
        assert_eq!(life.visible_owner(), Some(&owner("b")));
        life.advance(None, 1);
        assert_eq!(life.visible_owner(), None);
    }

    #[test]
    fn collision_clipping_stacking_and_ownerless_occlusion() {
        let r = ScreenRect::new(0, 0, 100, 100);
        let mut surfaces = vec![
            HitSurface {
                rect: r,
                clip_rect: r,
                draw_list_ordinal: 0,
                z_order: 99,
                item_ordinal: 9,
                owner: Some(owner("old")),
            },
            HitSurface {
                rect: r,
                clip_rect: ScreenRect::new(0, 0, 50, 100),
                draw_list_ordinal: 1,
                z_order: 0,
                item_ordinal: 0,
                owner: Some(owner("clipped")),
            },
            HitSurface {
                rect: r,
                clip_rect: r,
                draw_list_ordinal: 1,
                z_order: 1,
                item_ordinal: 1,
                owner: Some(owner("front")),
            },
        ];
        assert_eq!(frontmost_owner(&surfaces, (99, 99)), Some(&owner("front")));
        assert_eq!(frontmost_owner(&surfaces, (100, 99)), None);
        surfaces.push(HitSurface {
            rect: r,
            clip_rect: r,
            draw_list_ordinal: 2,
            z_order: 0,
            item_ordinal: 0,
            owner: None,
        });
        assert_eq!(frontmost_owner(&surfaces, (10, 10)), None);
    }

    #[test]
    fn capture_rects_chrome_clipping_and_order_are_exact() {
        for rect in [
            ScreenRect::new(1223, 671, 297, 148),
            ScreenRect::new(1238, 876, 291, 164),
        ] {
            let payload = TooltipPayload {
                owner: owner("map"),
                rows: vec![
                    TooltipRow {
                        source_ordinal: 10,
                        line_kind: TooltipLineKind::Header,
                        source_identity: "header".into(),
                        evaluation_context: "province".into(),
                        availability: RowAvailability::Resolved,
                        text: "Header".into(),
                    },
                    TooltipRow {
                        source_ordinal: 20,
                        line_kind: TooltipLineKind::Scripted,
                        source_identity: "breakdown[0]".into(),
                        evaluation_context: "country=GER".into(),
                        availability: RowAvailability::Resolved,
                        text: "Dynamic".into(),
                    },
                ],
            };
            let commands = compose_tooltip(
                &ResolvedPlacement {
                    screen_rect: rect,
                    evidence: PlacementEvidence::RenderDocOracle {
                        capture_sha256: "authenticated".into(),
                    },
                },
                ScreenRect::new(0, 0, 1920, 1080),
                &payload,
            )
            .unwrap();
            assert_eq!(
                commands[0],
                TooltipDrawCommand::NineSlice {
                    rect,
                    clip_rect: rect,
                    texture: "gfx/interface/tiles/tiles_dialog.dds".into(),
                    border_px: 18
                }
            );
            assert!(matches!(
                commands[1],
                TooltipDrawCommand::Row {
                    source_ordinal: 10,
                    ..
                }
            ));
            assert!(matches!(
                commands[2],
                TooltipDrawCommand::Row {
                    source_ordinal: 20,
                    ..
                }
            ));
        }
        let p = ResolvedPlacement {
            screen_rect: ScreenRect::new(1900, 1060, 100, 60),
            evidence: PlacementEvidence::ExtractedEngineRect {
                producer_record: "x".into(),
            },
        };
        assert_eq!(
            p.clipped_to(ScreenRect::new(0, 0, 1920, 1080)).unwrap().1,
            ScreenRect::new(1900, 1060, 20, 20)
        );
    }

    #[test]
    fn data_blocked_and_reordered_content_fail_closed() {
        let row = TooltipRow {
            source_ordinal: 1,
            line_kind: TooltipLineKind::Scripted,
            source_identity: "script".into(),
            evaluation_context: "country=GER".into(),
            availability: RowAvailability::DataBlocked {
                field: "resolved_scripted_text".into(),
            },
            text: String::new(),
        };
        let mut p = TooltipPayload {
            owner: owner("a"),
            rows: vec![row],
        };
        assert!(p
            .validate()
            .unwrap_err()
            .to_string()
            .contains("DATA_BLOCKED"));
        p.rows = vec![
            TooltipRow {
                source_ordinal: 2,
                line_kind: TooltipLineKind::Header,
                source_identity: "b".into(),
                evaluation_context: "x".into(),
                availability: RowAvailability::Resolved,
                text: "b".into(),
            },
            TooltipRow {
                source_ordinal: 1,
                line_kind: TooltipLineKind::Header,
                source_identity: "a".into(),
                evaluation_context: "x".into(),
                availability: RowAvailability::Resolved,
                text: "a".into(),
            },
        ];
        assert!(p.validate().is_err());
    }
}
