
#[cfg(test)]
use crate::map_furniture::{vp_icon_relation_column, VpIconDiplomaticPairState};
use crate::map_furniture::{
    VP_STRIP_COLOR_ENEMY_RED, VP_STRIP_COLOR_NEUTRAL_GRAY, VP_STRIP_COLOR_SELF_OR_FACTION,
};
use std::collections::BTreeMap;

pub const DIPLOMACY_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub const DIPLOMACY_VP_STRIP_TEXTURE: &str = "gfx/interface/onmap_victorypoints_strip.dds";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiplomacyRelation {
    SelfCountry,
    SameFaction,
    AtWarEnemy,
    Neutral,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiploPairInput {
    pub at_war: bool,
    pub same_faction: bool,
}

pub fn classify_relation(
    owner_tag: Option<&str>,
    viewed_tag: Option<&str>,
    pair: DiploPairInput,
) -> DiplomacyRelation {
    let Some(owner) = owner_tag.filter(|t| !t.is_empty()) else {
        return DiplomacyRelation::Neutral;
    };
    let Some(viewed) = viewed_tag.filter(|t| !t.is_empty()) else {
        return DiplomacyRelation::Neutral;
    };
    if owner == viewed {
        return DiplomacyRelation::SelfCountry;
    }
    if pair.same_faction {
        return DiplomacyRelation::SameFaction;
    }
    if pair.at_war {
        return DiplomacyRelation::AtWarEnemy;
    }
    DiplomacyRelation::Neutral
}

pub fn relation_to_vp_column(relation: DiplomacyRelation) -> u32 {
    match relation {
        DiplomacyRelation::SelfCountry | DiplomacyRelation::SameFaction => {
            VP_STRIP_COLOR_SELF_OR_FACTION
        }
        DiplomacyRelation::AtWarEnemy => VP_STRIP_COLOR_ENEMY_RED,
        DiplomacyRelation::Neutral => VP_STRIP_COLOR_NEUTRAL_GRAY,
    }
}

pub fn country_relations(
    viewed_tag: &str,
    pairs_by_owner: &BTreeMap<String, DiploPairInput>,
) -> BTreeMap<String, DiplomacyRelation> {
    pairs_by_owner
        .iter()
        .map(|(owner, pair)| {
            (
                owner.clone(),
                classify_relation(Some(owner), Some(viewed_tag), *pair),
            )
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DiplomacyPalette {
    pub self_or_faction: Option<[f32; 4]>,
    pub at_war: Option<[f32; 4]>,
    pub neutral: Option<[f32; 4]>,
}

impl DiplomacyPalette {
    pub const fn unknown_fail_closed() -> DiplomacyPalette {
        DiplomacyPalette {
            self_or_faction: None,
            at_war: None,
            neutral: None,
        }
    }
}

pub fn relation_fill(relation: DiplomacyRelation, palette: &DiplomacyPalette) -> Option<[f32; 4]> {
    match relation {
        DiplomacyRelation::SelfCountry | DiplomacyRelation::SameFaction => palette.self_or_faction,
        DiplomacyRelation::AtWarEnemy => palette.at_war,
        DiplomacyRelation::Neutral => palette.neutral,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_agrees_with_furn_rel_icon_law() {
        let tags = [None, Some(""), Some("GER"), Some("ENG")];
        for &owner in &tags {
            for &viewed in &tags {
                for at_war in [false, true] {
                    for same_faction in [false, true] {
                        let pair = DiploPairInput {
                            at_war,
                            same_faction,
                        };
                        let relation = classify_relation(owner, viewed, pair);
                        let icon_pair = VpIconDiplomaticPairState {
                            same_faction,
                            at_war,
                        };
                        assert_eq!(
                            relation_to_vp_column(relation),
                            vp_icon_relation_column(owner, viewed, icon_pair),
                            "owner={owner:?} viewed={viewed:?} at_war={at_war} same_faction={same_faction}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn missing_or_empty_tags_are_neutral() {
        let p = DiploPairInput {
            at_war: true,
            same_faction: true,
        };
        assert_eq!(
            classify_relation(None, Some("GER"), p),
            DiplomacyRelation::Neutral
        );
        assert_eq!(
            classify_relation(Some("GER"), None, p),
            DiplomacyRelation::Neutral
        );
        assert_eq!(
            classify_relation(Some(""), Some("GER"), p),
            DiplomacyRelation::Neutral
        );
        assert_eq!(
            classify_relation(Some("GER"), Some(""), p),
            DiplomacyRelation::Neutral
        );
    }

    #[test]
    fn self_beats_faction_beats_war() {
        assert_eq!(
            classify_relation(
                Some("GER"),
                Some("GER"),
                DiploPairInput {
                    at_war: true,
                    same_faction: false
                }
            ),
            DiplomacyRelation::SelfCountry
        );
        assert_eq!(
            classify_relation(
                Some("ITA"),
                Some("GER"),
                DiploPairInput {
                    at_war: true,
                    same_faction: true
                }
            ),
            DiplomacyRelation::SameFaction
        );
        assert_eq!(
            classify_relation(
                Some("ENG"),
                Some("GER"),
                DiploPairInput {
                    at_war: true,
                    same_faction: false
                }
            ),
            DiplomacyRelation::AtWarEnemy
        );
        assert_eq!(
            classify_relation(Some("SWE"), Some("GER"), DiploPairInput::default()),
            DiplomacyRelation::Neutral
        );
    }

    #[test]
    fn fill_is_fail_closed_when_palette_unknown() {
        let pal = DiplomacyPalette::unknown_fail_closed();
        for r in [
            DiplomacyRelation::SelfCountry,
            DiplomacyRelation::SameFaction,
            DiplomacyRelation::AtWarEnemy,
            DiplomacyRelation::Neutral,
        ] {
            assert_eq!(relation_fill(r, &pal), None, "{r:?} must fail closed");
        }
    }

    #[test]
    fn fill_routes_classes_to_palette_slots() {
        let pal = DiplomacyPalette {
            self_or_faction: Some([0.1, 0.2, 0.3, 1.0]),
            at_war: Some([0.9, 0.1, 0.1, 1.0]),
            neutral: Some([0.5, 0.5, 0.5, 1.0]),
        };
        assert_eq!(
            relation_fill(DiplomacyRelation::SelfCountry, &pal),
            pal.self_or_faction
        );
        assert_eq!(
            relation_fill(DiplomacyRelation::SameFaction, &pal),
            pal.self_or_faction
        );
        assert_eq!(
            relation_fill(DiplomacyRelation::AtWarEnemy, &pal),
            pal.at_war
        );
        assert_eq!(relation_fill(DiplomacyRelation::Neutral, &pal), pal.neutral);
    }


    fn fixture_pair(owner: &str, viewed: &str, tick: usize) -> DiploPairInput {
        let faction = |tag: &str| -> Option<&'static str> {
            if tick < 3 {
                None
            } else {
                match tag {
                    "GER" | "ITA" => Some("Axis"),
                    "ENG" => Some("Allies"),
                    _ => None,
                }
            }
        };
        let same_faction = owner != viewed
            && matches!((faction(owner), faction(viewed)), (Some(a), Some(b)) if a == b);
        let belligerent = |a: &str, b: &str| {
            let s = [a, b];
            s.contains(&"ENG") && (s.contains(&"GER") || s.contains(&"ITA"))
        };
        let at_war = tick >= 7 && owner != viewed && belligerent(owner, viewed);
        DiploPairInput {
            at_war,
            same_faction,
        }
    }

    fn relation_for(owner: &str, viewed: &str, tick: usize) -> DiplomacyRelation {
        classify_relation(Some(owner), Some(viewed), fixture_pair(owner, viewed, tick))
    }

    #[test]
    fn fx_war_faction_matrix_pov_ger() {
        use DiplomacyRelation::*;
        for tick in 0..12 {
            assert_eq!(relation_for("GER", "GER", tick), SelfCountry, "tick {tick}");
        }
        for tick in 0..3 {
            assert_eq!(
                relation_for("ITA", "GER", tick),
                Neutral,
                "ITA pre-faction tick {tick}"
            );
        }
        for tick in 3..12 {
            assert_eq!(
                relation_for("ITA", "GER", tick),
                SameFaction,
                "ITA post-faction tick {tick}"
            );
        }
        for tick in 0..7 {
            assert_eq!(
                relation_for("ENG", "GER", tick),
                Neutral,
                "ENG pre-war tick {tick}"
            );
        }
        for tick in 7..12 {
            assert_eq!(
                relation_for("ENG", "GER", tick),
                AtWarEnemy,
                "ENG war tick {tick}"
            );
        }
        assert_ne!(relation_for("ITA", "GER", 2), relation_for("ITA", "GER", 3));
        assert_ne!(relation_for("ENG", "GER", 6), relation_for("ENG", "GER", 7));
    }

    #[test]
    fn fx_war_faction_matrix_pov_eng() {
        use DiplomacyRelation::*;
        for tick in 0..12 {
            assert_eq!(relation_for("ENG", "ENG", tick), SelfCountry, "tick {tick}");
        }
        for owner in ["GER", "ITA"] {
            for tick in 0..7 {
                assert_eq!(
                    relation_for(owner, "ENG", tick),
                    Neutral,
                    "{owner} pre-war tick {tick}"
                );
            }
            for tick in 7..12 {
                assert_eq!(
                    relation_for(owner, "ENG", tick),
                    AtWarEnemy,
                    "{owner} war tick {tick}"
                );
            }
            assert_ne!(relation_for(owner, "ENG", 6), relation_for(owner, "ENG", 7));
        }
    }

    #[test]
    fn fill_is_keyed_per_viewed_tag() {
        assert_eq!(
            relation_for("GER", "GER", 8),
            DiplomacyRelation::SelfCountry
        );
        assert_eq!(relation_for("GER", "ENG", 8), DiplomacyRelation::AtWarEnemy);
        assert_eq!(
            relation_for("ITA", "GER", 8),
            DiplomacyRelation::SameFaction
        );
        assert_eq!(relation_for("ITA", "ENG", 8), DiplomacyRelation::AtWarEnemy);
    }

    #[test]
    fn country_relations_batches_one_pov() {
        let mut pairs = BTreeMap::new();
        for owner in ["GER", "ITA", "ENG"] {
            pairs.insert(owner.to_string(), fixture_pair(owner, "GER", 8));
        }
        let rels = country_relations("GER", &pairs);
        assert_eq!(rels["GER"], DiplomacyRelation::SelfCountry);
        assert_eq!(rels["ITA"], DiplomacyRelation::SameFaction);
        assert_eq!(rels["ENG"], DiplomacyRelation::AtWarEnemy);
    }
}
