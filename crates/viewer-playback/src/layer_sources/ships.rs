use viewer_layers_forces::ships::{Roster, Naval};
use crate::document::Content;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_gpu::camera::{MapDefines, MapSize};
use viewer_unit_reads::counter_move::{RouteWorld, UnitRoute};
use viewer_foundation::perspective::Vision;
use viewer_unit_models::{equipment_graphics::{Decided, LimitValues, Limits}, unit_models::{Force, ForcePart, ShipGround}};

impl Naval for Content {
    fn drawn_at(&self, height: f32) -> bool {
        self.unit_models.gates.ships(height)
    }

    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]> {
        &self.unit_models.bounds
    }

    fn map(&self) -> (MapSize, MapDefines) {
        (MapSize::of(&self.map), self.map_defines)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.counters.region(province)
    }

    fn figure(&self, force: &Force, route: Result<UnitRoute<'_>, &'static str>, link: &dyn Fn(u32) -> Option<u32>, vision: &Vision, skip: &dyn Fn(u32) -> Option<bool>, ordinal: Option<u32>, speed: Option<i32>, day: i32, roster: &mut dyn FnMut() -> Roster) -> ForcePart {
        let counters = &self.counters;
        let movement = self.movement.as_deref();
        let heights = viewer_gpu::province_map::HeightField { width: self.assets.heightmap.width, height: self.assets.heightmap.height, values: self.assets.heightmap_texels() };
        let world = movement.map(|movement| RouteWorld { map: &movement.map, heights, width: self.map.width as f32 });
        let definitions = &self.map.definitions;
        let land = |province| match movement.and_then(|movement| movement.map.terrain(province)) {
            Some(terrain) => terrain.flags & 1 != 0,
            None => matches!(definitions.get(&province).map(|it| it.kind), Some(content_reader_map::map::ProvinceKind::Land)),
        };
        let anchor = |province, slot| counters.anchor(province, slot);
        let scale = |province, kind| counters.stack_scale(province, kind);
        let ground = ShipGround { movement, world: world.as_ref(), anchor: &anchor, land: &land, scale: &scale };
        self.unit_models.naval_force(force, route, &ground, link, vision, skip, ordinal, speed, day, roster)
    }

    fn rosters(&self, frame: &DecodedFrame, forces: &[u64]) -> io::Result<(Vec<(u64, Roster)>, Decided)> {
        engine_executable::executable_tables::scope(self.tables, || {
            let hour = crate::document::Hour::new(self, frame, 0.0);
            let evaluate = |items: &[content_reader_parser::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
            let values = LimitValues::default();
            let limits = Limits::new(&values, &evaluate);
            let rosters = self.unit_models.resolve_rosters(frame, limits, &self.counters.terrain.combat_terrain, &self.names.localisation, forces)?;
            Ok((rosters, limits.decided()))
        })
    }

    fn decide_the_same(&self, frame: &DecodedFrame, decided: &Decided) -> bool {
        engine_executable::executable_tables::scope(self.tables, || {
            let hour = crate::document::Hour::new(self, frame, 0.0);
            let evaluate = |items: &[content_reader_parser::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
            let values = LimitValues::default();
            Limits::new(&values, &evaluate).decide_the_same(decided)
        })
    }
}
