//! The province order cursor: the game's rule over the first selected unit and the province under
//! the pointer, read at the playback head once each time either, Shift, the viewer or the hour
//! changes. Where the rule does not decide it the arrow stands, its reason counted with the
//! selection's refusals.

use crate::document::Document;
use viewer_foundation::{
    map::movement_reads::{self, OrderCursor},
    unit_handle::Unit,
};
use viewer_playback::cursors::MapCursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    frame: usize,
    hour: replay_data::Hour,
    province: u32,
    first: Unit,
    viewer: u32,
    shift: bool,
}

/// The order cursor last chosen, with what it was chosen for.
#[derive(Default)]
pub(crate) struct Held(Option<(Key, MapCursor)>);

fn shown(cursor: OrderCursor) -> MapCursor {
    match cursor {
        OrderCursor::NoMove => MapCursor::NoMove,
        OrderCursor::FriendlyMove => MapCursor::FriendlyMove,
        OrderCursor::AttackMove => MapCursor::AttackMove,
        OrderCursor::MoveToEnemyProvince => MapCursor::MoveToEnemyProvince,
        OrderCursor::AddUnits => MapCursor::AddUnits,
        OrderCursor::ChainMove => MapCursor::ChainMove,
        OrderCursor::ChainMoveNaval => MapCursor::ChainMoveNaval,
    }
}

impl super::Viewer {
    /// The order cursor over the province at `point` (interface pixels). In the Strategic Air and
    /// Navy modes the game's picker answers a strategic region, which takes the arrow.
    pub(super) fn ground_cursor(&mut self, document: &Document, point: [f32; 2], shift: bool) -> MapCursor {
        use viewer_world::map::map_mode::{MapMode, Stock};
        if matches!(self.shown_mode(), MapMode::Stock(Stock::StrategicAir | Stock::StrategicNavy)) {
            return MapCursor::Normal;
        }
        let Some(&first) = self.unit_selection.ordered_units().first() else {
            return MapCursor::Normal;
        };
        let (Ok(vision), Some(view)) = (self.shown_vision(), self.last_view.as_ref()) else {
            return MapCursor::Normal;
        };
        let screen = point.map(|value| value * view.ui_scale);
        let Ok(Some(province)) = super::pick(&view.camera, screen, view.viewport, &document.map, document.assets.heightmap_texels()) else {
            return MapCursor::Normal;
        };
        let head = super::head_rows::head(super::live_of(&self.playhead_map));
        let reads = document.icons.raids.movement_reads();
        let held = &mut self.selection_input.order;
        let mut refused = None;
        let chosen = self.head_rows.cursor(head, None, |frame| {
            let key = Key { frame: std::ptr::from_ref(frame) as usize, hour: frame.hour, province, first, viewer: vision.ours(), shift };
            if let Some((at, cursor)) = held.0
                && at == key
            {
                return cursor;
            }
            let cursor = movement_reads::order_cursor(frame, reads, first, province, key.viewer, shift).map_or_else(
                |why| {
                    refused = Some(why);
                    MapCursor::Normal
                },
                shown,
            );
            held.0 = Some((key, cursor));
            cursor
        });
        if let Some(why) = refused {
            self.selection_input.refuse(why);
        }
        chosen.unwrap_or(MapCursor::Normal)
    }
}
