//! Air theatre membership at one frame, as the air superiority widgets read it:
//! each theatre's regions are the mission regions of the wings its groups hold.
//! Complete tables prove an empty theatre; a member whose group or wing is not
//! recorded refuses the whole membership rather than leaving a partial list
//! that looks complete.

use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Every recorded theatre's mission regions, by theatre id.
pub fn regions(frame: &DecodedFrame) -> Result<BTreeMap<u64, BTreeSet<u32>>, String> {
    let theatres = frame
        .complete::<tables::AirTheatres>()
        .map_err(|why| why.to_string())?;
    let groups = frame
        .complete::<tables::AirTheatreGroups>()
        .map_err(|why| why.to_string())?;
    let members = frame
        .complete::<tables::AirTheatreMembers>()
        .map_err(|why| why.to_string())?;
    let wings = frame
        .complete::<tables::AirWings>()
        .map_err(|why| why.to_string())?;
    let wings: HashMap<_, _> = wings
        .iter()
        .map(|wing| (wing.id, wing.mission_region))
        .collect();
    let groups: HashMap<_, _> = groups
        .iter()
        .map(|group| (group.id, group.theatre))
        .collect();
    let mut membership: BTreeMap<u64, BTreeSet<u32>> = theatres
        .iter()
        .map(|theatre| (theatre.id, BTreeSet::new()))
        .collect();
    for member in members {
        let theatre = groups
            .get(&member.group)
            .ok_or("Air-theatre member's group is not recorded")?;
        let region = wings
            .get(&member.wing)
            .ok_or("Air-theatre member's wing is not recorded")?;
        if let Some(region) = region {
            membership
                .get_mut(theatre)
                .ok_or("Air-theatre group's theatre is not recorded")?
                .insert(*region);
        }
    }
    Ok(membership)
}
