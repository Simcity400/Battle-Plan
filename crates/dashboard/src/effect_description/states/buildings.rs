use super::*;
use installed_content::{
    buildings::BuildingType,
    crt::{Scanned, scan_i32},
    parser::{self, c_string},
};

// ReadScriptSignedInt32@1424AB970 retains the member on failed scans.
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

// ReadFixed5@1424ABB10 uses script_fixed5@1424B0480, not a scoped variable.
fn plain_fixed5(items: &[Item], name: &str) -> i64 {
    items
        .iter()
        .filter(|item| item.keyword_is(name))
        .filter_map(|item| installed_content::fixed5::script_fixed5(item.value.scalar()?))
        .next_back()
        .unwrap_or(0)
}

// 14031BA50 / 14031BFE0 keep plain and scoped slots independently.
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
        // 140313210 / 140317CE0 / 14048EC50 evaluate +80 before naming +58.
        let level = self.building_level(item, scopes, 0)?;
        let type_field = field(children(item), "type").ok_or_else(|| {
            format!(
                "{} has no building template; native own description dereferences +0x58 (140313210 / 140317CE0 / 14048EC50)",
                self.building_class(item)
            )
        })?;
        let building = self.building_type(type_field)?.ok_or_else(|| {
            format!(
                "{} unknown type needs the native null-template localisation strings +0x210 (140676C60 / 1414A4470), which content does not define",
                self.building_class(item)
            )
        })?;
        let building = self.building_label(building, false)?;
        let number = level.to_string();
        let (key, number_argument, expand): (&[u8], _, _) = match self.effect_class(item).map(|class| class.vtable) {
            Some(0x142737C30) /* CAddBuildingConstructionEffect */ => {
                // 140309720 defaults instant_build to true; province blocks retain +60.
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
            Some(0x142737E00) /* CSetBuildingConstructionEffect */ => (
                b"EFFECT_SET_BUILDING_CONSTRUCTION",
                Argument::Text(&number),
                false,
            ),
            Some(0x1427A75C0) /* CAddOffsiteBuildingEffect@?A0x9cb35315 */ => (
                b"EFFECT_ADD_OFFSITE_BUILDING",
                Argument::Text(&number),
                true,
            ),
            _ => return Err(format!("{} has no construction-description port", self.building_class(item))),
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
        // 14031B760 uses raw key text and retains the last provincial template.
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
            "CConstructBuildingInRandomProvinceEffect has no provincial building template; native 140315E10 dereferences +0x58".to_string()
        })?;
        let building = self.building_label(selected, false)?;
        let state = match scopes.this {
            Scope::State(state) => state,
            _ => NULL_STATE,
        };
        if self.valid_state(state)?.is_none() {
            return Err(
                "CConstructBuildingInRandomProvinceEffect has no current state; native 140315E10 passes null to state_name@1409CC380, which dereferences +0x48".into(),
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
        ) // 140315E10: no LF, and the authored amount is not shown.
    }

    pub(in crate::effect_description) fn damage_building(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 1403160C0: an explicit type bypasses all target/selector evaluation.
        let Some(type_field) = field(children(item), "type") else {
            let province = self.building_province(item, scopes)?;
            if province.is_none() && self.building_state(item, scopes)?.is_none() {
                return Ok(String::new());
            }
            return if has_building_tags(item) {
                Err(self.building_tag_refusal(item))
            } else {
                Ok(String::new()) // 140312650: no type or tags finds no building.
            };
        };
        let building = self.building_type(type_field)?.ok_or_else(|| {
            "CDamageBuildingEffect unknown type needs the native null-template plural localisation string +0x230 (140676C60 / 1414A4470), which content does not define".to_string()
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
            text.push_str(&indent_nonempty_lines(&repair, 3)); // 1424B7850.
            text.push('\n');
        }
        Ok(text)
    }

    pub(in crate::effect_description) fn remove_building(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // FindBuildingEffectTarget@1403126C0 always resolves province first.
        let province = self.building_province(item, scopes)?;
        let kind = field(children(item), "type")
            .map(|type_field| {
                self.building_type(type_field)?.ok_or_else(|| {
                    "CRemoveBuildingEffect unknown type needs native null-template province_max +0x2BC (140676C60 / 1403126C0), which content does not expose".to_string()
                })
            })
            .transpose()?;
        if kind.is_some_and(BuildingType::provincial) {
            return if province.is_none() {
                Ok(String::new()) // Remove vf25 is ReturnZero@14011CD10.
            } else {
                Err("CRemoveBuildingEffect provincial targeting: core content plumbing pending: installed_content::movement::MovementMap::terrain(id).flags for province-definition land flag +0xD2 bit0 (1403126C0)".into())
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
        let token = self.context.world()?.token_of(kind.name.as_bytes())?;
        let buildings = self
            .context
            .text
            .frame()
            .complete::<tables::StateBuildings>()
            .map_err(|why| why.to_string())?;
        if !buildings
            .iter()
            .any(|row| row.location == state && Some(row.building_type) == token)
        {
            return Ok(String::new()); // 141160CD0: level0/inactive rows still exist.
        }
        // 140316760 names the target's state before evaluating +228.
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
        // Factory 14033EAE0 leaves +58/+5C/+60 uninitialised.
        let building = field(children(item), "building").ok_or_else(|| {
            "CModifyBuildingEffect missing building leaves native template pointer +0x60 uninitialised (14033EAE0)".to_string()
        })?;
        if building.value.scalar().is_none() {
            return Ok(String::new()); // RecordError@1424ACF90 skips its invalid block.
        }
        let Some(building) = self.building_type(building)? else {
            return Ok(String::new()); // 1403851D0 tests the null template's +20.
        };
        let amount = plain_integer(children(item), "amount").ok_or_else(|| {
            "CModifyBuildingEffect missing or failed amount assignment leaves native +0x58 uninitialised (14033EAE0 / 1424AB970)".to_string()
        })?;
        let resource = field(children(item), "resource").ok_or_else(|| {
            "CModifyBuildingEffect missing resource leaves native resource index +0x5C uninitialised (14033EAE0)".to_string()
        })?;
        let resource = resource
            .value
            .c_str()
            .map_err(|why| format!("CModifyBuildingEffect resource: {why}"))?;
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
        ) // 1403851D0: FRAME is the resource +D8 database id, not icon_frame +DC.
    }

    pub(in crate::effect_description) fn add_resource(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        // 140376E70 evaluates amount, builds the icon string, then resolves state.
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
                .scoped_state_id(scalar_operand(state)?, scopes)? as u32,
            None => match scopes.this {
                Scope::State(state) => state,
                _ => NULL_STATE,
            },
        };
        if self.valid_state(state)?.is_none() {
            return Ok(String::new()); // 14036C450: no days evaluation for invalid state.
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
        // 140381910 evaluates receiver before state, without state fallback.
        let receiver = match field(children(item), "receiver") {
            Some(receiver) => self
                .evaluator()?
                .scoped_country(scalar_operand(receiver)?, scopes)?,
            None => None,
        };
        let state = match field(children(item), "state") {
            Some(state) => self
                .evaluator()?
                .scoped_state_id(scalar_operand(state)?, scopes)? as u32,
            None => 0,
        };
        if self.valid_state(state)?.is_none() {
            return Ok(String::new());
        }
        let receiver = receiver.ok_or_else(|| {
            "CGiveResourceRightsEffect invalid receiver needs reserved CCountry[0].definite_name +0x50 (140BA5980; native writer 140709950); its country-zero naming inputs are not recorded".to_string()
        })?;
        let receiver = self.country_name(receiver, true)?;
        let state = self.state_name(state)?;
        // Factory 14033E180 clears +64. Reader 1403B32D0 writes resources at
        // +210/+21C, while 140381A9B tests +64: authored resources keep this key.
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
        // CStateEffect::vf25 1403A1950 resolves selector flags with default zero.
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
        ) // 14038B470.
    }

    pub(in crate::effect_description) fn build_railway(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.require_no_effect_children(item)?; // 1403B0960 delegates unknown keys.
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
            return Ok(String::new()); // 14037A240.
        }
        // 1401F91B0 appends repeated paths; each failed %i scan retains new zero.
        let mut path = Vec::new();
        for part in children(item).iter().filter(|item| item.keyword_is("path")) {
            let items = part.value.items().ok_or_else(|| {
                "CBuildRailwayEffect path requires the native integer-array projection (1401F91B0)".to_string()
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
                    return Ok(String::new()); // 140E7F720: unset start node.
                };
                let state = self
                    .evaluator()?
                    .scoped_state_id(scalar_operand(state)?, scopes)?
                    as u32;
                if self.valid_state(state)?.is_none() {
                    return Ok(String::new());
                }
            }
            return Err(
                "CBuildRailwayEffect endpoint selection/pathfinding is not ported (140E7E560 / 140E7F720 / 140E7F370 / 140E7EF70 / 140E7A700); native state-node selection uses CState province-array order +0x18/+0x24".into(),
            );
        }
        if field(children(item), "controller_priority").is_some() {
            return Err(
                "CBuildRailwayEffect controller_priority needs native eligibility/MTTH evaluation in its controller event scope (140E7EDD0 / 1405455A0); fallback pathfinding 140E7E560 is not ported".into(),
            );
        }
        if parser::read_bool_field(children(item), "build_only_on_allied", false) {
            match scopes.this {
                Scope::Country(country) if country as i32 > 0 => {
                    return Err(
                        "CBuildRailwayEffect build_only_on_allied needs native country/controller eligibility (140E7EDD0 / 1406F3A90); fallback pathfinding 140E7E560 is not ported".into(),
                    );
                }
                Scope::Country(_) | Scope::State(_) => {} // 140E7E635: country zero gives no filter.
                _ => {
                    return Err(
                        "CBuildRailwayEffect ally filtering needs the native current-scope country tag +0x08 for this projected scope kind (140E7E635)".into(),
                    );
                }
            }
        }
        let provinces = self
            .context
            .text
            .frame()
            .complete::<tables::Provinces>()
            .map_err(|why| why.to_string())?;
        let endpoint_state = |id: u32| {
            provinces.iter().find(|row| row.id == id).map(|row| row.state).ok_or_else(|| {
                format!(
                    "CBuildRailwayEffect endpoint province {id} is absent; native 14037A240 dereferences its state pointer +0xC0"
                )
            })
        };
        // Unfiltered 140E7EDD0 returns1 without reading intermediate provinces.
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
        } // 14037A240: no LF.
    }

    pub(in crate::effect_description) fn create_railway_gun(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let location = plain_integer(children(item), "location").unwrap_or(0) as u32;
        let provinces = self
            .context
            .text
            .frame()
            .complete::<tables::Provinces>()
            .map_err(|why| why.to_string())?;
        let state = match provinces.iter().find(|row| row.id == location) {
            Some(province) if province.state == 0 => {
                return Err(
                    "CCreateRailwayGunEffect location has no state; native 14037CDA0 passes null to state_name@1409CC380, which dereferences +0x48".into(),
                );
            }
            Some(province) => Some(self.state_name(province.state)?),
            None => None,
        };
        // 14037CDA0 names the found province's state before the country adjective.
        let country = match scopes.this {
            Scope::Country(country) if country as i32 > 0 => country,
            Scope::Country(_) | Scope::State(_) => {
                return Err(
                    "CCreateRailwayGunEffect country zero needs reserved CCountry[0].adjective +0x30 (140BA5560; native writer 140709950); its country-zero naming inputs are not recorded".into(),
                );
            }
            _ => {
                return Err(
                    "CCreateRailwayGunEffect OWNER needs native current-scope country tag +0x08 for this projected scope kind (14037CDA0 / 140BA5560); the scope projection does not expose it".into(),
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
        } // 14037CDA0: no LF.
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
            "CSetStrategicStateLocationBuildingLevel needs cached strategic-location templates (14051BBB0) and their description helper 140AB07D0; content is not exposed to descriptions".into(),
        ) // 14051B6F0.
    }

    fn buildings(&self) -> Result<&installed_content::buildings::Buildings, String> {
        self.context
            .content
            .buildings
            .ok_or_else(|| self.content_gap("buildings"))
    }

    fn building_type(&self, item: &Item) -> Result<Option<&BuildingType>, String> {
        // FindByToken@140676C60 uses the value token id, not its quoted text.
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
            return Ok(None); // Complete key names can prove a native null-template lookup.
        }
        self.buildings()?
            .by_token(token)
            .map_err(|why| why.to_string())
    }

    fn building_label(&self, building: &BuildingType, plural: bool) -> Result<String, String> {
        // 1414A4470 localises declared spelling and the literal _plural suffix.
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

    fn building_class(&self, item: &Item) -> &'static str {
        self.effect_class(item)
            .map_or("building effect", |class| class.name)
    }

    fn building_level(&self, item: &Item, scopes: Scopes, default: i64) -> Result<i32, String> {
        let value = match field(children(item), "level") {
            Some(level) => self.scoped_value(scalar_operand(level)?, scopes)?,
            None => default,
        };
        Ok(crate::triggers::fixed_round(value) as i32) // 1424D8760.
    }

    fn building_province(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, String> {
        let (plain, scoped) = building_selector(children(item), "province");
        let province = match scoped {
            Some(scoped) => (self.scoped_value(scalar_operand(scoped)?, scopes)? / 100_000) as u32,
            None => plain as u32,
        }; // 140319930 truncates division; it does not round.
        Ok(self
            .context
            .text
            .frame()
            .complete::<tables::Provinces>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.id == province)
            .map(|row| row.id))
    }

    fn building_state(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, String> {
        let (plain, scoped) = building_selector(children(item), "state");
        if let Some(scoped) = scoped {
            let state = self
                .evaluator()?
                .scoped_state_id(scalar_operand(scoped)?, scopes)? as u32;
            return Ok(self.valid_state(state)?.map(|row| row.id));
        }
        if self.valid_state(plain as u32)?.is_some() {
            return Ok(Some(plain as u32));
        }
        // 140319B80: only the plain selector falls back to current state.
        let state = match scopes.this {
            Scope::State(state) => state,
            _ => NULL_STATE,
        };
        Ok(self.valid_state(state)?.map(|row| row.id))
    }

    fn building_tag_refusal(&self, item: &Item) -> String {
        format!(
            "{} tag-selected building helper is not ported (1403126C0 / 1411620D0); multiple matching targets need CBuildingStatus ordered pointer vector +0x38/+0x44, unrecorded by StateBuildings/ProvinceBuildings (writer GetOrCreateBuilding@14115E150; reset 14115FD70)",
            self.building_class(item)
        )
    }

    fn resources(&self) -> Result<&[installed_content::resources::Resource], String> {
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
            return Ok(0); // FindByName@140AB2DF0 returns the native null resource.
        }
        let token = self.context.world()?.token_of(name.as_bytes())?;
        self.context
            .text
            .frame()
            .complete::<tables::ResourceDefinitions>()
            .map_err(|why| why.to_string())?
            .iter()
            .filter(|row| row.id != 0 && Some(row.token) == token)
            .map(|row| row.id)
            .min()
            .map(|id| id as i32)
            .ok_or_else(|| {
                format!(
                    "CModifyBuildingEffect resource {name} has no recorded native database index (CStrategicResource +0xD8, constructor 140BBB090; ResourceDefinitions)"
                )
            })
    }

    fn resource_icon(&self, item: &Item) -> Result<i32, String> {
        let resource = field(children(item), "type").ok_or_else(|| {
            "CAddResourceEffect missing type needs the native null resource's signed icon_frame +0xDC (140376E70); content does not expose it".to_string()
        })?;
        let name = resource
            .value
            .c_str()
            .map_err(|why| format!("CAddResourceEffect resource type: {why}"))?;
        let mut matches = self
            .resources()?
            .iter()
            .filter(|resource| resource.token.eq_ignore_ascii_case(name));
        let first = matches.next().ok_or_else(|| {
            format!(
                "CAddResourceEffect unknown resource {name} needs the native null resource's signed icon_frame +0xDC (140376E70); content does not expose it"
            )
        })?;
        if matches.any(|later| later.icon_frame != first.icon_frame) {
            return Err(format!(
                "CAddResourceEffect resource {name} has differing repeated icon declarations; native FindByName@140AB2DF0 selects the first database record, whose content identity is unverified"
            ));
        }
        first.icon_frame.map(|frame| frame as i32).ok_or_else(|| {
            format!(
                "CAddResourceEffect resource {name} signed/default/nonpositive icon_frame +0xDC is not preserved by installed_content::resources::Resource.icon_frame (140BBB090 / 140BBD150 / 140376E70)"
            )
        })
    }
}
