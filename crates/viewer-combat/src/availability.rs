
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use viewer_engagements::combat_selection::Selection;
use viewer_foundation::tables::Limitations;

pub fn combat_limitations(frame: &DecodedFrame, selection: Selection) -> Limitations {
    let mut out = Limitations::default();
    out.note::<tables::Politics>(
        frame,
        "combat flags cannot choose a missing country's ruling ideology",
    );
    out.note::<tables::PoliticalParties>(
        frame,
        "combat flags cannot resolve a missing ideology token",
    );
    match selection {
        Selection::AirRegion(_) | Selection::State(_) | Selection::NavalRegion(_) => {}
        Selection::Land(_) => {
            out.note::<tables::LandCombatDetails>(
                frame,
                "commander details remain unknown where missing; committed and reserve lists use LandCombatDivisions roles in full-side ordinal order, or the unsplit LandCombats roster when those rows are missing too. Recorded commitment-list order can differ.",
            );
            out.note::<tables::LandCombatDivisions>(
                frame,
                "hourly combat names, stats, exile badges and entry flags remain unknown \
                 where missing",
            );
            out.note::<tables::LandCombatModifiers>(
                frame,
                "a combat without its side rows leaves its border-war class, title and                  border modifier hovers undecided",
            );
            out.note::<tables::LandCombatDivisionModifiers>(
                frame,
                "a division without its row shows no modifier icons or combat-stat hovers",
            );
            out.note::<tables::LandCombats>(
                frame,
                "the selected combat's absent participants remain unknown",
            );
            out.note::<tables::LandCombatSides>(
                frame,
                "a combat without its row shows no side-flag modifier icons, and a committed \
                 division whose anti-air value decides the anti-air icon (frame 13, side flag \
                 0x4000) ends its icon list there, omitting that icon and the exile \
                 icons 14 and 15 after it; the combat \
                 window counts the entries cut",
            );
            out.note::<tables::Divisions>(
                frame,
                "missing divisions have no flag, template or bars, and their combat no progress",
            );
            out.note::<tables::DivisionTemplateIcons>(
                frame,
                "missing division templates have no unit icon",
            );
            out.note::<tables::DivisionStatus>(
                frame,
                "missing maxima leave strength and organisation bars empty",
            );
        }
        Selection::Naval(_) => {
            out.note::<tables::NavalCombatDetails>(
                frame,
                "screening, progress and commanders remain unknown where missing",
            );
            out.note::<tables::NavalCombatShips>(
                frame,
                "ship bars and escape progress remain unknown where missing",
            );
            out.note::<tables::NavalCombats>(
                frame,
                "the selected battle's positioning and anti-air values remain unknown",
            );
            out.note::<tables::NavalCombatAirBaseWingHandles>(
                frame,
                "positioning with carrier or land-based air remains unknown: base fill is not counted",
            );
            out.note::<tables::ShipModels>(
                frame,
                "screening and anti-air remain unknown: ship categories and AA bases are missing",
            );
            out.note::<tables::NavalCombatParticipants>(
                frame,
                "only recorded battle slots are shown; no complete fleet census is claimed",
            );
            out.note::<tables::Ships>(
                frame,
                "a missing non-convoy ship has no known category or combat column",
            );
        }
        Selection::NavalResult(_) => {
            out.note::<tables::NavalCombatResultDetails>(
                frame,
                "retained ship and plane snapshots remain unknown where missing",
            );
            out.note::<tables::NavalResults>(
                frame,
                "no battle result is inferred from an absent live combat",
            );
            out.note::<tables::NavalResultCountries>(
                frame,
                "result flags stay empty where their recorded side is missing",
            );
            out.note::<tables::NavalResultMembers>(
                frame,
                "unrecorded result members remain unknown",
            );
        }
    }
    out
}
