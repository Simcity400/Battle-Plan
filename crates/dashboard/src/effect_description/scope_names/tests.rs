use super::*;
use crate::catalogue::tests::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use crate::scripted_text::{EventScope, Link};
use replay::{frame::TableView, rows::*};

const RAID: u64 = 7 << 32 | 84;
const UNIT: u64 = 8 << 32 | 55;
const MIO: u64 = 6 << 32 | 79;

fn fixture() -> (Fixture, Folder) {
    let mut f = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        ("common/defines/test.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=5}}"),
        ("common/special_projects/projects/test.txt", "synthetic_project={narrative={name=PROJECT_NAME}} synthetic_default={}"),
        ("localisation/test_l_english.yml", concat!(
            "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n",
            " PROJECT_NAME:0 \"base\"\n AAA_PROJECT_NAME:0 \"Project[ROOT.GetTag]/[FROM.GetTag]/[PREV.GetTag]\"\n",
            " synthetic_default:0 \"Default\"\n SYN_MIO:0 \"base MIO\"\n BBB_SYN_MIO:0 \"MIO[FROM.GetTag]\"\n",
            " EFFECT_CONTEXT_SPECIAL_PROJECT:0 \"$SPECIAL_PROJECT$ $FLAG$\"\n",
            " EFFECT_ADD_ARMY_EXPERIENCE:0 \"XP$VAL|0$\"\n",
            " raid_type_synthetic_raid:0 \"Strike\"\n TARGET:0 \"Hit $LOCATION$\"\n",
            " raid_inst_item_desc:0 \"$TYPE$/$TARGET$\"\n RAID_TARGET_NAME:0 \"$WHAT$ in $LOCATION$\"\n",
        )),
    ]);
    catalogue.scripted_content().unwrap().set_synthetic_environment(Default::default());
    f.catalogue = catalogue;
    *f.frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
        LexicalTokenRow { token: 10, literal: b"synthetic_project".to_vec() },
        LexicalTokenRow { token: 11, literal: b"synthetic_default".to_vec() },
        LexicalTokenRow { token: 12, literal: b"synthetic_raid".to_vec() },
        LexicalTokenRow { token: 13, literal: b"SYN_MIO".to_vec() },
    ]));
    *f.frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(vec![
        ProjectRow { owner: 100, local_id: 9, definition_id: 10, ..Default::default() },
        ProjectRow { owner: 100, local_id: 10, definition_id: 11, ..Default::default() },
    ]));
    *f.frame.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(vec![RaidRow { reference_type: 84, id: 7, ..Default::default() }]));
    *f.frame.tables[TableId::RaidTargets.index()] = TableView::Complete(TableRows::RaidTargets(vec![RaidTargetRow { reference_type: 84, id: 7, raid_type: 12, scope: RaidScope { state: 2, ..Default::default() }, ..Default::default() }]));
    *f.frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![DivisionRow { id: UNIT, logical_owner: 100, ..Default::default() }]));
    *f.frame.tables[TableId::IndustrialOrganisations.index()] = TableView::Complete(TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow { organisation: MIO, country: 100, owner: 200, token: 13, ..Default::default() }]));
    // Supply only synthetic content, through the production name's cache.
    let mut raids = installed_content::raids::Catalog::default();
    let mut kind = installed_content::raids::RaidType { name: "synthetic_raid".into(), ..Default::default() };
    kind.tooltip.target_loc_key = "TARGET".into();
    raids.types.push(kind);
    assert!(f.catalogue.scripted_content().unwrap().effects().unwrap().raids.set(Ok(raids)).is_ok());
    (f, folder)
}

#[test]
fn project_and_mio_names_use_native_keys_and_fresh_environments() {
    let (f, _folder) = fixture();
    let mut graph = EventScopes::new(vec![EventScope {
        root: Link::Scope(1), from: Link::Scope(1), prev: Link::Scope(1),
        ..EventScope::of(EventObject::Country(1))
    }, EventScope::of(EventObject::Country(2))]);
    f.with_renderer(&mut graph, Content::default(), |renderer| {
        assert_eq!(renderer.project_name(100, 9).as_deref(), Ok("ProjectAAA/AAA/AAA"));
        assert_eq!(renderer.project_name(100, 10).as_deref(), Ok("Default"));
        assert_eq!(renderer.organisation_name(MIO).as_deref(), Ok("MIOBBB"));
        assert!(renderer.project_name(100, 999).unwrap_err().contains("Projects lacks"));
        assert!(renderer.organisation_name(999).unwrap_err().contains("IndustrialOrganisations lacks"));
    });
    let out = f.render("sp:synthetic_project={army_experience=2}", &mut fresh(), Content::default());
    assert_eq!(out.text, "ProjectAAA/AAA/AAA \u{12}AAA XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn raid_name_joins_instance_target_type_and_content_and_refuses_unknown_identity() {
    let (mut f, _folder) = fixture();
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert_eq!(renderer.raid_name(RAID).as_deref(), Ok("Strike/Hit Two"));
        assert!(renderer.raid_name(999).unwrap_err().contains("Raids lacks"));
        assert!(renderer.unit_name(UNIT).unwrap_err().contains("CNameGroupMember+0x60"));
        assert!(renderer.unit_name(999).unwrap_err().contains("Divisions lacks"));
    });
    if let TableView::Complete(TableRows::RaidTargets(rows)) = &mut *f.frame.tables[TableId::RaidTargets.index()] {
        rows[0].raid_type = 0;
    }
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert_eq!(renderer.raid_name(RAID).as_deref(), Ok("/Two"));
    });
    *f.frame.tables[TableId::RaidTargets.index()] = TableView::Unavailable { reason: "synthetic target fault".into() };
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert!(renderer.raid_name(RAID).unwrap_err().contains("synthetic target fault"));
    });
}

#[test]
fn unit_and_raid_contexts_have_no_invented_header_and_still_walk_children() {
    let (f, _folder) = fixture();
    for object in [EventObject::Raid(RAID), EventObject::Unit(UNIT)] {
        let mut graph = EventScopes::new(vec![EventScope::of(object)]);
        let out = f.render("THIS={army_experience=2}", &mut graph, Content::default());
        assert_eq!(out.text, "XP2\n");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn carried_saved_targets_do_not_invent_country_or_state_members() {
    let (f, _folder) = fixture();
    let mut graph = fresh();
    graph.saved_targets.insert(0, vec![
        (b"raid".to_vec(), EventObject::Raid(RAID)),
        (b"unit".to_vec(), EventObject::Unit(UNIT)),
        (b"project".to_vec(), EventObject::Project { owner: 100, id: 9 }),
    ]);
    f.with_renderer(&mut graph, Content::default(), |renderer| {
        for name in ["raid", "unit", "project"] {
            let operand = installed_content::parser::parse(format!("target=event_target:{name}").as_bytes()).unwrap();
            let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
            assert_eq!(renderer.country_target(&operand.items[0], scopes), Ok(None));
            assert_eq!(renderer.state_target(&operand.items[0], scopes), Ok(0));
        }
    });
}
