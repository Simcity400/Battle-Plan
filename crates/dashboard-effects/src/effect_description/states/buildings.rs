use super::*;
use replay_data::rows::{ProvinceRow, ResourceDefinitionRow};

const PROVINCE_STATE: u64 = mask::<ProvinceRow>(&["state"]);
const RESOURCE_TOKEN: u64 = mask::<ResourceDefinitionRow>(&["token"]);
use content_reader_key_names::buildings::BuildingType;
use content_reader_parser::{crt::{Scanned, scan_i32}, parser::{self, c_string}};

fn plain_integer(items: &[Item], name: &str) -> Option<i32> {
    items
        .iter()
        .filter(|item| item.keyword_is(name))
        .filter_map(|item| match scan_i32(item.value.scalar()?) {
            Scanned::Stored(value) => Some(value),
            Scanned::NoMatch | Scanned::End => None,
        })
        .next_back()
}

fn plain_fixed5(items: &[Item], name: &str) -> i64 {
    items
        .iter()
        .filter(|item| item.keyword_is(name))
        .filter_map(|item| content_reader_parser::fixed5::script_fixed5(item.value.scalar()?))
        .next_back()
        .unwrap_or(0)
}

fn building_selector<'a>(items: &'a [Item], name: &str) -> (i32, Option<&'a Item>) {
    let mut plain = 0;
    let mut scoped = None;
    for item in items.iter().filter(|item| item.keyword_is(name)) {
        if item.number() {
            if let Some(text) = item.value.scalar()
                && let Scanned::Stored(value) = scan_i32(text)
            {
                plain = value;
            }
        } else {
            scoped = Some(item);
        }
    }
    (plain, scoped)
}

fn has_building_tags(item: &Item) -> bool {
    children(item)
        .iter()
        .filter(|item| item.keyword_is("tags"))
        .any(|item| item.value.items().is_none_or(|items| !items.is_empty()))
}

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn building_construction(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let level = self.building_level(item, scopes, 0)?;
        let type_field = field(children(item), "type").ok_or_else(|| {
            format!(
                "{} has no building template; own description requires that template",
                self.building_effect_name(item)
            )
        })?;
        let building = self.building_type(type_field)?.ok_or_else(|| {
            format!(
                "{} unknown type needs the null-template localisation strings, which content does not define",
                self.building_effect_name(item)
            )
        })?;
        let building = self.building_label(building, false)?;
        let number = level.to_string();
        let (key, number_argument, expand): (&[u8], _, _) =
            match self.effect_class(item).map(|class| class.vtable) {
                Some(effect_identity::CONSTRUCTION_ORDER) => {
                    let province = plain_integer(children(item), "province").unwrap_or(0) != 0;
                    let instant = parser::read_bool_field(children(item), "instant_build", true);
                    let key: &[u8] = match (province, instant) {
                        (false, false) => b"EFFECT_ADD_BUILDING_CONSTRUCTION",
                        (false, true) => b"EFFECT_ADD_BUILDING_CONSTRUCTION_INSTANT",
                        (true, false) => b"EFFECT_ADD_BUILDING_CONSTRUCTION_PROVINCE",
                        (true, true) => b"EFFECT_ADD_BUILDING_CONSTRUCTION_INSTANT_PROVINCE",
                    };
                    (key, Argument::Int32(level), false)
                }
                Some(effect_identity::BUILDING_LEVEL) => (
                    b"EFFECT_SET_BUILDING_CONSTRUCTION",
                    Argument::Text(&number),
                    false,
                ),
                Some(effect_identity::EXTERNAL_BUILDING) => (
                    b"EFFECT_ADD_OFFSITE_BUILDING",
                    Argument::Text(&number),
                    true,
                ),
                _ => {
                    return Err(format!(
                        "{} has no construction-description port",
                        self.building_effect_name(item)
                    ));
                }
            };
        self.line(
            key,
            &[
                ("NUMBER", number_argument),
                ("BUILDING", Argument::Text(&building)),
            ],
            scopes,
            expand,
        )
    }

    pub(in crate::effect_description) fn construct_random_province(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let buildings = self.buildings()?;
        let mut selected = None;
        for child in children(item) {
            let name = parser::text(c_string(child.key.as_deref().unwrap_or_default()))
                .map_err(|why| format!("random-province building name: {why}"))?;
            if let Some(building) = buildings.by_token(name).map_err(|why| why.to_string())?
                && building.provincial()
            {
                selected = Some(building);
            }
        }
        let selected = selected.ok_or_else(|| {
            "construct_building_in_random_province has no provincial building template; own description requires that template".to_string()
        })?;
        let building = self.building_label(selected, false)?;
        let state = match scopes.this {
            Scope::State(state) => state,
            _ => NULL_STATE,
        };
        if self.valid_state(state)?.is_none() {
            return Err(
                "construct_building_in_random_province has no current state; state naming requires an existing state".into(),
            );
        }
        let state = self.state_name(state)?;
        self.localised(
            b"CONSTRUCT_BUILDING_IN_STATE",
            &[
                ("BUILDING", Argument::Text(&building)),
                ("STATE", Argument::Text(&state)),
            ],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn damage_building(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(type_field) = field(children(item), "type") else {
            let province = self.building_province(item, scopes)?;
            if province.is_none() && self.building_state(item, scopes)?.is_none() {
                return Ok(String::new());
            }
            return if has_building_tags(item) {
                Err(self.building_tag_refusal(item))
            } else {
                Ok(String::new())
            };
        };
        let building = self.building_type(type_field)?.ok_or_else(|| {
            "damage_building unknown type needs the null-template plural localisation string, which content does not define".to_string()
        })?;
        let building = self.building_label(building, true)?;
        let damage = plain_fixed5(children(item), "damage");
        let repair = plain_fixed5(children(item), "repair_speed_modifier");
        let mut text = self.line(
            b"EFFECT_DAMAGE_BUILDING",
            &[
                ("BUILDING", Argument::Text(&building)),
                ("AMOUNT", Argument::Fixed5(damage)),
            ],
            scopes,
            false,
        )?;
        if repair != 0 {
            let repair = self.localised(
                b"EFFECT_TEMPORARY_REPAIR_SPEED_MODIFIER",
                &[
                    ("BUILDING", Argument::Text(&building)),
                    ("FACTOR", Argument::Fixed5(repair)),
                ],
                scopes,
                false,
            )?;
            text.push_str(&indent_nonempty_lines(&repair, 3));
            text.push('\n');
        }
        Ok(text)
    }

    pub(in crate::effect_description) fn remove_building(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let province = self.building_province(item, scopes)?;
        let kind = field(children(item), "type")
            .map(|type_field| {
                self.building_type(type_field)?.ok_or_else(|| {
                    "remove_building unknown type needs null-template province_max, which content does not expose".to_string()
                })
            })
            .transpose()?;
        if kind.is_some_and(BuildingType::provincial) {
            return if province.is_none() {
                Ok(String::new())
            } else {
                Err("remove_building provincial targeting: core content plumbing pending: content_reader_map::movement::MovementMap::terrain(id).flags for the province-definition land flag".into())
            };
        }
        if kind.is_none() && province.is_some() {
            return if has_building_tags(item) {
                Err(self.building_tag_refusal(item))
            } else {
                Ok(String::new())
            };
        }
        let Some(state) = self.building_state(item, scopes)? else {
            return Ok(String::new());
        };
        if has_building_tags(item) {
            return Err(self.building_tag_refusal(item));
        }
        let Some(kind) = kind else {
            return Ok(String::new());
        };
        let token = self
            .context
            .world()?
            .token_of(kind.name.as_bytes())
            .map_err(|why| why.to_string())?;
        let buildings = self.complete::<tables::StateBuildings>(0)?;
        if !token.is_some_and(|token| {
            buildings.contains(u128::from(state) << 32 | u128::from(token))
        }) {
            return Ok(String::new());
        }
        let location = self.state_name(state)?;
        let level = self.building_level(item, scopes, 0)?.to_string();
        let building = self.building_label(kind, false)?;
        self.line(
            b"EFFECT_REMOVE_BUILDING",
            &[
                ("NUMBER", Argument::Text(&level)),
                ("BUILDING", Argument::Text(&building)),
                ("LOCATION", Argument::Text(&location)),
            ],
            scopes,
            false,
        )
    }

    pub(in crate::effect_description) fn modify_building_resources(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let building = field(children(item), "building").ok_or_else(|| {
            "modify_building missing building leaves the template pointer uninitialised".to_string()
        })?;
        if building.value.scalar().is_none() {
            return Ok(String::new());
        }
        let Some(building) = self.building_type(building)? else {
            return Ok(String::new());
        };
        let amount = plain_integer(children(item), "amount").ok_or_else(|| {
            "modify_building missing or failed amount assignment leaves the amount uninitialised".to_string()
        })?;
        let resource = field(children(item), "resource").ok_or_else(|| {
            "modify_building missing resource leaves the resource index uninitialised".to_string()
        })?;
        let resource = resource
            .value
            .c_str()
            .map_err(|why| format!("modify_building resource: {why}"))?;
        let frame = self.resource_index(resource)?;
        let building = self.building_label(building, false)?;
        self.line(
            b"EFFECT_MODIFY_BUILDING_RESOURCES",
            &[
                ("BUILDING", Argument::Text(&building)),
                ("AMOUNT", Argument::Int32(amount)),
                ("FRAME", Argument::Int32(frame)),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn add_resource(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let amount = match field(children(item), "amount") {
            Some(amount) => self.scoped_value(scalar_operand(amount)?, scopes)?,
            None => 0,
        };
        let amount = crate::triggers::fixed_round(amount) as i32;
        let resource = self.resource_icon(item)?;
        let resource = format!("{}\u{13}resources_strip|{resource} ", amount.wrapping_abs());
        let state = match field(children(item), "state") {
            Some(state) => self
                .evaluator()?
                .scoped_state_id(scalar_operand(state)?, scopes)
                .map_err(|why| why.to_string())? as u32,
            None => match scopes.this {
                Scope::State(state) => state,
                _ => NULL_STATE,
            },
        };
        if self.valid_state(state)?.is_none() {
            return Ok(String::new());
        }
        let state = self.state_name(state)?;
        let days = match field(children(item), "days") {
            Some(days) => self.scoped_value(scalar_operand(days)?, scopes)?,
            None => 0,
        };
        let days = crate::triggers::fixed_round(days) as i32;
        let show = parser::read_bool_field(children(item), "show_state_in_tooltip", true);
        let key: &[u8] = match (amount > 0, days != 0, show) {
            (true, false, true) => b"EFFECT_ADD_RESOURCE",
            (false, false, true) => b"EFFECT_REMOVE_RESOURCE",
            (true, true, true) => b"EFFECT_ADD_TEMPORARY_RESOURCE",
            (false, true, true) => b"EFFECT_REMOVE_TEMPORARY_RESOURCE",
            (true, false, false) => b"EFFECT_ADD_RESOURCE_NO_STATE",
            (false, false, false) => b"EFFECT_REMOVE_RESOURCE_NO_STATE",
            (true, true, false) => b"EFFECT_ADD_TEMPORARY_RESOURCE_NO_STATE",
            (false, true, false) => b"EFFECT_REMOVE_TEMPORARY_RESOURCE_NO_STATE",
        };
        self.line(
            key,
            &[
                ("RESOURCE", Argument::Text(&resource)),
                ("STATE", Argument::Text(&state)),
                ("DAYS", Argument::Int32(days)),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn give_resource_rights(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let receiver = match field(children(item), "receiver") {
            Some(receiver) => self
                .evaluator()?
                .scoped_country(scalar_operand(receiver)?, scopes)
                .map_err(|why| why.to_string())?,
            None => None,
        };
        let state = match field(children(item), "state") {
            Some(state) => self
                .evaluator()?
                .scoped_state_id(scalar_operand(state)?, scopes)
                .map_err(|why| why.to_string())? as u32,
            None => 0,
        };
        if self.valid_state(state)?.is_none() {
            return Ok(String::new());
        }
        let receiver = receiver.unwrap_or(0);
        let receiver = self.country_name(receiver, true)?;
        let state = self.state_name(state)?;
        self.line(
            b"EFFECT_GIVE_RESOURCE_RIGHTS",
            &[
                ("RECEIVER", Argument::Text(&receiver)),
                ("STATE", Argument::Text(&state)),
            ],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn remove_resource_rights(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let state = self.state_target(item, scopes)?;
        if self.valid_state(state)?.is_none() {
            return Ok(String::new());
        }
        let state = self.state_name(state)?;
        self.line(
            b"EFFECT_REMOVE_RESOURCE_RIGHTS",
            &[("STATE", Argument::Text(&state))],
            scopes,
            true,
        )
    }

    pub(in crate::effect_description) fn build_railway(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let level = self.building_level(item, scopes, 100_000)?;
        if level < 1 {
            return Ok(String::new());
        }
        let maximum = self
            .context
            .content
            .max_railway_level
            .ok_or_else(|| self.content_gap("MAX_RAILWAY_LEVEL"))?;
        if level > maximum {
            return Ok(String::new());
        }
        let mut path = Vec::new();
        for part in children(item).iter().filter(|item| item.keyword_is("path")) {
            let items = part.value.items().ok_or_else(|| {
                "build_railway path requires the native integer-array projection".to_string()
            })?;
            for province in items {
                let text = scalar_operand(province)?;
                path.push(match scan_i32(text) {
                    Scanned::Stored(value) => value as u32,
                    Scanned::NoMatch | Scanned::End => 0,
                });
            }
        }
        if path.is_empty() {
            let start = plain_integer(children(item), "start_province").unwrap_or(0);
            if start <= 0 {
                let Some(state) = field(children(item), "start_state") else {
                    return Ok(String::new());
                };
                let state = self
                    .evaluator()?
                    .scoped_state_id(scalar_operand(state)?, scopes)
                    .map_err(|why| why.to_string())? as u32;
                if self.valid_state(state)?.is_none() {
                    return Ok(String::new());
                }
            }
            return Err(
                "build_railway endpoint selection/pathfinding is unavailable; state-node selection needs province-array order".into(),
            );
        }
        if field(children(item), "controller_priority").is_some() {
            return Err(
                "build_railway controller_priority needs eligibility/MTTH evaluation in its controller event scope; fallback pathfinding is unavailable".into(),
            );
        }
        if parser::read_bool_field(children(item), "build_only_on_allied", false) {
            match scopes.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. }
                    if country as i32 > 0 =>
                {
                    return Err(
                        "build_railway build_only_on_allied needs country/controller eligibility; fallback pathfinding is unavailable".into(),
                    );
                }
                Scope::Country(_) | Scope::CountryLeader { .. } | Scope::State(_) => {}
                _ => {
                    return Err(
                        "build_railway ally filtering needs the current-scope country tag for this projected scope kind".into(),
                    );
                }
            }
        }
        let provinces = self.complete::<tables::Provinces>(PROVINCE_STATE)?;
        let endpoint_state = |id: u32| {
            keyed(&provinces, u128::from(id))?.map(|row| row.state).ok_or_else(|| {
                format!(
                    "build_railway endpoint province {id} is absent; state lookup requires an existing province"
                )
            })
        };
        let first = endpoint_state(path[0])?;
        let last = endpoint_state(*path.last().unwrap())?;
        if first == 0 || last == 0 {
            return Ok(String::new());
        }
        if first == last {
            let state = self.state_name(first)?;
            self.localised(
                b"EFFECT_BUILD_RAILWAY_IN_STATE",
                &[
                    ("LEVEL", Argument::Int32(level)),
                    ("STATE", Argument::Text(&state)),
                ],
                scopes,
                true,
            )
        } else {
            let to = self.state_name(last)?;
            let from = self.state_name(first)?;
            self.localised(
                b"EFFECT_BUILD_RAILWAY",
                &[
                    ("LEVEL", Argument::Int32(level)),
                    ("FROM", Argument::Text(&from)),
                    ("TO", Argument::Text(&to)),
                ],
                scopes,
                true,
            )
        }
    }

    pub(in crate::effect_description) fn create_railway_gun(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let location = plain_integer(children(item), "location").unwrap_or(0) as u32;
        let provinces = self.complete::<tables::Provinces>(PROVINCE_STATE)?;
        let state = match keyed(&provinces, u128::from(location))? {
            Some(province) if province.state == 0 => {
                return Err(
                    "create_railway_gun location has no state; state naming requires an existing state".into(),
                );
            }
            Some(province) => Some(self.state_name(province.state)?),
            None => None,
        };
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. }
                if country as i32 > 0 =>
            {
                country
            }
            Scope::Country(_) | Scope::CountryLeader { .. } | Scope::State(_) => 0,
            _ => {
                return Err(
                    "create_railway_gun OWNER needs the current-scope country tag for this projected scope kind; the scope projection does not expose it".into(),
                );
            }
        };
        let owner = self.context.text.country_adjective(country)?;
        match state {
            Some(state) => self.localised(
                b"EFFECT_CREATE_RAILWAY_GUN_LOCATION",
                &[
                    ("OWNER", Argument::Text(&owner)),
                    ("STATE", Argument::Text(&state)),
                ],
                scopes,
                true,
            ),
            None => self.localised(
                b"EFFECT_CREATE_RAILWAY_GUN",
                &[("OWNER", Argument::Text(&owner))],
                scopes,
                true,
            ),
        }
    }

    pub(in crate::effect_description) fn strategic_state_location(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let state = match scopes.this {
            Scope::State(state) => state,
            _ => NULL_STATE,
        };
        if self.valid_state(state)?.is_none() {
            return Ok(String::new());
        }
        if item.value.items().is_some_and(|items| items.is_empty()) {
            return Ok(String::new());
        }
        Err(
            "set_strategic_state_location_building_level needs cached strategic-location templates and their descriptions; content is not exposed to descriptions".into(),
        )
    }

    fn buildings(&self) -> Result<&content_reader_key_names::buildings::Buildings, String> {
        self.context
            .content
            .buildings
            .ok_or_else(|| self.content_gap("buildings"))
    }

    fn building_type(&self, item: &Item) -> Result<Option<&BuildingType>, String> {
        if item.value_keyword().is_none() {
            return Ok(None);
        }
        let token = item
            .value
            .c_str()
            .map_err(|why| format!("{} building type: {why}", key(item)))?;
        if self
            .context
            .content
            .key_names
            .is_some_and(|names| !names.counts.is_building(token.as_bytes()))
        {
            return Ok(None);
        }
        self.buildings()?
            .by_token(token)
            .map_err(|why| why.to_string())
    }

    fn building_label(&self, building: &BuildingType, plural: bool) -> Result<String, String> {
        let key = if plural {
            format!("{}_plural", building.name)
        } else {
            building.name.clone()
        };
        self.context
            .localisation
            .localise(key.as_bytes())
            .map_err(|why| why.to_string())
    }

    fn building_effect_name(&self, item: &Item) -> String {
        key(item)
    }

    fn building_level(&self, item: &Item, scopes: Scopes, default: i64) -> Result<i32, String> {
        let value = match field(children(item), "level") {
            Some(level) => self.scoped_value(scalar_operand(level)?, scopes)?,
            None => default,
        };
        Ok(crate::triggers::fixed_round(value) as i32)
    }

    fn building_province(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, String> {
        let (plain, scoped) = building_selector(children(item), "province");
        let province = match scoped {
            Some(scoped) => (self.scoped_value(scalar_operand(scoped)?, scopes)? / 100_000) as u32,
            None => plain as u32,
        };
        Ok(self
            .complete::<tables::Provinces>(0)?
            .contains(u128::from(province))
            .then_some(province))
    }

    fn building_state(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, String> {
        let (plain, scoped) = building_selector(children(item), "state");
        if let Some(scoped) = scoped {
            let state = self
                .evaluator()?
                .scoped_state_id(scalar_operand(scoped)?, scopes)
                .map_err(|why| why.to_string())? as u32;
            return Ok(self.valid_state(state)?.map(|row| row.id));
        }
        if self.valid_state(plain as u32)?.is_some() {
            return Ok(Some(plain as u32));
        }
        let state = match scopes.this {
            Scope::State(state) => state,
            _ => NULL_STATE,
        };
        Ok(self.valid_state(state)?.map(|row| row.id))
    }

    fn building_tag_refusal(&self, item: &Item) -> String {
        format!(
            "{} tag-selected building helper is unavailable; multiple matching targets need the ordered building roster, unrecorded by state buildings/province buildings",
            self.building_effect_name(item)
        )
    }

    fn resources(&self) -> Result<&[content_reader_key_names::resources::Resource], String> {
        self.context
            .content
            .resources
            .ok_or_else(|| self.content_gap("resources"))
    }

    fn resource_index(&self, name: &str) -> Result<i32, String> {
        if !self
            .resources()?
            .iter()
            .any(|resource| resource.token.eq_ignore_ascii_case(name))
        {
            return Ok(0);
        }
        let token = self
            .context
            .world()?
            .token_of(name.as_bytes())
            .map_err(|why| why.to_string())?;
        find(
            self.complete::<tables::ResourceDefinitions>(RESOURCE_TOKEN)?,
            |row| row.id != 0 && Some(row.token) == token,
        )?
        .map(|row| row.id as i32)
            .ok_or_else(|| {
                format!(
                    "modify_building resource {name} has no recorded database index in resource definitions"
                )
            })
    }

    fn resource_icon(&self, item: &Item) -> Result<i32, String> {
        let resource = field(children(item), "type").ok_or_else(|| {
            "add_resource missing type needs the null resource's signed icon_frame; content does not expose it".to_string()
        })?;
        let name = resource
            .value
            .c_str()
            .map_err(|why| format!("add_resource resource type: {why}"))?;
        let mut matches = self
            .resources()?
            .iter()
            .filter(|resource| resource.token.eq_ignore_ascii_case(name));
        let first = matches.next().ok_or_else(|| {
            format!(
                "add_resource unknown resource {name} needs the null resource's signed icon_frame; content does not expose it"
            )
        })?;
        if matches.any(|later| later.icon_frame != first.icon_frame) {
            return Err(format!(
                "add_resource resource {name} has differing repeated icon declarations; name lookup selects the first database record, whose content identity is unverified"
            ));
        }
        first.icon_frame.map(|frame| frame as i32).ok_or_else(|| {
            format!(
                "add_resource resource {name} signed/default/nonpositive icon_frame is not preserved by content_reader_key_names::resources::Resource.icon_frame"
            )
        })
    }
}
