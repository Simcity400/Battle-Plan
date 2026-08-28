//! The effective vanilla closure, evaluated by the exact engine rule.
//!
//! `extractor/native/runs/uz_ability_panel/READ_LAW.md` is the rule; this gate
//! is what stops the shipped evaluator drifting away from the closure the
//! recorder admits.

use game_ui::ability_visibility::{
    ability_visibility, parse_allowed_body, subunit_unlocked_abilities, trait_ability_unlocks,
    AllowedBody, AllowedNode, AllowedScope, AllowedWorldFacts, CasterFacts, CommandedFormation,
    Operand, Verdict, GAP_BORDER_WAR, GAP_DLC_MASK, GAP_LEADER_KIND, GAP_OWNER_TAG,
};
use game_ui::paradox_text::{parse_file, Document};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

fn game_root() -> PathBuf {
    std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT))
}

fn documents(directory: &Path) -> Vec<(String, Document)> {
    let mut entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
        .into_iter()
        .map(|path| {
            let document = parse_file(&path).unwrap_or_else(|error| {
                panic!("{} does not parse: {error}", path.display());
            });
            (path.display().to_string(), document)
        })
        .collect()
}

fn allowed_bodies(root: &Path) -> BTreeMap<String, AllowedBody> {
    let mut bodies = BTreeMap::new();
    for (source, document) in documents(&root.join("common/abilities")) {
        for top in document.root.fields() {
            assert!(
                top.key
                    .as_deref()
                    .is_some_and(|key| key.eq_ignore_ascii_case("ability")),
                "unexpected top-level construct in {source}"
            );
            for ability in top.value.fields() {
                let id = ability.key.as_deref().expect("named ability").to_string();
                let body = match ability.value.first_value_ci("allowed") {
                    Some(value) => parse_allowed_body(value).unwrap_or_else(|error| {
                        panic!("{id} in {source} has an underivable allowed body: {error}")
                    }),
                    None => AllowedBody::default(),
                };
                bodies.insert(id, body);
            }
        }
    }
    bodies
}

fn census(node: &AllowedNode, out: &mut BTreeSet<&'static str>) {
    let (label, children): (&'static str, Vec<&AllowedNode>) = match node {
        AllowedNode::And(children) => ("AND", children.iter().collect()),
        AllowedNode::Or(children) => ("OR", children.iter().collect()),
        AllowedNode::Nor(children) => ("NOT", children.iter().collect()),
        AllowedNode::If {
            limit,
            body,
            otherwise,
        } => (
            "if",
            limit
                .iter()
                .chain(body.iter())
                .chain(otherwise.iter().map(std::convert::AsRef::as_ref))
                .collect(),
        ),
        AllowedNode::OwnerScope(children) => ("OWNER", children.iter().collect()),
        AllowedNode::HasDlc(_) => ("has_dlc", Vec::new()),
        AllowedNode::HasCompletedFocus(_) => ("has_completed_focus", Vec::new()),
        AllowedNode::HasCountryFlag(_) => ("has_country_flag", Vec::new()),
        AllowedNode::HasAbility(_) => ("has_ability", Vec::new()),
        AllowedNode::IsLeadingArmyGroup(_) => ("is_leading_army_group", Vec::new()),
        AllowedNode::IsBorderWar(_) => ("is_border_war", Vec::new()),
    };
    out.insert(label);
    for child in children {
        census(child, out);
    }
}

struct ClosureFacts {
    trait_unlocks: BTreeMap<String, BTreeSet<String>>,
    abilities: BTreeSet<String>,
}

impl AllowedWorldFacts for ClosureFacts {
    fn dlc_mask(&self) -> Operand<u32> {
        Operand::Recorded(u32::MAX)
    }

    fn completed_focus(&self, _country_tag: &str, _focus: &str) -> Operand<bool> {
        Operand::Recorded(false)
    }

    fn country_flag_value(&self, _country_tag: &str, _flag: &str) -> Operand<Option<i64>> {
        Operand::Recorded(None)
    }

    fn trait_unlocks_ability(&self, trait_token: &str, ability_token: &str) -> Option<bool> {
        self.trait_unlocks
            .get(trait_token)
            .map(|set| set.contains(ability_token))
    }

    fn ability_exists(&self, ability_token: &str) -> bool {
        self.abilities.contains(ability_token)
    }
}

#[test]
fn every_vanilla_allowed_body_parses_into_the_taught_condition_kinds() {
    let root = game_root();
    assert!(
        root.is_dir(),
        "the exact ability derivation gate requires the retained the game install at {}",
        root.display()
    );
    let bodies = allowed_bodies(&root);
    assert_eq!(bodies.len(), 22, "vanilla authors 22 command abilities");

    let mut kinds = BTreeSet::new();
    for body in bodies.values() {
        for node in &body.nodes {
            census(node, &mut kinds);
        }
    }
    assert_eq!(
        kinds.into_iter().collect::<Vec<_>>(),
        vec![
            "NOT",
            "OR",
            "OWNER",
            "has_ability",
            "has_completed_focus",
            "has_country_flag",
            "has_dlc",
            "if",
            "is_border_war",
            "is_leading_army_group",
        ]
    );

    let with_border_war = bodies
        .values()
        .filter(|body| {
            let mut kinds = BTreeSet::new();
            for node in &body.nodes {
                census(node, &mut kinds);
            }
            kinds.contains("is_border_war")
        })
        .count();
    assert_eq!(
        with_border_war, 22,
        "every vanilla body tests is_border_war, so UZ-GAP-2 gates the whole panel"
    );
}

#[test]
fn the_engines_subunit_unlock_marking_is_the_five_hq_support_abilities() {
    let root = game_root();
    assert!(root.is_dir(), "requires the retained the game install");
    let unlocked = subunit_unlocked_abilities(&documents(&root.join("common/units")))
        .expect("the vanilla unit closure defines each sub-unit once");
    let authored = allowed_bodies(&root)
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let marked = unlocked
        .intersection(&authored)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        marked,
        vec![
            "deeper_dig_in".to_string(),
            "defense_in_depth".to_string(),
            "extra_medics".to_string(),
            "front_overrun".to_string(),
            "rotating_reserves".to_string(),
        ]
    );
    assert!(
        unlocked.difference(&authored).next().is_none(),
        "every enable_ability token resolves to an authored ability"
    );
}

#[test]
fn the_whole_vanilla_panel_refuses_by_name_under_the_current_capture() {
    let root = game_root();
    assert!(root.is_dir(), "requires the retained the game install");
    let bodies = allowed_bodies(&root);
    let facts = ClosureFacts {
        trait_unlocks: trait_ability_unlocks(&documents(&root.join("common/unit_leader")))
            .expect("the vanilla trait closure defines each trait once"),
        abilities: bodies.keys().cloned().collect(),
    };
    let unlocked = subunit_unlocked_abilities(&documents(&root.join("common/units")))
        .expect("the vanilla unit closure defines each sub-unit once");
    // The caster a capture can build today: d47 files a character under the
    // roster it was enumerated from, which is not CCharacter+0x68, and nothing
    // reads CArmy+0x1B0 or CUnitLeader+0xE7C.
    let caster = CasterFacts {
        owner_country_tag: Operand::Unrecorded(GAP_OWNER_TAG),
        traits: Vec::new(),
        land_leader: Operand::Unrecorded(GAP_LEADER_KIND),
        commanded: Some(CommandedFormation {
            is_army_group: false,
            border_war: Operand::Unrecorded(GAP_BORDER_WAR),
            member_unlocked_abilities: Operand::Recorded(BTreeSet::new()),
            hq_unit_unlocked_abilities: Operand::Recorded(BTreeSet::new()),
        }),
        parent_group_leader_traits: None,
        deployed_hq: Operand::Recorded(false),
    };
    let scope = AllowedScope {
        country_tag: Some("GER"),
        caster: Some(&caster),
        formation_selected: false,
    };
    let mut refused = 0usize;
    let mut hidden = 0usize;
    for (id, body) in &bodies {
        match ability_visibility(id, unlocked.contains(id), body, &scope, &facts) {
            Verdict::Refused(reason) => {
                assert!(
                    [GAP_BORDER_WAR, GAP_LEADER_KIND, GAP_OWNER_TAG, GAP_DLC_MASK]
                        .contains(&reason.as_str()),
                    "{id} refused for an unnamed reason: {reason}"
                );
                refused += 1;
            }
            Verdict::Hidden => hidden += 1,
            Verdict::Visible => panic!("{id} claims visibility while UZ-GAP-2 is open"),
        }
    }
    assert_eq!(refused + hidden, 22);
    assert!(refused > 0, "the gaps must reach the panel, not be absorbed");
}

fn determined_caster() -> CasterFacts {
    CasterFacts {
        owner_country_tag: Operand::Recorded("GER".into()),
        traits: vec!["fortress_buster".into()],
        land_leader: Operand::Recorded(true),
        commanded: Some(CommandedFormation {
            is_army_group: false,
            border_war: Operand::Recorded(false),
            member_unlocked_abilities: Operand::Recorded(BTreeSet::new()),
            hq_unit_unlocked_abilities: Operand::Recorded(BTreeSet::from([
                "extra_medics".to_string()
            ])),
        }),
        parent_group_leader_traits: None,
        deployed_hq: Operand::Recorded(true),
    }
}

#[test]
fn closing_the_capture_gaps_and_the_key_bindings_determines_the_whole_panel() {
    let root = game_root();
    assert!(root.is_dir(), "requires the retained the game install");
    let bodies = allowed_bodies(&root);
    let facts = ClosureFacts {
        trait_unlocks: trait_ability_unlocks(&documents(&root.join("common/unit_leader")))
            .expect("the vanilla trait closure defines each trait once"),
        abilities: bodies.keys().cloned().collect(),
    };
    let unlocked = subunit_unlocked_abilities(&documents(&root.join("common/units")))
        .expect("the vanilla unit closure defines each sub-unit once");
    let caster = determined_caster();
    let scope = AllowedScope {
        country_tag: Some("GER"),
        caster: Some(&caster),
        formation_selected: false,
    };
    let visible = bodies
        .iter()
        .filter(|(id, body)| {
            match ability_visibility(id, unlocked.contains(*id), body, &scope, &facts) {
                Verdict::Refused(reason) => panic!("{id} still refuses: {reason}"),
                Verdict::Visible => true,
                Verdict::Hidden => false,
            }
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec![
            "extra_medics".to_string(),
            "force_attack".to_string(),
            "last_stand".to_string(),
            "siege_artillery".to_string(),
            "staff_office_plan".to_string(),
        ]
    );
}

/// The shipped key-to-id table is the one the registrar emits. Reading the
/// pinned disassembly back is what stops a hand-typed id drifting.
#[test]
fn every_bound_key_carries_the_id_the_registrar_emits() {
    let evidence = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../extractor/native/runs/uz_ability_panel/evidence")
        .join("20260828T1050Z-uz-key-id-bindings.disasm.txt");
    let text = std::fs::read_to_string(&evidence)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", evidence.display()));

    let mut emitted: BTreeMap<String, u32> = BTreeMap::new();
    let mut pending: Option<u32> = None;
    for line in text.lines() {
        if line.starts_with("== ") {
            pending = None;
        } else if let Some(at) = line.find("mov   edx, dword [0x") {
            pending = line[at..]
                .rsplit_once("]=")
                .and_then(|(_, tail)| tail.strip_prefix("0x"))
                .and_then(|tail| u32::from_str_radix(tail.trim(), 16).ok());
        } else if let Some(id) = pending.take() {
            if let Some(at) = line.find("lea   r8, [") {
                let name = line[at..]
                    .split('"')
                    .nth(1)
                    .expect("the registration names its key")
                    .to_string();
                emitted.insert(name, id);
            } else {
                pending = Some(id);
            }
        }
    }
    assert!(
        emitted.len() >= 9,
        "the pinned disassembly lost registrations: {emitted:?}"
    );

    let mut checked = 0usize;
    for (key, binding) in game_ui::ability_visibility::AUTHORED_KEY_BINDINGS {
        let game_ui::ability_visibility::AuthoredKeyBinding::KeywordId { id, .. } = binding else {
            continue;
        };
        let Some(seen) = emitted.get(*key) else {
            panic!("{key} claims id {id:#x} with no registration in the pinned disassembly");
        };
        assert_eq!(seen, id, "{key}");
        checked += 1;
    }
    assert_eq!(checked, 8, "eight keys are bound by a registration");
}
