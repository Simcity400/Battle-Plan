//! COMBAT BUBBLES — a clickable marker at each active combat's `location_province`
//! on the map, owner-aware and sized/styled, plus the click → bubble pick.
//!
//! A bubble's MAP position is the centroid (pixel mean) of its province in the
//! province-id raster. The same `MapCamera` the political map uses then projects
//! that map pixel to a SCREEN pixel; the bubble is drawn there. Picking is the
//! inverse + a radius test (no GPU readback) — the cheap CPU pick the slice uses.
//!
//! No-fabrication: a province absent from the raster yields NO bubble (we never
//! place a marker at a guessed coordinate). The bubble's "owner" colour is the
//! DEFENDER holding tag (derived); when unavailable the bubble uses a neutral
//! contested style, never a fabricated owner colour.

use std::collections::HashMap;

use render::map::ProvinceRaster;

/// Centroid (mean pixel) of each province id in a raster, plus its pixel count.
/// Computed once per raster; reused for every frame's bubble layout.
#[derive(Debug, Clone, Default)]
pub struct ProvinceCentroids {
    /// province id -> (cx, cy) in MAP pixel space.
    centroid: HashMap<u32, (f32, f32)>,
}

impl ProvinceCentroids {
    /// Single linear pass over the raster accumulating per-id pixel sums.
    pub fn from_raster(raster: &ProvinceRaster) -> ProvinceCentroids {
        let mut acc: HashMap<u32, (f64, f64, u64)> = HashMap::new();
        let w = raster.width as usize;
        for (i, &id) in raster.ids.iter().enumerate() {
            if id == 0 {
                continue; // 0 = no province (sea border / undefined)
            }
            let x = (i % w) as f64;
            let y = (i / w) as f64;
            let e = acc.entry(id).or_insert((0.0, 0.0, 0));
            e.0 += x;
            e.1 += y;
            e.2 += 1;
        }
        let centroid = acc
            .into_iter()
            .map(|(id, (sx, sy, n))| {
                let n = n.max(1) as f64;
                (id, ((sx / n) as f32, (sy / n) as f32))
            })
            .collect();
        ProvinceCentroids { centroid }
    }

    /// Map-pixel centroid of a province, `None` if the province has no pixels
    /// (absent from the land raster) — no bubble is placed in that case.
    pub fn get(&self, province: u32) -> Option<(f32, f32)> {
        self.centroid.get(&province).copied()
    }
}

/// The map camera: a map-pixel → screen-pixel affine (pan + uniform zoom). This
/// matches the political map's pan/zoom so bubbles track the same coordinate frame.
#[derive(Debug, Clone, Copy)]
pub struct MapCamera {
    /// Screen pixel = (map_pixel - pan) * zoom + offset.
    pub pan: (f32, f32),
    pub zoom: f32,
    /// Screen offset of the map viewport's top-left.
    pub offset: (f32, f32),
}

impl MapCamera {
    pub fn new(pan: (f32, f32), zoom: f32, offset: (f32, f32)) -> MapCamera {
        MapCamera { pan, zoom, offset }
    }
    /// Project a MAP pixel to a SCREEN pixel.
    pub fn to_screen(&self, mx: f32, my: f32) -> (f32, f32) {
        (
            (mx - self.pan.0) * self.zoom + self.offset.0,
            (my - self.pan.1) * self.zoom + self.offset.1,
        )
    }
}

/// A laid-out combat bubble: where to draw it and what combat it represents.
#[derive(Debug, Clone, PartialEq)]
pub struct Bubble {
    pub combat_id: String,
    /// Screen-space centre.
    pub sx: f32,
    pub sy: f32,
    /// Radius in screen pixels (scaled by the side ref totals — bigger battle,
    /// bigger bubble — clamped to a sane range).
    pub radius: f32,
    /// DEFENDER holding tag (derived) if known, for owner-aware tinting; `None` →
    /// neutral contested style (never a fabricated owner).
    pub defender_tag: Option<String>,
}

/// Lay out bubbles for a set of combats. Each needs the province centroid; a
/// combat whose province is absent from the raster is SKIPPED (no fabricated
/// position). `size_hint(combat_id)` returns the total ref count → bubble size.
pub fn layout_bubbles(
    combats: &[crate::model::CombatVM],
    centroids: &ProvinceCentroids,
    camera: &MapCamera,
) -> Vec<Bubble> {
    let mut out = Vec::new();
    for vm in combats {
        let pid = vm.location_province;
        if pid < 0 {
            continue;
        }
        let c = match centroids.get(pid as u32) {
            Some(c) => c,
            None => continue, // no centroid -> no bubble (no-fabrication)
        };
        let (sx, sy) = camera.to_screen(c.0, c.1);
        let total_refs = vm.attacker.ref_count() + vm.defender.ref_count();
        // size scales gently with engagement size; clamp so it stays readable.
        let radius = (8.0 + (total_refs as f32).sqrt() * 3.0).clamp(8.0, 28.0);
        out.push(Bubble {
            combat_id: vm.combat_id.clone(),
            sx,
            sy,
            radius,
            defender_tag: vm.defender.tag.tag().map(|t| t.to_string()),
        });
    }
    out
}

/// Pick the bubble under a screen point (the nearest whose radius contains the
/// point). Returns its `combat_id`. `None` if the click is on no bubble. This is
/// the bubble-click → open-panel input.
pub fn pick_bubble(bubbles: &[Bubble], x: f32, y: f32) -> Option<String> {
    let mut best: Option<(f32, &Bubble)> = None;
    for b in bubbles {
        let dx = x - b.sx;
        let dy = y - b.sy;
        let d2 = dx * dx + dy * dy;
        if d2 <= b.radius * b.radius {
            if best.map(|(bd, _)| d2 < bd).unwrap_or(true) {
                best = Some((d2, b));
            }
        }
    }
    best.map(|(_, b)| b.combat_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use render::map::ProvinceRaster;

    // a 4x2 raster: left half province 7, right half province 9.
    fn raster() -> ProvinceRaster {
        let ids = vec![7, 7, 9, 9, 7, 7, 9, 9];
        ProvinceRaster { width: 4, height: 2, ids, max_id: 9 }
    }

    #[test]
    fn centroid_is_pixel_mean() {
        let c = ProvinceCentroids::from_raster(&raster());
        // province 7 occupies columns 0,1 over both rows -> cx = 0.5, cy = 0.5
        assert_eq!(c.get(7), Some((0.5, 0.5)));
        // province 9 occupies columns 2,3 -> cx = 2.5, cy = 0.5
        assert_eq!(c.get(9), Some((2.5, 0.5)));
        // unknown province -> no centroid
        assert_eq!(c.get(123), None);
    }

    #[test]
    fn camera_projects_map_to_screen() {
        let cam = MapCamera::new((0.0, 0.0), 2.0, (100.0, 50.0));
        assert_eq!(cam.to_screen(0.5, 0.5), (101.0, 51.0));
    }

    fn vm(combat_id: &str, province: i64, atk: usize, def: usize) -> crate::model::CombatVM {
        use crate::model::*;
        CombatVM {
            combat_id: combat_id.into(),
            kind: "land".into(),
            location_province: province,
            location_name: Field::Unavailable { reason: "" },
            duration_days: Field::Unavailable { reason: "" },
            attacker: SideRoster {
                side: Side::Attacker,
                tag: SideTag::Unavailable,
                refs: vec![[0, 0]; atk],
                refs_present: true,
                division_roster: Field::Unavailable { reason: "" },
            },
            defender: SideRoster {
                side: Side::Defender,
                tag: SideTag::HoldingTagDerived("ENG".into()),
                refs: vec![[0, 0]; def],
                refs_present: true,
                division_roster: Field::Unavailable { reason: "" },
            },
            combat_width: Field::Unavailable { reason: "" },
            terrain: Field::Unavailable { reason: "" },
            progress_bar: Field::Unavailable { reason: "" },
            tactics: Field::Unavailable { reason: "" },
            modifiers_vector: Field::Unavailable { reason: "" },
            air_support: Field::Unavailable { reason: "" },
            leaders: Field::Unavailable { reason: "" },
            division_stat_rows: Field::Unavailable { reason: "" },
        }
    }

    #[test]
    fn layout_skips_provinces_absent_from_raster() {
        let c = ProvinceCentroids::from_raster(&raster());
        let cam = MapCamera::new((0.0, 0.0), 1.0, (0.0, 0.0));
        let combats = vec![vm("a", 7, 1, 7), vm("b", 99999, 1, 1)];
        let bubbles = layout_bubbles(&combats, &c, &cam);
        // province 7 has a centroid -> bubble; 99999 absent -> skipped (no fabrication)
        assert_eq!(bubbles.len(), 1);
        assert_eq!(bubbles[0].combat_id, "a");
        assert_eq!((bubbles[0].sx, bubbles[0].sy), (0.5, 0.5));
        assert_eq!(bubbles[0].defender_tag.as_deref(), Some("ENG"));
    }

    #[test]
    fn pick_finds_bubble_under_point_and_nearest_on_overlap() {
        let bubbles = vec![
            Bubble { combat_id: "a".into(), sx: 10.0, sy: 10.0, radius: 8.0, defender_tag: None },
            Bubble { combat_id: "b".into(), sx: 14.0, sy: 10.0, radius: 8.0, defender_tag: None },
        ];
        // exactly on a's centre -> a
        assert_eq!(pick_bubble(&bubbles, 10.0, 10.0).as_deref(), Some("a"));
        // closer to b -> b (overlap resolved by nearest)
        assert_eq!(pick_bubble(&bubbles, 16.0, 10.0).as_deref(), Some("b"));
        // far away -> nothing
        assert_eq!(pick_bubble(&bubbles, 100.0, 100.0), None);
    }
}
