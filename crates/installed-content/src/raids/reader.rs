//! Native database dispatch; projection lives in the parent module.
use crate::{
    fields::invalid,
    key_names::KeyNames,
    parser::{
        Document,
        native::{self, Shape},
    },
    scope::ScopeKind,
};
use std::{collections::BTreeSet, io};

#[derive(Clone, Copy)]
enum Node {
    File,
    Categories,
    Types,
    Category,
    Raid,
    Target,
    Building,
    Bounds,
    EquipmentAmounts,
    StartingPoint,
    Requirements,
    BattalionTypes,
    Equipment,
    Animations,
    Stage,
    Arrow,
    Model,
    Factors,
    Formula,
    StandardModifier,
    CustomModifier,
    Mtth,
    Levels,
    Outcome,
    Visual,
}

impl Node {
    /// The object a node reads, as its field names it.
    fn what(self) -> &'static str {
        use Node::*;
        match self {
            File => "raid file",
            Categories => "categories",
            Types => "types",
            Category => "raid category",
            Raid => "raid type",
            Target => "target_type",
            Building => "target_type building",
            Bounds => "amount",
            EquipmentAmounts => "equipment amounts",
            StartingPoint => "starting_point",
            Requirements => "unit_requirements",
            BattalionTypes => "battalion_types",
            Equipment => "unit_requirements equipment",
            Animations => "unit_animations",
            Stage => "unit_animations stage",
            Arrow => "arrow",
            Model => "unit_model",
            Factors => "success_factors",
            Formula => "success formula",
            StandardModifier => "standard success modifier",
            CustomModifier => "custom success modifier",
            Mtth => "ai_will_do",
            Levels => "success_levels",
            Outcome => "success level",
            Visual => "visual_effect",
        }
    }

    /// The native reader of a node's fields.
    fn reader(self) -> &'static str {
        use Node::*;
        match self {
            File => "140A8CA20",
            Categories => "140A88270",
            Types => "140A88840",
            Category => "14157ED70",
            Raid => "140A8FA30",
            Target => "14157F920",
            Building => "1415779B0",
            Bounds => "141577790",
            EquipmentAmounts => "14157E530",
            StartingPoint => "14157EE40",
            Requirements => "14157FD70",
            BattalionTypes => "14157E7B0",
            Equipment => "14157E990",
            Animations => "14157FAF0",
            Stage => "141578480",
            Arrow => "14157E670",
            Model => "14157FB30",
            Factors => "14157F860",
            Formula => "14157F4D0",
            StandardModifier => "14157F800",
            CustomModifier => "14157F2A0",
            Mtth => "CMeanTimeToHappen",
            Levels => "14157F8B0",
            Outcome => "141577C10",
            Visual => "141578280",
        }
    }
}

/// One file's dispatch. `equipment` is the registry 14157E530 looks a key
/// up in; the triggers are read against `names`; each place the game
/// reports, and each misread, goes to `notes`.
pub(super) fn document(
    bytes: &[u8],
    equipment: super::Equipment<'_>,
    names: &KeyNames,
    notes: &mut Vec<String>,
) -> io::Result<Document> {
    use Node::*;
    let mut reported = Vec::new();
    let mut battalions = BTreeSet::new();
    // Database 140A8CA20, declaration loops 140A88270/140A88840;
    // CRaidCategory 14157ED70, CRaidType 140A8FA30. The file loop is EOF,
    // nested loops stop at close. Unknown fields report and skip blocks.
    // 140192E60 constructs each file's lexer in mode 0 (1424A6370).
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(File),
        Some(names),
        &mut |node, field| match (node, field.keyword().as_deref()) {
            // 140A88270 and 140A88840 need an opening brace; otherwise they
            // report the token and return, entering nothing.
            (File, Some(b"categories")) if field.opens() => Shape::Object(Categories),
            (File, Some(b"types")) if field.opens() => Shape::Object(Types),
            (File, Some(key @ (b"categories" | b"types"))) => {
                reported.push(format!(
                    "line {}: {} has no block, which {} reports and enters nowhere",
                    field.key.line,
                    String::from_utf8_lossy(key),
                    if key == b"types" {
                        "140A88840"
                    } else {
                        "140A88270"
                    },
                ));
                Shape::Skip
            }
            (Categories, _) => Shape::Object(Category),
            (Types, _) => Shape::Object(Raid),
            (Category, Some(b"visible" | b"available"))
            | (
                Raid,
                Some(
                    b"allowed" | b"visible" | b"show_target" | b"available" | b"launchable"
                    | b"launchable_from" | b"cancel_trigger",
                ),
            ) => Shape::Trigger,
            (Category, Some(b"free_targeting" | b"faction_influence_score_on_success")) => {
                Shape::One
            }
            (Category, Some(b"intel_source"))
                if matches!(
                    field.value_keyword().as_deref(),
                    Some(b"army" | b"naval" | b"air" | b"civilian")
                ) =>
            {
                Shape::One
            }
            (Raid, Some(b"ai_will_do")) => Shape::Object(Mtth),
            (Raid, Some(b"target_type")) => Shape::Object(Target), // 140A89350 / 14157F920
            (Raid, Some(b"essential_equipment" | b"additional_equipment")) => {
                Shape::Object(EquipmentAmounts)
            } // 140A88D80 / 14157E530
            (Raid, Some(b"starting_point")) => Shape::Object(StartingPoint), // 140A89160 / 14157EE40
            // Each field is a new requirement (140A87530), whose one
            // CBattalionTypeRequirements every battalion_types reads into.
            (Raid, Some(b"unit_requirements")) => {
                battalions.clear();
                Shape::Object(Requirements)
            } // 140A89920 / 14157FD70
            (Raid, Some(b"unit_animations")) => Shape::Object(Animations), // 140A89540 / 14157FAF0
            (Raid, Some(b"arrow")) => Shape::Object(Arrow),                // 14157E670
            (Raid, Some(b"unit_model")) => Shape::Object(Model),           // 14157FB30
            (Raid, Some(b"success_factors")) => Shape::Object(Factors),    // 14157F860
            (Raid, Some(b"success_levels")) => Shape::Object(Levels),      // 14157F8B0
            (
                Raid,
                Some(
                    b"range_factor"
                    | b"max_distance"
                    | b"category"
                    | b"custom_map_icon"
                    | b"fire_only_once"
                    | b"command_power"
                    | b"days_re_enable"
                    | b"ai_min_success_chance"
                    | b"days_to_prepare"
                    | b"launch_sound"
                    | b"custom_terrain_icon"
                    | b"target_loc_key"
                    | b"unlimited_ai_range"
                    | b"unit_icon"
                    | b"target_icon"
                    | b"equipment_icon"
                    | b"speed_multiplier"
                    | b"nuke_type",
                ),
            ) => Shape::One,
            // ReadScriptInt32Array (1401F91B0) resolves each token into the
            // value slot (+0xC0).
            (Target, Some(b"province")) if field.opens() => Shape::List,
            (Target, Some(b"state")) if field.opens() => Shape::Trigger,
            (Target, Some(b"province" | b"state" | b"captured_army_leader")) => Shape::One,
            (Target, Some(b"building")) => Shape::Object(Building), // 1415779B0
            // Any other key, a quoted one included: ReportUnexpectedToken
            // (1424AD100), no selector added.
            (Target, _) => {
                reported.push(format!(
                    "line {}: target_type key {} is reported and skipped (14157F920)",
                    field.key.line,
                    String::from_utf8_lossy(&field.key.text)
                ));
                Shape::Skip
            }
            (Building, Some(b"type" | b"tags")) if field.opens() => Shape::List, // 140309360, into +0xC0
            // 140309DC0 registers the lexer's current token (1424A6500), not
            // the value slot's.
            (Building, Some(b"type" | b"tags")) => Shape::Current,
            (Building, Some(b"is_coastal")) => Shape::One,
            (Building, Some(b"level")) | (Equipment, Some(b"amount")) => Shape::Object(Bounds), // 141577790
            (Bounds, Some(b"min" | b"max")) => Shape::One,
            // 14157E530: `nukes` (0x34CD) ReadScriptSignedInt32; a key whose
            // token 1409EB110 finds as an equipment type ParseTokenInt32 on
            // the value token alone; any other, a quoted key included,
            // ReportUnexpectedToken, skipping its block. Where the database
            // was lost before it could hold the key, both read a scalar value
            // alike (the projection refuses the type); a block they do not.
            (EquipmentAmounts, Some(b"nukes")) => Shape::One,
            (EquipmentAmounts, Some(key)) if equipment(key) == Some(true) => Shape::One,
            (EquipmentAmounts, Some(key)) if equipment(key).is_none() => {
                if field.opens() {
                    Shape::Unsettled(format!(
                        "raid equipment {} has a block, which 14157E530 reads by whether it is an equipment type, and the equipment database was lost before it",
                        String::from_utf8_lossy(&field.key.text)
                    ))
                } else {
                    Shape::One
                }
            }
            (EquipmentAmounts, _) => {
                reported.push(format!(
                    "line {}: raid equipment {} is not an equipment type; reported and skipped (14157E530)",
                    field.key.line,
                    String::from_utf8_lossy(&field.key.text)
                ));
                Shape::Skip
            }
            (StartingPoint, Some(b"types" | b"building_types")) => Shape::RawList,
            (StartingPoint, Some(b"allow_faction_buildings")) => Shape::One,
            (Requirements, Some(b"battalion_types")) => Shape::Object(BattalionTypes),
            (Requirements, Some(b"equipment")) => Shape::Object(Equipment),
            (BattalionTypes, _) => {
                // 14157E7B0 finds the key's token among those held: a repeat
                // is RecordError, its block skipped. A quoted key is token
                // 0xF (the projection refuses it).
                let key = field.keyword().unwrap_or_default();
                if battalions.insert(key) {
                    Shape::Object(Bounds)
                } else {
                    reported.push(format!(
                        "line {}: raid battalion_types repeats {}; reported and skipped (14157E7B0)",
                        field.key.line,
                        String::from_utf8_lossy(&field.key.text)
                    ));
                    Shape::Skip
                }
            }
            // 14157E990 -> 1409FB540 / 140309360, each token into +0xC0.
            (Equipment, Some(b"type" | b"modules")) => Shape::List,
            (Animations, Some(b"stage")) => Shape::Object(Stage),
            (Stage, Some(b"state" | b"intro" | b"at_progress")) => Shape::One, // 141578480
            (Arrow, Some(b"type"))
                if matches!(
                    field.value_keyword().as_deref(),
                    Some(b"land" | b"naval" | b"air" | b"line" | b"ballistic")
                ) =>
            {
                Shape::One
            }
            (
                Model,
                Some(b"scale" | b"entity" | b"equipment" | b"default_entity" | b"start_offset"),
            ) => Shape::One,
            (Factors, Some(b"success" | b"critical" | b"disaster")) => Shape::Object(Formula),
            (Formula, Some(b"base")) => Shape::One,
            // 14157F4D0 selects native standard classes by token; any other
            // name constructs a custom modifier, including quoted names.
            (Formula, Some(key)) if super::tooltips::standard(key).is_some() => {
                Shape::Object(StandardModifier)
            }
            (Formula, _) => Shape::Object(CustomModifier),
            (
                StandardModifier | CustomModifier,
                Some(b"weight" | b"reference" | b"start_weight" | b"start_reference"),
            ) => Shape::One, // 14157F800
            (CustomModifier, Some(b"enable")) => Shape::ByKey, // assign-trigger, 14157F2A0
            (CustomModifier, Some(b"formula")) => Shape::Object(Mtth),
            (CustomModifier, Some(b"scope" | b"can_target_affect" | b"can_actor_affect")) => {
                Shape::One
            }
            (Mtth, _) => native::mean_time_to_happen(field),
            (Levels, Some(b"success" | b"failure" | b"limited_success" | b"critical_success")) => {
                Shape::Object(Outcome)
            }
            (Outcome, Some(b"division_effects" | b"actor_effects" | b"victim_effects")) => {
                if field.keyword_is("division_effects") {
                    // Raid outcome reader 141577C10 passes unit scope at 141577D04.
                    Shape::EffectIn(ScopeKind::Unit.bit())
                } else {
                    // 141577C10 passes raid-instance scope at 141577CEC/141577CD7.
                    Shape::EffectIn(ScopeKind::RaidInstance.bit())
                }
            }
            (Outcome, Some(b"custom_sound" | b"destroy_additional_equipment")) => Shape::One,
            (Outcome, Some(b"visual_effect")) => Shape::Object(Visual),
            (Visual, Some(b"animation" | b"entity")) => Shape::One, // 141578280
            // Every reader here reports any other field through
            // ReportUnexpectedToken (1424AD100) and skips its block.
            _ => {
                reported.push(format!(
                    "line {}: {} field {} is reported and skipped ({}, 1424AD100)",
                    field.key.line,
                    node.what(),
                    String::from_utf8_lossy(&field.key.text),
                    node.reader(),
                ));
                Shape::Skip
            }
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("raids: {cause}")));
    }
    notes.extend(reported);
    notes.extend(read.misreads.iter().map(ToString::to_string));
    Ok(Document { items: read.items })
}
