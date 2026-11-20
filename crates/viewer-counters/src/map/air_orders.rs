//! What the game gives an observer who right-clicks with air wings selected. The orders are
//! built and dropped unposted; their sounds and the attached-wing confirmation still come.
use replay_data::rows::{AirWingRow, tables};
use replay_frame::frame::DecodedFrame;

/// A base right-click's sound, played before any check.
pub const TRANSFER_SOUND: &str = "order_airwing_effect";
/// A region order a wing accepts.
pub const ACCEPTED_SOUND: &str = "select_aircraft";
/// A region order a wing refuses.
pub const INVALID_SOUND: &str = "order_invalid_effect";
/// The confirmation the orders of wings attached to an orders group wait for.
pub const CONFIRMATION_TITLE: &str = "AIRWING_ATTACHED_MANUAL_ORDER_TITLE";
pub const CONFIRMATION_DESCRIPTION: &str = "AIRWING_ATTACHED_MANUAL_ORDER_DESC";

/// What one right-click gives, in the game's order: sounds, then the confirmation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub sounds: Vec<&'static str>,
    pub confirmation: bool,
    /// What the recording cannot decide, by name.
    pub refused: Vec<String>,
}

fn selected<'a>(frame: &'a DecodedFrame, wings: &[u64]) -> Result<Vec<&'a AirWingRow>, String> {
    let rows = frame.complete::<tables::AirWings>().map_err(|error| format!("air orders: {error}"))?;
    wings
        .iter()
        .map(|&id| {
            rows.binary_search_by_key(&id, |row| row.id)
                .map(|at| &rows[at])
                .map_err(|_| format!("air orders: selected wing {id} is absent from AirWings"))
        })
        .collect()
}

/// Whether a wing waits for confirmation: attached to an orders group.
fn attached(wing: &AirWingRow) -> Result<bool, String> {
    match wing.attached_orders_group {
        Some(group) => Ok(group != 0),
        None => Err(format!("air orders: wing {}'s attached orders group is unavailable", wing.id)),
    }
}

/// A right-click on an air base: its sound whatever is selected, then the confirmation when a
/// selected wing is attached.
pub fn base_right_click(frame: &DecodedFrame, wings: &[u64]) -> Response {
    let mut response = Response { sounds: vec![TRANSFER_SOUND], ..Response::default() };
    match selected(frame, wings).and_then(|rows| rows.into_iter().try_fold(false, |any, wing| Ok(any | attached(wing)?))) {
        Ok(confirmation) => response.confirmation = confirmation,
        Err(why) => response.refused.push(why),
    }
    response
}

/// A right-click on the map's strategic `region` with wings selected. With the order modifier
/// each wing already on a mission there takes an order to leave it, which always passes its
/// check. Without it each wing's assignment is checked (`checks`, in selection order): it sounds
/// accepted or refused, and an accepted wing attached to an orders group waits for confirmation.
pub fn region_right_click(
    frame: &DecodedFrame,
    wings: &[u64],
    region: u32,
    modifier: bool,
    checks: impl FnOnce() -> Vec<Result<bool, String>>,
) -> Response {
    let mut response = Response::default();
    if wings.is_empty() {
        return response;
    }
    let rows = match selected(frame, wings) {
        Ok(rows) => rows,
        Err(why) => {
            response.refused.push(why);
            return response;
        }
    };
    if modifier {
        response.sounds = rows.iter().filter(|wing| wing.mission_region == Some(region)).map(|_| ACCEPTED_SOUND).collect();
        return response;
    }
    for (wing, check) in rows.into_iter().zip(checks()) {
        match check {
            Ok(true) => {
                response.sounds.push(ACCEPTED_SOUND);
                match attached(wing) {
                    Ok(attached) => response.confirmation |= attached,
                    Err(why) => response.refused.push(why),
                }
            }
            Ok(false) => response.sounds.push(INVALID_SOUND),
            Err(why) => response.refused.push(format!("air orders: wing {}'s assignment sound is withheld: {why}", wing.id)),
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::{Metrics, TableView};

    fn frame(wings: Vec<AirWingRow>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 10,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| TableView::Unavailable { reason: "synthetic".into() }.into()),
        };
        *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(wings));
        frame
    }

    fn wing(id: u64, region: Option<u32>, group: Option<u64>) -> AirWingRow {
        AirWingRow { id, mission_region: region, attached_orders_group: group, ..Default::default() }
    }

    #[test]
    fn a_base_plays_its_sound_whatever_is_selected_and_confirms_attached_wings() {
        let frame = frame(vec![wing(1, None, Some(0)), wing(2, None, Some(77)), wing(3, None, None)]);
        assert_eq!(base_right_click(&frame, &[]), Response { sounds: vec![TRANSFER_SOUND], ..Response::default() });
        assert_eq!(base_right_click(&frame, &[1]), Response { sounds: vec![TRANSFER_SOUND], ..Response::default() });
        assert_eq!(base_right_click(&frame, &[1, 2]), Response { sounds: vec![TRANSFER_SOUND], confirmation: true, refused: Vec::new() });
        // An unknown attachment withholds the confirmation by name; the sound still plays.
        let unknown = base_right_click(&frame, &[3]);
        assert_eq!(unknown.sounds, [TRANSFER_SOUND]);
        assert!(!unknown.confirmation);
        assert_eq!(unknown.refused.len(), 1);
    }

    #[test]
    fn the_modifier_order_sounds_once_per_wing_already_in_the_region() {
        let frame = frame(vec![wing(1, Some(5), Some(9)), wing(2, Some(6), Some(0)), wing(3, Some(5), Some(0)), wing(4, None, Some(0))]);
        let unchecked = || -> Vec<Result<bool, String>> { panic!("the modifier order is not checked") };
        let response = region_right_click(&frame, &[1, 2, 3, 4], 5, true, unchecked);
        assert_eq!(response, Response { sounds: vec![ACCEPTED_SOUND; 2], ..Response::default() });
        assert_eq!(region_right_click(&frame, &[], 5, true, unchecked), Response::default());
        assert_eq!(region_right_click(&frame, &[2, 4], 5, true, unchecked), Response::default());
    }

    #[test]
    fn an_assignment_sounds_each_wings_check_and_confirms_accepted_attached_wings() {
        let frame = frame(vec![wing(1, None, Some(9)), wing(2, None, Some(0)), wing(3, None, Some(8)), wing(4, None, None)]);
        let checks = || vec![Ok(false), Ok(true), Err("unknown".to_owned()), Ok(true)];
        let response = region_right_click(&frame, &[1, 2, 3, 4], 6, false, checks);
        // An attached wing whose check fails is refused before confirmation; an unknown check
        // withholds its sound by name; an accepted wing of unknown attachment withholds that.
        assert_eq!(response.sounds, [INVALID_SOUND, ACCEPTED_SOUND, ACCEPTED_SOUND]);
        assert!(!response.confirmation);
        assert_eq!(response.refused.len(), 2);
        let confirmed = region_right_click(&frame, &[2, 3], 6, false, || vec![Ok(true), Ok(true)]);
        assert_eq!(confirmed, Response { sounds: vec![ACCEPTED_SOUND; 2], confirmation: true, refused: Vec::new() });
    }
}
