use crate::combat_markers::LandMarkers;
use crate::combat_markers::LandCombatDefines;
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use super::Movements;
use viewer_engagements::engagement_history::{History, LAND_ICON};
use viewer_foundation::perspective::Vision;
use viewer_foundation::visibility::Relations;
use viewer_engagements::engagement_history::{LAND, Provenance, combats, kept_frames};

pub fn kept_frames_at(history: &History, frame: &DecodedFrame) -> LandMarkers {
    use viewer_foundation::perspective::{Choice, Country};
    let links: BTreeMap<u32, u32> = (1..=4).map(|id| (id, id)).collect();
    let tags: BTreeMap<u32, String> = (1..=4)
        .zip(["AAA", "BBB", "CCC", "DDD"])
        .map(|(id, tag)| (id, tag.to_owned()))
        .collect();
    let choice = Choice {
        country: Country::Tag("AAA".into()),
        fog: true,
    };
    let vision = Vision::read(frame, &links, &tags, &choice, Err("no fog content")).unwrap();
    let relations = Relations::read(frame, &links);
    let relations = relations.as_ref().map_err(|reason| *reason);
    let engaged = combats(frame, LAND, 1, &links, relations, Some(history)).unwrap();
    assert_eq!(engaged.1, Provenance::Indexed);
    let (kept, provenance) = kept_frames(frame, LAND_ICON, 1, &engaged.0, Some(history)).unwrap();
    assert_eq!(provenance, Provenance::Indexed);
    LandMarkers::read(
        frame,
        &vision,
        &links,
        &engaged,
        &kept,
        &Movements::default(),
        &LandCombatDefines::FIXTURE,
    )
    .unwrap()
}
