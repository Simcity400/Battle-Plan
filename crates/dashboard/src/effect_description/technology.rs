use super::*;

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    fn technology_catalog(&self) -> Result<&installed_content::technology::Catalog, String> {
        self.context.content.doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?
            .technologies.as_ref().ok_or("technology description catalogue unavailable".into())
    }

    // CAddBreakthroughPoints/Progress own 140492790/140492DF0; readers
    // 140496160/140496280 store specialization tokens and integer/fixed5
    // value respectively. Each row is ensured LF, then indented two spaces.
    // Header/row keys: points 1427A9D18/1427A9D40, progress 1427A9E40/1427A9E68.
    pub(super) fn breakthrough(&self, item: &Item, scopes: Scopes, progress: bool) -> Result<String, String> {
        let fields = children(item);
        let mut tokens = Vec::new();
        let world = self.context.world()?;
        let catalog = self.character_content()?.special_projects.as_ref().map_err(Clone::clone)?;
        for entry in fields.iter().filter(|entry| entry.keyword_is("specialization")) {
            // all@0x2B97 -> 14048F9B0 inserts in manager registration order.
            if entry.value_keyword().as_deref() == Some("all") {
                for name in catalog.specializations_in_order()? {
                    tokens.push(world.token_of(name.as_bytes())?.ok_or("LexicalTokens lacks breakthrough specialization")?);
                }
                continue;
            }
            let operand = entry.value.scalar().unwrap_or_default();
            if catalog.specialization(&installed_content::tokens::Token::c_str(operand))?.is_none() { continue; }
            let token = world.token_of(operand)?.ok_or("LexicalTokens lacks breakthrough specialization")?;
            tokens.push(token);
        }
        if tokens.is_empty() { return Ok(String::new()); }
        let raw_country = match scopes.this { Scope::Country(country) => world.raw_country(country).unwrap_or(0), _ => 0 };
        let amount = field(fields, "value");
        let mut rows = String::new();
        for token in installed_content::special_projects::specialization_order(&tokens) {
            let operand = world.token(token)?.ok_or("LexicalTokens lacks specialization text")?;
            let Some(name) = self.char_specialization_name(operand, raw_country, scopes)? else { continue; };
            let amount = if progress {
                Argument::Fixed5(amount.map(installed_content::modifiers::stored).unwrap_or(0))
            } else { Argument::Int32(amount.and_then(|field| field.value.scalar()).map(integer).unwrap_or(0)) };
            let mut row = self.char_native_text(if progress { b"ADD_BREAKTHROUGH_PROGRESS_EFFECT_SPECIALIZATION" } else { b"ADD_BREAKTHROUGH_POINTS_EFFECT_SPECIALIZATION" }, &[("SPECIALIZATION", Argument::Text(&name)), ("AMOUNT", amount)], scopes, false)?;
            if !row.ends_with('\n') { row.push('\n'); }
            rows.push_str(&row);
        }
        let mut header = self.char_native_text(if progress { b"ADD_BREAKTHROUGH_PROGRESS_EFFECT_DESC" } else { b"ADD_BREAKTHROUGH_POINTS_EFFECT_DESC" }, &[], scopes, false)?;
        if !header.ends_with('\n') { header.push('\n'); }
        header.push_str(&indent_nonempty_lines(&rows, 2));
        if !header.ends_with('\n') { header.push('\n'); }
        Ok(header)
    }

    // CAddFactionPowerProjectionEffect::vf7 1404ABFC0 evaluates +0x58
    // then expands in a copied environment with its faction pointer installed.
    // Country membership is recorded; the independent faction carrier is core-owned.
    // ADD_FACTION_POWER_PROJECTION_EFFECT_NO_FACTION@1427B39A8.
    pub(super) fn faction_power_projection(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let amount = self.scoped_value(scalar_operand(item)?, scopes)?;
        let mut target = item.clone();
        target.value = installed_content::parser::Value::Scalar(b"THIS".to_vec());
        let country = self.country_target(&target, scopes)?.unwrap_or(0);
        if country > 0 && self.context.world()?.faction(country)?.is_some() {
            return Err("copied faction localisation scope is pending lead I's core (1404ABFC0/141174150)".into());
        }
        self.char_native_text(b"ADD_FACTION_POWER_PROJECTION_EFFECT_NO_FACTION", &[("VAL", Argument::Fixed5(amount))], scopes, true)
    }

    // 140493450 -> 140529480: the project type/ID are recorded (Projects,
    // type86, +8/+0x0C). Lead I owns the +0x90/+0x94 scope/evaluator carrier.
    pub(super) fn project_progress(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        self.scoped_value(scalar_operand(item)?, scopes)?;
        Err("project scope carrier +0x90/+0x94 is pending lead I's core (140529480, own 140493450)".into())
    }

    // 140494060/1405381D0, reader 1404964D0: project/state/scientist/
    // iteration_output/show_modifiers only. A resolved project's native
    // modifier output is also core-owned (14144AC10, non-default mask).
    pub(super) fn complete_project(&self, _item: &Item, _scopes: Scopes) -> Result<String, String> {
        Err("project selector sp: and scope carrier are pending lead I's core (1405382B0/1405381D0, own 140494060)".into())
    }

    // 1404A3F20 -> 1404A43D0: province order, active facilities/programs,
    // wars and project definitions are already recorded. The independent raid
    // reference +0x88/+0x8C and its ROOT graph require lead I's carrier.
    pub(super) fn raid_project_progress(&self, _item: &Item, _scopes: Scopes) -> Result<String, String> {
        Err("raid scope carrier +0x88/+0x8C is pending lead I's core (1404A43D0, own 1404A3F20)".into())
    }

    // CTechnologyTemplateVariable 14139BDC0/14139B770/14139B4A0:
    // named templates first; only explicit var: evaluates a native +0x3C
    // handle. The recorded database_index (+0x38) is not that handle.
    fn technology_template(
        &self,
        operand: &[u8],
        scopes: Scopes,
    ) -> Result<Option<&installed_content::technology::Definition>, String> {
        let catalog = self.technology_catalog()?;
        if let Some(template) = catalog.get(operand)? { return Ok(Some(template)); }
        if !operand.starts_with(b"var:") { return Ok(None); }
        let handle = (self.scoped_value(operand, scopes)? / 100_000) as u32;
        let rows = self.context.text.frame().complete::<tables::TechnologyDefinitions>()
            .map_err(|why| why.to_string())?;
        match rows.iter().find(|row| row.technology_handle == handle) {
            Some(row) => catalog.get(&row.id),
            None => Ok(None),
        }
    }

    // LocalizeForCountry 140BBA780: a defined TAG_key takes precedence;
    // neither the name nor the eventual technology wrapper is expanded.
    fn technology_country_name(&self, key: &[u8], country: u32, original: bool, scopes: Scopes) -> Result<String, String> {
        let mut tag = if country > 0 { self.tag(country)?.as_bytes().to_vec() } else { Vec::new() };
        if original && country > 0 {
            // 140EC9A60 -> 140BA61F0/1406DF830. CountryAppearance.name_tag
            // already applies the native dynamic-country boundary predicate.
            let world = self.context.world()?;
            let appearance = self.context.text.frame().complete::<tables::CountryAppearance>()
                .map_err(|why| why.to_string())?.iter()
                .find(|row| world.slot(row.country) == Some(country))
                .ok_or("CountryAppearance lacks technology owner")?;
            if !appearance.name_tag.is_empty() { tag = appearance.name_tag.clone(); }
        }
        let mut candidate = tag;
        candidate.push(b'_');
        candidate.extend_from_slice(key);
        let candidate_defined = std::str::from_utf8(&candidate).ok()
            .is_some_and(|candidate| self.context.localisation.get(candidate).is_some());
        self.localised(if country > 0 && candidate_defined { &candidate } else { key }, &[], scopes, false)
    }

    // 140EC9A60 selects a sole enabled equipment name unless the template's
    // force_use_small_tech_layout (+0x40A) is set. 140EBDC40 constructs this
    // roster from content; feature 0x21 is SimulationFeatures bit7 (1401AE640).
    fn technology_name(&self, template: &installed_content::technology::Definition, country: u32, scopes: Scopes, runtime: bool) -> Result<String, String> {
        let catalog = self.technology_catalog()?;
        let mut equipment = Vec::new();
        if runtime && !template.small_layout {
            for entry in &template.equipment {
                if entry.keyword_is("limit") {
                    match self.evaluator()?.evaluate(children(entry), Scopes::empty()) {
                        crate::triggers::Eval::True => continue,
                        crate::triggers::Eval::False => break,
                        crate::triggers::Eval::Unknown(why) => return Err(why.into_iter().collect::<Vec<_>>().join("; ")),
                    }
                }
                let Some(name) = entry.value.scalar() else { return Err("technology equipment reader is unsettled (140AC3C70)".into()); };
                let Some(index) = catalog.equipment.get(name) else { continue; };
                let definition = &catalog.equipment.equipment[index];
                if catalog.equipment.category(definition) & 0x200 != 0 {
                    let features = self.context.text.frame().complete::<tables::SimulationFeatures>()
                        .map_err(|why| why.to_string())?.first().ok_or("SimulationFeatures absent")?;
                    if features.dlc_mask & 0x80 == 0 { continue; }
                }
                if !equipment.contains(&index) { equipment.push(index); }
            }
        }
        let key = if equipment.len() == 1 {
            catalog.equipment.equipment[equipment[0]].name.as_slice()
        } else if !template.parent.is_empty() { &template.parent } else { &template.id };
        self.technology_country_name(key, country, runtime, scopes)
    }

    // CSetTechnologyEffect reader 140323D80 accepts popup and technology
    // keys only. Own 140321D70 ignores the level byte and describes in order.
    // EFFECT_GIVE_TECHNOLOGY@14273C300.
    pub(super) fn set_technology(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let mut target = item.clone();
        target.value = installed_content::parser::Value::Scalar(b"THIS".to_vec());
        let country = self.country_target(&target, scopes)?.unwrap_or(0); // 14052B6A0 includes state owner.
        let mut out = String::new();
        for field in children(item).iter().filter(|field| !field.keyword_is("popup")) {
            let Some(template) = self.technology_template(field.key.as_deref().unwrap_or_default(), scopes)? else { continue; };
            let name = self.technology_name(template, country, scopes, false)?;
            out.push_str(&self.char_native_line(b"EFFECT_GIVE_TECHNOLOGY", &[("NAME", Argument::Text(&name))], scopes, false)?);
        }
        Ok(out)
    }

    // CAddTechBonusEffect 1403208E0: technology list (+0x58) precedes
    // deduplicated categories (+0x70); separators depend on original positions.
    // Own fields are exactly reader 140323AC0's keys; name is not described.
    // Both/ahead/bonus keys@14273BC60/14273BC08/14273BC38, uses@14273BBE8.
    pub(super) fn technology_bonus(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let technology: Vec<_> = fields.iter().filter(|field| field.keyword_is("technology")).collect();
        let mut category = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for entry in fields.iter().filter(|field| field.keyword_is("category")) {
            let token = installed_content::tokens::Token::c_str(entry.value.scalar().unwrap_or_default());
            if seen.insert(token) { category.push(entry); }
        }
        let country = match scopes.this { Scope::Country(country) => country, _ => 0 };
        let mut list = String::new();
        for (index, entry) in technology.iter().enumerate() {
            if let Some(template) = self.technology_template(entry.value.scalar().unwrap_or_default(), scopes)? {
                list.push_str(&self.technology_name(template, country, scopes, true)?);
                if index + 1 < technology.len() || !category.is_empty() { list.push_str(", "); }
            }
        }
        for (index, entry) in category.iter().enumerate() {
            if let Some(name) = self.technology_catalog()?.category(entry.value.scalar().unwrap_or_default())? {
                list.push_str(&self.technology_country_name(name, country, false, scopes)?);
                if index + 1 < category.len() { list.push_str(", "); }
            }
        }
        // 14225CBD0 replaces the final comma-space by one space + AND +
        // suffix. The AND localisation supplies its own trailing space.
        if let Some(index) = list.rfind(", ") {
            let and = self.char_native_text(b"AND", &[], scopes, false)?;
            list.replace_range(index..index + 2, &format!(" {and}"));
        }
        let ahead = field(fields, "ahead_reduction").map(installed_content::modifiers::stored).unwrap_or(0);
        let bonus = field(fields, "bonus").map(installed_content::modifiers::stored).unwrap_or(0);
        let uses = field(fields, "uses").and_then(|field| field.value.scalar()).map(integer).unwrap_or(1);
        if ahead == 0 && bonus == 0 { return Ok(String::new()); }
        let uses_text = self.char_native_text(b"EFFECT_TECH_BONUS_USES", &[("USES", Argument::Int32(uses))], scopes, false)?;
        if ahead == 0 {
            self.char_native_line(b"EFFECT_ADD_TECH_BONUS_BONUS_ONLY", &[("LIST", Argument::Text(&list)), ("BONUS", Argument::Fixed5(bonus)), ("USES", Argument::Text(&uses_text))], scopes, false)
        } else if bonus == 0 {
            self.char_native_line(b"EFFECT_ADD_TECH_BONUS_AHEAD_ONLY", &[("LIST", Argument::Text(&list)), ("AHEAD", Argument::Fixed5(ahead)), ("USES", Argument::Text(&uses_text))], scopes, false)
        } else {
            // The both-values branch passes int32 USES, unlike either single-value branch.
            self.char_native_line(b"EFFECT_ADD_TECH_BONUS", &[("LIST", Argument::Text(&list)), ("AHEAD", Argument::Fixed5(ahead)), ("BONUS", Argument::Fixed5(bonus)), ("USES", Argument::Int32(uses))], scopes, false)
        }
    }

    fn doctrine_name(&self, kind: u8, operand: &[u8], scopes: Scopes) -> Result<Option<String>, String> {
        let database = self.context.content.doctrines.ok_or_else(|| self.content_gap("doctrines"))?;
        database.name(kind, operand)?.map(|name| self.localised(&name.value, &[], scopes, false)).transpose()
    }

    // 14051E0E0/14051E1A0: null template is empty; name+0x28 plain,
    // NAME@142727E7C, no LF. Sub's block fields are reader 14051ECD0.
    // SET_GRAND_DOCTRINE_EFFECT@1427BAE58, SET_SUB_DOCTRINE_EFFECT@1427BAFD0.
    pub(super) fn set_doctrine(&self, item: &Item, scopes: Scopes, sub: bool) -> Result<String, String> {
        let operand = item.value.scalar().or_else(|| field(children(item), "sub_doctrine").and_then(|field| field.value.scalar())).unwrap_or_default();
        let Some(name) = self.doctrine_name(u8::from(sub), operand, scopes)? else { return Ok(String::new()); };
        self.char_native_text(if sub { b"SET_SUB_DOCTRINE_EFFECT" } else { b"SET_GRAND_DOCTRINE_EFFECT" }, &[("NAME", Argument::Text(&name))], scopes, false)
    }

    // Shared own 14051D460, daily vf25 14051E6D0, bonus vf25 14051E7C0.
    // Reader fallback 141468860 consumes all five filters before ReadEffect.
    // TRACK_EFFECT_ALL_TRACKS/COMPLEX_HEADER@1427BB078/1427BB110;
    // only keys@1427BB090/1427BB0B0/1427BB0D8/1427BB0F8;
    // condition keys@1427BB130/1427BB150/1427BB178/1427BB1A0/1427BB1C8.
    pub(super) fn mastery_bonus(&self, item: &Item, scopes: Scopes, daily: bool) -> Result<String, String> {
        let fields = children(item);
        let amount = field(fields, if daily { "amount" } else { "bonus" }).map(installed_content::modifiers::stored).unwrap_or(0);
        // Constructors 14051C900/14051C9A0 initialize +0x88 days to zero.
        let days = field(fields, "days").and_then(|field| field.value.scalar()).map(integer).unwrap_or(0);
        let inner = self.char_native_text(if daily { b"ADD_DAILY_MASTERY_EFFECT" } else { b"ADD_MASTERY_BONUS_EFFECT" }, &[(if daily { "AMOUNT" } else { "BONUS" }, Argument::Fixed5(amount)), ("DAYS", Argument::Int32(days))], scopes, false)?;
        let mut names = Vec::new();
        for (selector, kind, only, condition) in [
            ("folder", 3, &b"TRACK_EFFECT_ONLY_FOLDER"[..], &b"TRACK_EFFECT_CONDITION_FOLDER"[..]),
            ("grand_doctrine", 0, b"TRACK_EFFECT_ONLY_GRAND_DOCTRINE", b"TRACK_EFFECT_CONDITION_GRAND_DOCTRINE"),
            ("sub_doctrine", 1, b"TRACK_EFFECT_ONLY_SUBDOCTRINE", b"TRACK_EFFECT_CONDITION_SUBDOCTRINE"),
            ("track", 2, b"TRACK_EFFECT_ONLY_TRACK", b"TRACK_EFFECT_CONDITION_TRACK_TYPE"),
        ] {
            let Some(operand) = field(fields, selector).and_then(|field| field.value.scalar()) else { continue; };
            let name = if kind == 3 {
                let database = self.context.content.doctrines.ok_or_else(|| self.content_gap("doctrines"))?;
                database.folder_exact(operand)?.map(|folder| self.localised(&folder.name, &[], scopes, false)).transpose()?
            } else { self.doctrine_name(kind, operand, scopes)? };
            if let Some(name) = name { names.push((only, condition, name)); }
        }
        let index = field(fields, "track_index").and_then(|field| field.value.scalar()).map(integer).unwrap_or(-1);
        if names.is_empty() && index < 0 {
            return self.char_native_text(b"TRACK_EFFECT_ALL_TRACKS", &[("TRACK_EFFECT_DESC", Argument::Text(&inner))], scopes, false);
        }
        if names.len() == 1 && index < 0 {
            return self.char_native_text(names[0].0, &[("TRACK_EFFECT_DESC", Argument::Text(&inner)), ("NAME", Argument::Text(&names[0].2))], scopes, false);
        }
        let mut out = self.char_native_text(b"TRACK_EFFECT_COMPLEX_HEADER", &[("TRACK_EFFECT_DESC", Argument::Text(&inner))], scopes, false)?;
        for (_, condition, name) in names {
            let row = self.char_native_text(condition, &[("NAME", Argument::Text(&name))], scopes, false)?;
            append_description(&mut out, &row);
        }
        if index >= 0 {
            let row = self.char_native_text(b"TRACK_EFFECT_CONDITION_TRACK_INDEX", &[("INDEX", Argument::Int32(index))], scopes, false)?;
            append_description(&mut out, &row);
        }
        Ok(out)
    }

    // CAddResearchSlotEffect::GetDescription 140320610; vf26 evaluates
    // CScopedVariable +0x58 as int32. Zero takes the removal branch too.
    // Add/remove/set keys@14273BEB8/14273BED8/14273BFD8.
    pub(super) fn research_slots(
        &self,
        item: &Item,
        scopes: Scopes,
        set: bool,
    ) -> Result<String, String> {
        let count = (self.scoped_value(scalar_operand(item)?, scopes)? / 100_000) as i32;
        if set {
            // CSetResearchSlotsEffect::GetDescription 140321CA0, no LF.
            return self.char_native_text(
                b"EFFECT_SET_RESEARCH_SLOTS",
                &[("COUNT", Argument::Int32(count))],
                scopes,
                false,
            );
        }
        let (key, count) = if count > 0 {
            (&b"EFFECT_ADD_RESEARCH_SLOT"[..], count)
        } else {
            (&b"EFFECT_REMOVE_RESEARCH_SLOT"[..], count.wrapping_neg())
        };
        self.char_native_line(key, &[("COUNT", Argument::Int32(count))], scopes, false)
    }

    // CInheritTechnologyEffect::GetDescription 1403216F0; country target
    // 141384830, ordinary country name 140BA5960, COUNTRY@142728968. No LF.
    // EFFECT_INHERIT_TECHNOLOGY@14273C4F8.
    pub(super) fn inherit_technology(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(country) = self.country_target(item, scopes)?.filter(|country| *country > 0) else {
            return Ok(String::new());
        };
        let name = self.country_name(country, false)?;
        self.char_native_text(
            b"EFFECT_INHERIT_TECHNOLOGY",
            &[("COUNTRY", Argument::Text(&name))],
            scopes,
            false,
        )
    }

    // Own 1402FCAD0/1402FE970: only +0x58's scoped value is described;
    // VAL@14272F830, caller-scope expansion followed by an unconditional LF.
    // Add/set keys@142730328/142730458.
    pub(super) fn mio_research_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
        set: bool,
    ) -> Result<String, String> {
        let value = self.scoped_value(scalar_operand(item)?, scopes)?;
        self.char_native_line(
            if set { b"EFFECT_MIO_SET_RESEARCH_BONUS" } else { b"EFFECT_MIO_ADD_RESEARCH_BONUS" },
            &[("VAL", Argument::Fixed5(value))],
            scopes,
            true,
        )
    }

    // CStealRandomTechBonusEffect::GetDescription 140322180 describes only
    // instant (+0x194). The technology list belongs to selected vf9 140322220.
    // EFFECT_STEAL_RANDOM_TECH/INSTANT@14273BAB8/14273BA90.
    pub(super) fn steal_technology_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let instant = field(children(item), "instant")
            .is_some_and(|value| value.value_keyword().as_deref() == Some("yes"));
        self.char_native_text(
            if instant { b"EFFECT_STEAL_RANDOM_TECH_INSTANT" } else { b"EFFECT_STEAL_RANDOM_TECH" },
            &[],
            scopes,
            true,
        )
    }

    // CSetFactionResearhUnlockedEffect::vf7 1404AFCB0: the inner key's
    // REASEARCH spelling is native, and is localised without expansion.
    // FACTION_ACCESS_TAB_REASEARCH@1427B3470.
    pub(super) fn faction_research_unlocked(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = self.char_native_text(b"FACTION_ACCESS_TAB_REASEARCH", &[], scopes, false)?;
        self.char_native_text(
            if item.value_keyword().as_deref() == Some("yes") { b"FACTION_ACCESS_TAB_UNLOCK" } else { b"FACTION_ACCESS_TAB_LOCK" },
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )
    }

    fn folder_name(&self, value: &[u8]) -> Result<String, String> {
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        let folder = database.folder(value)?.ok_or_else(|| {
            format!(
                "doctrine folder {} is absent",
                String::from_utf8_lossy(value)
            )
        })?;
        self.context
            .localisation
            .localise(&folder.name)
            .map_err(|why| why.to_string())
    }

    // 14051D460 -> 14051E8B0; only the reached folder selector.
    pub(super) fn mastery(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        for field in fields {
            if !matches!(
                key(field).to_ascii_lowercase().as_str(),
                "amount" | "folder"
            ) {
                return Err(format!(
                    "mastery filter {} is outside reached folder form (14051E8B0)",
                    key(field)
                ));
            }
        }
        let amount = field(fields, "amount")
            .map(installed_content::modifiers::stored)
            .unwrap_or(0);
        let folder = field(fields, "folder")
            .and_then(|item| item.value.scalar())
            .ok_or("mastery folder form absent (14051E8B0)")?;
        let database = self
            .context
            .content
            .doctrines
            .ok_or_else(|| self.content_gap("doctrines"))?;
        let folder = database
            .folder_exact(folder)?
            .ok_or("mastery folder is absent (14051D460)")?;
        let name = self
            .context
            .localisation
            .localise(&folder.name)
            .map_err(|why| why.to_string())?;
        let effect = self.localised(
            b"ADD_MASTERY_EFFECT",
            &[("AMOUNT", Argument::Fixed5(amount))],
            scopes,
            false,
        )?;
        self.localised(
            b"TRACK_EFFECT_ONLY_FOLDER",
            &[
                ("TRACK_EFFECT_DESC", Argument::Text(&effect)),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            false,
        )
    }

    // 14031FCD0: folder display, authored reduction, formatted use count.
    pub(super) fn doctrine_reduction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        // Native +0x58 technology and +0x70 category lists precede +0x88
        // folders (14031FCD0); never silently omit a technology operand.
        if let Some(technology) = field(fields, "technology") {
            return Err(format!(
                "technology {} doctrine reduction description is not ported (14031FCD0 +0x58)",
                String::from_utf8_lossy(technology.value.scalar().unwrap_or_default())
            ));
        }
        let reduction = field(fields, "cost_reduction")
            .map(installed_content::modifiers::stored)
            .unwrap_or(0);
        let uses = field(fields, "uses")
            .and_then(|item| item.value.scalar())
            .map(installed_content::fixed5::decimal_i32)
            .unwrap_or(1);
        let mut folders = Vec::new();
        for item in fields.iter().filter(|item| item.keyword_is("category")) {
            folders.push(self.folder_name(item.value.scalar().unwrap_or_default())?);
        }
        if folders.is_empty() {
            return Err("non-folder doctrine reduction is outside reached form (14031FCD0)".into());
        }
        let list = match folders.as_slice() {
            [one] => one.clone(),
            _ => {
                let last = folders.pop().unwrap();
                format!(
                    "{} {} {last}",
                    folders.join(", "),
                    self.context
                        .localisation
                        .localise(b"AND") // Native literal at 14273BBDC, used by 14225CBD0.
                        .map_err(|why| why.to_string())?
                )
            }
        };
        let uses = self.localised(
            b"EFFECT_TECH_BONUS_USES",
            &[("USES", Argument::Int32(uses))],
            scopes,
            false,
        )?;
        self.line(
            b"EFFECT_ADD_DOCTRINE_COST_REDUCTION",
            &[
                ("LIST", Argument::Text(&list)),
                ("REDUCTION", Argument::Fixed5(reduction)),
                ("USES", Argument::Text(&uses)),
            ],
            scopes,
            false,
        )
    }
}
