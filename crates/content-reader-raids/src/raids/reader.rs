use content_reader_script::native::{self, Shape};
use crate::{
    fields::invalid,
    key_names::KeyNames,
    parser::{
        Document,
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

    fn reader(self) -> &'static str {
        use Node::*;
        match self {
            File => "raid file reader",
            Categories => "category container reader",
            Types => "type container reader",
            Category => "category reader",
            Raid => "raid type reader",
            Target => "target selector reader",
            Building => "building selector reader",
            Bounds => "bounds reader",
            EquipmentAmounts => "equipment pool reader",
            StartingPoint => "starting-point reader",
            Requirements => "unit requirement reader",
            BattalionTypes => "battalion-type reader",
            Equipment => "required equipment reader",
            Animations => "animation reader",
            Stage => "animation stage reader",
            Arrow => "arrow reader",
            Model => "unit model reader",
            Factors => "success factors reader",
            Formula => "success formula reader",
            StandardModifier => "standard modifier reader",
            CustomModifier => "custom modifier reader",
            Mtth => "mean-time-to-happen reader",
            Levels => "success levels reader",
            Outcome => "outcome reader",
            Visual => "visual effect reader",
        }
    }
}

pub(super) fn document(
    bytes: &[u8],
    equipment: super::Equipment<'_>,
    names: &KeyNames,
    notes: &mut Vec<crate::Diagnostic>,
) -> io::Result<Document> {
    use Node::*;
    let mut reported = Vec::new();
    let mut battalions = BTreeSet::new();
    let read = native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(File),
        Some(names),
        &mut |node, field| match (node, field.keyword().as_deref()) {
            (File, Some(b"categories")) if field.opens() => Shape::Object(Categories),
            (File, Some(b"types")) if field.opens() => Shape::Object(Types),
            (File, Some(key @ (b"categories" | b"types"))) => {
                reported.push(format!(
                    "line {}: {} has no block, which {} reports and enters nowhere",
                    field.key.line,
                    String::from_utf8_lossy(key),
                    if key == b"types" {
                        "type container reader"
                    } else {
                        "category container reader"
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
            (Raid, Some(b"target_type")) => Shape::Object(Target),
            (Raid, Some(b"essential_equipment" | b"additional_equipment")) => {
                Shape::Object(EquipmentAmounts)
            }
            (Raid, Some(b"starting_point")) => Shape::Object(StartingPoint),
            (Raid, Some(b"unit_requirements")) => {
                battalions.clear();
                Shape::Object(Requirements)
            }
            (Raid, Some(b"unit_animations")) => Shape::Object(Animations),
            (Raid, Some(b"arrow")) => Shape::Object(Arrow),
            (Raid, Some(b"unit_model")) => Shape::Object(Model),
            (Raid, Some(b"success_factors")) => Shape::Object(Factors),
            (Raid, Some(b"success_levels")) => Shape::Object(Levels),
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
            (Target, Some(b"province")) if field.opens() => Shape::List,
            (Target, Some(b"state")) if field.opens() => Shape::Trigger,
            (Target, Some(b"province" | b"state" | b"captured_army_leader")) => Shape::One,
            (Target, Some(b"building")) => Shape::Object(Building),
            (Target, _) => {
                reported.push(format!(
                    "line {}: target_type key {} is reported and skipped",
                    field.key.line,
                    String::from_utf8_lossy(&field.key.text)
                ));
                Shape::Skip
            }
            (Building, Some(b"type" | b"tags")) if field.opens() => Shape::List,
            (Building, Some(b"type" | b"tags")) => Shape::Current,
            (Building, Some(b"is_coastal")) => Shape::One,
            (Building, Some(b"level")) | (Equipment, Some(b"amount")) => Shape::Object(Bounds),
            (Bounds, Some(b"min" | b"max")) => Shape::One,
            (EquipmentAmounts, Some(b"nukes")) => Shape::One,
            (EquipmentAmounts, Some(key)) if equipment(key) == Some(true) => Shape::One,
            (EquipmentAmounts, Some(key)) if equipment(key).is_none() => {
                if field.opens() {
                    Shape::Unsettled(format!(
                        "raid equipment {} has a block, whose reading depends on whether it is an equipment type, and the equipment database was lost before it",
                        String::from_utf8_lossy(&field.key.text)
                    ))
                } else {
                    Shape::One
                }
            }
            (EquipmentAmounts, _) => {
                reported.push(format!(
                    "line {}: raid equipment {} is not an equipment type; reported and skipped",
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
                let key = field.keyword().unwrap_or_default();
                if battalions.insert(key) {
                    Shape::Object(Bounds)
                } else {
                    reported.push(format!(
                        "line {}: raid battalion_types repeats {}; reported and skipped",
                        field.key.line,
                        String::from_utf8_lossy(&field.key.text)
                    ));
                    Shape::Skip
                }
            }
            (Equipment, Some(b"type" | b"modules")) => Shape::List,
            (Animations, Some(b"stage")) => Shape::Object(Stage),
            (Stage, Some(b"state" | b"intro" | b"at_progress")) => Shape::One,
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
            (Formula, Some(key)) if super::tooltips::standard(key).is_some() => {
                Shape::Object(StandardModifier)
            }
            (Formula, _) => Shape::Object(CustomModifier),
            (
                StandardModifier | CustomModifier,
                Some(b"weight" | b"reference" | b"start_weight" | b"start_reference"),
            ) => Shape::One,
            (CustomModifier, Some(b"enable")) => Shape::ByKey,
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
                    Shape::EffectIn(ScopeKind::Unit.bit())
                } else {
                    Shape::EffectIn(ScopeKind::RaidInstance.bit())
                }
            }
            (Outcome, Some(b"custom_sound" | b"destroy_additional_equipment")) => Shape::One,
            (Outcome, Some(b"visual_effect")) => Shape::Object(Visual),
            (Visual, Some(b"animation" | b"entity")) => Shape::One,
            _ => {
                reported.push(format!(
                    "line {}: {} field {} is reported and skipped by {}",
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
    notes.extend(
        reported
            .into_iter()
            .map(|message| (crate::DiagnosticKind::Misread, message)),
    );
    notes.extend(
        read.misreads
            .iter()
            .map(|note| (note.kind, note.to_string())),
    );
    Ok(Document { items: read.items })
}
