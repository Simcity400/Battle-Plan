//! Equipment localisation links projected from the native technology reader.
use crate::{
    ResolvedLayers,
    fields::invalid,
    modifiers,
    parser::{
        Item,
        native::{self, Shape},
    },
};
use std::{collections::BTreeMap, io};
mod units;

#[derive(Clone, Copy)]
enum Node {
    File,
    Technologies,
    Technology,
    Path,
    Folder,
    Scalars,
    Building,
    Mtth,
    Uniform,
    Effect,
    Unit(units::Node),
    Modifier(modifiers::Node),
}

pub fn parse_equipment(input: &[u8]) -> io::Result<BTreeMap<String, Option<String>>> {
    parse_with_registry(input, &units::Registry::default())
}

fn parse_with_registry(
    input: &[u8],
    registry: &units::Registry,
) -> io::Result<BTreeMap<String, Option<String>>> {
    use Node::*;
    let mut undecided = Vec::new();
    let mut refused = Vec::new();
    let mut leading_limit = true;
    // CTechnologyDatabase 140ABCFC0 (EOF loop), CTechnologyTemplate
    // 140AC3960 / 140AC3C70. All dynamic content names come from layers.
    // CTechnologyDatabase::LoadDirectory (140AC1BA0) constructs each file's
    // lexer in mode 0 (1424A6370).
    let read = native::read(input, native::Mode::KeepMark, File, &mut |node, field| {
        match (node, field.keyword().as_deref()) {
            (File, Some(b"technologies")) => Shape::Object(Technologies),
            (File, Some(b"technology_categories")) => Shape::RawList,
            (File, Some(b"technology_folders")) => {
                refused.push("technology_folders: unsupported folder database reader".into());
                Shape::Skip
            }
            (File, _) => Shape::Skip,
            (Technologies, _) => Shape::Object(Technology),
            (
                Technology,
                Some(b"allow" | b"unlock" | b"allow_branch" | b"on_research_complete_limit"),
            ) => Shape::Trigger,
            (Technology, Some(b"ai_will_do")) => Shape::Object(Mtth),
            (Technology, Some(b"modifier")) => Shape::Object(Modifier(modifiers::Node::Modifier)),
            (Technology, Some(b"path")) => Shape::Object(Path),
            (Technology, Some(b"folder")) => Shape::Object(Folder),
            (Technology, Some(b"dependencies" | b"ai_research_weights")) => Shape::Object(Scalars),
            (Technology, Some(b"enable_building")) => Shape::Object(Building),
            (
                Technology,
                Some(
                    b"categories"
                    | b"xor"
                    | b"sub_technologies"
                    | b"enable_equipment_modules"
                    | b"special_project_specialization",
                ),
            ) => Shape::RawList,
            (Technology, Some(b"enable_equipments")) => Shape::TechnologyEquipment,
            (Technology, Some(b"enable_subunits")) if field.opens() => {
                leading_limit = true;
                Shape::Statements(Uniform)
            }
            (Technology, Some(b"enable_subunits")) => Shape::One,
            // 140AB8A50 / 140A83900 accept limit until the first anonymous
            // statement (140A835F0); other assignments use only the value.
            (Uniform, Some(b"limit")) if leading_limit => Shape::Trigger,
            (Uniform, _) => {
                if field.key.text.is_empty() {
                    leading_limit = false;
                }
                if registry
                    .units
                    .contains(&crate::parser::c_string(&field.value.text).to_ascii_lowercase())
                {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            // 140AC3C70 peeks for a leading limit (on_research_complete_limit,
            // +0x470), then CEffect::ReadScript (140534280) reads the body only
            // when the value slot is `{`.
            (Technology, Some(b"on_research_complete")) => Shape::LimitedBody(Effect),
            (Effect, _) => native::untraced(field),
            (Technology, Some(b"parent" | b"enable_tactic")) => Shape::Current,
            (
                Technology,
                Some(
                    b"is_special_project_tech"
                    | b"desc"
                    | b"research_cost"
                    | b"hidden"
                    | b"start_year"
                    | b"max_level"
                    | b"xp_research_bonus"
                    | b"doctrine"
                    | b"show_equipment_icon"
                    | b"show_effect_as_desc"
                    | b"force_use_small_tech_layout"
                    | b"xp_research_cost"
                    | b"xp_research_type"
                    | b"doctrine_name"
                    | b"sub_tech_index"
                    | b"xp_unlock_cost"
                    | b"xp_boost_cost",
                ),
            ) => Shape::One,
            // 140636690 checks subunits, then categories, before CModifier.
            (Technology, Some(key))
                if registry.units.contains(key) || registry.categories.contains(key) =>
            {
                Shape::Object(Unit(units::Node::Stats))
            }
            (Technology, _) => modifiers::shape(
                modifiers::Node::Modifier,
                field,
                modifiers::UNKNOWN,
                &mut undecided,
            )
            .map(Modifier),
            (Path, Some(b"leads_to_tech" | b"research_cost_coeff" | b"ignore_for_layout")) => {
                Shape::One
            } // 140AC3B00
            (Folder, Some(b"name")) => Shape::One, // 140AC39C0
            // 1402C7A50: the lexer's tokens, the value slot unchanged.
            (Folder, Some(b"position")) if field.opens() => Shape::RawList,
            (Folder | Path, _) => Shape::Skip,
            (Scalars | Building, _) => Shape::One, // 1413B04D0 and inline loops
            (Mtth, _) => native::mean_time_to_happen(field),
            (Modifier(node), _) => {
                modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided).map(Modifier)
            }
            (Unit(node), _) => registry
                .shape(node, field, &mut undecided, &mut refused)
                .map(Unit),
        }
    });
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("technology: {cause}")));
    }
    if let Some(cause) = refused.first() {
        return Err(invalid(cause));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("technology: {cause}")));
    }
    let mut result = BTreeMap::new();
    for root in read
        .items
        .iter()
        .filter(|item| item.keyword_is("technologies"))
    {
        for technology in root.value.items().unwrap_or_default() {
            let token = std::str::from_utf8(crate::parser::c_string(
                technology.key.as_deref().unwrap_or_default(),
            ))
            .map_err(|_| invalid("technology token is not UTF-8"))?;
            let mut unlocks: Vec<String> = Vec::new();
            let mut conditional = false;
            for field in technology.value.items().unwrap_or_default() {
                if field.keyword_is("folder") {
                    for position in field
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter(|i| i.keyword_is("position"))
                    {
                        let values = position.value.items().unwrap_or_default();
                        // 1402C7A50 has fixed-length recovery, not a list.
                        // These are the two forms for which List is identical.
                        let valid = values.len() == 2 && values.iter().all(Item::number)
                            || values.len() == 6
                                && values[0].value_keyword().as_deref() == Some("x")
                                && values[1].value.scalar() == Some(b"=")
                                && values[2].number()
                                && values[3].value_keyword().as_deref() == Some("y")
                                && values[4].value.scalar() == Some(b"=")
                                && values[5].number();
                        if !valid {
                            return Err(invalid(format!(
                                "technology {token} folder position: unsupported vector form"
                            )));
                        }
                    }
                }
                if !field.keyword_is("enable_equipments") {
                    continue;
                }
                for equipment in field.value.items().unwrap_or_default() {
                    if equipment.keyword_is("limit") {
                        conditional = true;
                        continue;
                    }
                    // 140AC3C70 has a special, evaluated `limit` branch in
                    // this raw loop. Its conditional early exit is not a list.
                    if equipment.value_keyword().as_deref() == Some("limit")
                        || matches!(equipment.kind, native::Kind::Open | native::Kind::Equals)
                    {
                        return Err(invalid(format!(
                            "technology {token} enable_equipments: unsupported conditional or nested unlock"
                        )));
                    }
                    let name = equipment.value.read_str().map_err(|e| {
                        invalid(format!("technology {token} enable_equipments: {e}"))
                    })?;
                    if !unlocks.iter().any(|old| old.eq_ignore_ascii_case(name)) {
                        unlocks.push(name.into());
                    }
                }
            }
            result.insert(
                token.to_owned(),
                (!conditional && unlocks.len() == 1).then(|| unlocks.remove(0)),
            );
        }
    }
    Ok(result)
}

pub fn read_equipment(
    layers: &ResolvedLayers,
) -> io::Result<(BTreeMap<String, String>, Vec<String>)> {
    let registry = units::Registry::read(layers)?;
    let mut result = BTreeMap::new();
    let mut warnings = Vec::new();
    // 140AC1BA0 enumerates this directory without recursion.
    for path in layers.directory_files("common/technologies", ".txt")? {
        match parse_with_registry(&layers.read(&path)?, &registry) {
            Ok(definitions) => result.extend(definitions),
            Err(error) => warnings.push(format!(
                "Technology equipment links unavailable in {path}: {error}"
            )),
        }
    }
    Ok((
        result
            .into_iter()
            .filter_map(|(token, equipment)| equipment.map(|equipment| (token, equipment)))
            .collect(),
        warnings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_bodies_follow_the_brace_guard_and_leading_limit() {
        // 140AC3C70 peeks for a leading limit and reads it as a trigger; then
        // 140534280 reads the body only when the value slot, as that left it,
        // is `{`. The one technology's unlock, by case:
        let link = |body: &str| {
            let text = format!("technologies={{t={{{body}}}}}");
            parse_equipment(text.as_bytes())
                .unwrap()
                .remove("t")
                .flatten()
        };
        // A scalar body reads nothing: the unlock after it is the technology's.
        assert_eq!(
            link("on_research_complete=no enable_equipments={one}").as_deref(),
            Some("one")
        );
        // An empty leading limit leaves its `{`: the body is read, and a later
        // `limit` in it is an effect.
        assert_eq!(
            link("on_research_complete={limit={} add_ideas=x limit=y} enable_equipments={two}")
                .as_deref(),
            Some("two")
        );
        // `limit = no`, or a limit whose last value is `yes`, leaves that value
        // in the slot: LoadBody and ReadScript report it and the fields after
        // it are the technology's.
        assert_eq!(
            link("on_research_complete={limit=no enable_equipments={custom_equipment}}").as_deref(),
            Some("custom_equipment")
        );
        assert_eq!(
            link("on_research_complete={limit={always=yes} enable_equipments={three}}").as_deref(),
            Some("three")
        );
        // A scalar body and then a limit block: that `{` opens the body over
        // the technology's remaining fields, the unlock among them.
        assert_eq!(
            link("on_research_complete=no limit={} enable_equipments={lost}"),
            None
        );
        // Forms whose token recovery is not followed are refused by name.
        for body in [
            "on_research_complete=no {ignored=1 enable_equipments={one}}",
            "enable_equipments={limit=no one}",
        ] {
            let text = format!("technologies={{t={{{body}}}}}");
            assert!(parse_equipment(text.as_bytes()).is_err(), "{body}");
        }
    }

    #[test]
    fn native_keywords_references_and_list_appends() {
        let links = parse_equipment(br#"@e=one TECHNOLOGIES={a={"enable_equipments"=wrong enable_equipments={@e} enable_equipments={ONE}} b={enable_equipments={one} enable_equipments={two}}}"#).unwrap();
        assert_eq!(links["a"].as_deref(), Some("one"));
        assert_eq!(links["b"], None);
    }

    #[test]
    fn conditional_unlock_preserves_following_fields_and_names_unsafe_tails() {
        let links = parse_equipment(br#"technologies={a={enable_equipments={limit={always=yes} one} research_cost=1} b={enable_equipments={two}}}"#).unwrap();
        assert_eq!(links["a"], None);
        assert_eq!(links["b"].as_deref(), Some("two"));
        let error =
            parse_equipment(b"technologies={a={enable_equipments={limit={always=no} one two}}}")
                .unwrap_err();
        assert!(error.to_string().contains("enable_equipments.limit"));
    }

    #[test]
    fn dynamic_reader_dispatch_uses_the_recorded_registry() {
        let mut registry = units::Registry::default();
        registry.units.insert(b"custom_unit".to_vec());
        registry.terrains.insert(b"custom_terrain".to_vec());
        let bytes = b"technologies={a={custom_unit={custom_terrain={attack=1}} enable_subunits={limit={always=yes} custom_unit} enable_equipments={one}}}";
        assert!(
            parse_equipment(bytes)
                .unwrap_err()
                .to_string()
                .contains("custom_unit")
        );
        assert_eq!(
            parse_with_registry(bytes, &registry).unwrap()["a"].as_deref(),
            Some("one")
        );
    }

    #[test]
    fn equipment_localisation_follows_only_an_unambiguous_authored_unlock() {
        let links = parse_equipment(
            br#"technologies = {
            @cost = 1
            weapons = { enable_equipments = { infantry_equipment_3 } }
            cars = { enable_equipments = { armored_car_equipment_2 } }
            multiple = { enable_equipments = { a b } }
            modifier_only = { research_cost = 2 }
        }"#,
        )
        .unwrap();
        assert_eq!(links["weapons"].as_deref(), Some("infantry_equipment_3"));
        assert_eq!(links["cars"].as_deref(), Some("armored_car_equipment_2"));
        for token in ["multiple", "modifier_only"] {
            assert_eq!(links[token], None);
        }
        let overrides =
            parse_equipment(br#"technologies = { a = { enable_equipments = { one } } a = {} }"#)
                .unwrap();
        assert_eq!(overrides["a"], None);
    }
}
