//! Event and targeted-decision descriptions, 14037BDC0 / 14038BF60 / 140720D40.

use super::*;
use crate::{
    scripted_text::{EventObject, EventScope, Link, random},
    triggers::{NULL_STATE, Scope},
};
use installed_content::decisions::Decision;
use replay::rows::{DecisionRow, tables};

const DECISION_RANDOM_COUNT_GAP: &str =
    "DecisionRandomItems is not complete for this decision-description seed (140721170/1407290D0)";

mod descriptions;
#[cfg(test)]
mod tests;

/// 14071C730 compares both native target slots and ignores lifecycle/ignore.
fn matching<'a>(
    rows: &'a [DecisionRow],
    definition: &[u8],
    owner: u32,
    mission: bool,
    target_country: Option<u32>,
    target_state: u32,
    slot: impl Fn(u32) -> Option<u32>,
) -> Result<Option<&'a DecisionRow>, String> {
    let mut candidates = rows
        .iter()
        .filter(|row| {
            row.container == if mission { 4 } else { 3 }
                && row.token.eq_ignore_ascii_case(definition)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|row| row.ordinal);
    for row in candidates {
        let row_owner = slot(row.country)
            .ok_or_else(|| format!("CountryLinks lacks Decisions owner {}", row.country))?;
        if row_owner != owner || row.numeric_target != target_state {
            continue;
        }
        let country = if row.country_target == 0 {
            None
        } else {
            Some(slot(row.country_target).ok_or_else(|| {
                format!("CountryLinks lacks Decisions target {}", row.country_target)
            })?)
        };
        if country == target_country {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// 1407290D0 skips the execution-count lookup only for fixed_random_seed.
fn decision_seed(
    game_seed: i32,
    definition: &Decision,
    additive: Option<Option<i32>>,
    target: i32,
) -> Result<u32, String> {
    let additive = if definition.fixed_random_seed {
        0
    } else {
        additive
            .ok_or("Decisions description_seed is unread (1407290D0)")?
            .unwrap_or(0)
    };
    Ok(game_seed
        .wrapping_add(definition.seed_component)
        .wrapping_add(additive)
        .wrapping_add(target) as u32)
}

/// 140721730 tests cosmetic/tag/base keys, each ideology rung before its base.
fn name_key(
    base: &str,
    cosmetic: &str,
    tag: &str,
    ideology: &str,
    exists: impl Fn(&str) -> bool,
) -> String {
    let suffix = format!("_{ideology}");
    let mut candidates = Vec::new();
    if !cosmetic.is_empty() {
        candidates.extend([
            format!("{cosmetic}_{base}{suffix}"),
            format!("{cosmetic}_{base}"),
        ]);
    }
    candidates.extend([
        format!("{tag}_{base}{suffix}"),
        format!("{tag}_{base}"),
        format!("{base}{suffix}"),
        base.to_owned(),
    ]);
    candidates
        .into_iter()
        .find(|key| exists(key))
        .unwrap_or_else(|| base.to_owned())
}

impl Renderer<'_, '_, '_> {
    // 140373020: +0x118 distinguishes authored definitions from the null sentinel
    // (140716650 / 140716C90); it is not an active-decision timer guard.
    pub(super) fn decision_remove_days(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = field(fields, "decision")
            .and_then(|item| item.value.read_string())
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or("decision content is unread (140373020)")?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let name = self.decision_title(definition, scopes)?;
        // ReadEffect 1403AED20 uses ReadScriptSignedInt32 1424AB970. A failed
        // scan leaves the field unchanged. 140373082 negates it in both branches.
        let days = fields
            .iter()
            .filter(|item| item.keyword_is("days"))
            .filter_map(|item| {
                item.value
                    .scalar()
                    .and_then(|value| installed_content::crt::scan_i32(value).stored())
            })
            .next_back()
            .unwrap_or(0);
        self.line(
            if days < 0 {
                b"REMOVE_DAYS_TO_DECISION" // 14275E688.
            } else {
                b"ADD_DAYS_TO_DECISION" // 14275E6A0.
            },
            &[
                ("DAYS", Argument::Int32(days.wrapping_neg())), // 14275179C.
                ("DECISION", Argument::Text(&name)),            // 14275E678.
            ],
            scopes,
            true,
        )
    }

    // GetLocalizedNameForScope 140720B40 selects the stored alternate/base key,
    // then 140721730's country ladder. It expands in the supplied scope unchanged.
    fn decision_title(&self, definition: &Decision, scopes: Scopes) -> Result<String, String> {
        let label_key = self.decision_name_key(definition, self.current_country(scopes)?)?;
        self.localised(label_key.as_bytes(), &[], scopes, true)
    }

    pub(super) fn custom_tooltip(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        // 1413832B0 -> 1402F19A0 evaluates the shared bound reader's tree
        // with empty supplied arguments and the caller's SScopeLocalizer.
        self.bound_localisation(
            &installed_content::gui::native::BoundLocalization::from_value(&item.value),
            scopes,
            true,
        )
    }

    // 14037BDC0: fresh seeded title scope; FROM is the caller's ROOT.
    pub(super) fn country_event(&mut self, item: &Item, scopes: Scopes) -> Result<String, String> {
        if let Some(desc) = field(children(item), "desc")
            .and_then(|item| item.value.read_string())
            .filter(|desc| !desc.is_empty())
        {
            return self.line(desc, &[], scopes, true);
        }
        let id = item
            .value
            .scalar()
            .or_else(|| field(children(item), "id").and_then(|item| item.value.scalar()))
            .ok_or("country_event has no id (14037BDC0)")?;
        let event = (self
            .context
            .content
            .events
            .ok_or("referenced event lookup unavailable")?)(id)?;
        if let Some(why) = &event.refused {
            return Err(why.clone());
        }
        let mut title_scope = EventScope::of(EventObject::Country(self.current_country(scopes)?));
        let cursor = scopes.cursor.unwrap_or(0);
        let at = {
            let mut graph = self.graph.borrow_mut();
            title_scope.from = graph.scopes[cursor].root;
            if title_scope.from == Link::Itself {
                title_scope.from = Link::Scope(cursor);
            }
            let parent = &mut graph.scopes[cursor];
            let seed = crate::scripted_text::random::scoped_draw(
                parent.random_seed,
                parent.random_counter,
            ) as u32;
            parent.random_seed = parent.random_seed.wrapping_add(1); // 14037BDC0 advances +0x0C.
            title_scope.random_seed = crate::scripted_text::random::scope_seed(seed);
            title_scope.random_counter = crate::scripted_text::random::scope_counter(seed);
            // 14037BDC0 derives title lanes; only a later selection needs them.
            title_scope.random_seed_refusal = parent.random_seed_refusal;
            let at = graph.scopes.len();
            graph.scopes.push(title_scope);
            at
        };
        let selected = self.evaluator()?.event_scopes(at)?;
        let title = match event.sole_title_key()? {
            Some(key) => self.localised(key, &[], selected, true)?,
            None => String::new(),
        };
        self.localised(
            b"EFFECT_COUNTRY_EVENT",
            &[("EVENT", Argument::Text(&title))],
            scopes,
            true,
        )
    }

    pub(super) fn removed_decision(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = field(fields, "decision")
            .and_then(|value| value.value.read_string())
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or("decision content is unread (14038BF60)")?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let owner = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            _ => return Err("remove_targeted_decision scope has no country (14038BF60)".into()),
        };
        let (country, state) = self.decision_target(field(fields, "target"), scopes)?;
        if country.is_none() && state == 0 {
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let rows = world
            .frame()
            .complete::<tables::Decisions>()
            .map_err(|why| why.to_string())?;
        let Some(row) = matching(
            rows,
            &definition.id,
            owner,
            definition.mission,
            country,
            state,
            |raw| world.recorded().slot(raw),
        )?
        else {
            return Ok(String::new());
        };
        if let Some(why) = &definition.refused {
            return Err(why.clone());
        }
        let title_scope = self.decision_title_scope(definition, row, owner)?;
        let name = self.decision_title(definition, title_scope)?;
        let mut text = self.localised(
            b"EFFECT_REMOVE_TARGETED_DECISION",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )?;
        text.push('\n');
        Ok(text)
    }

    fn decision_target(
        &self,
        target: Option<&Item>,
        scopes: Scopes,
    ) -> Result<(Option<u32>, u32), String> {
        let evaluator = self.evaluator()?;
        if let Some(item) = target {
            let value = item.value.scalar().unwrap_or_default();
            let text = std::str::from_utf8(installed_content::parser::c_string(value))
                .map_err(|_| "decision target is not UTF-8")?;
            // ParseScopeSelector's native event-target fallback includes tooltip
            // targets (1405302E0 / 14052FA10), unlike a trigger country operand.
            let parsed =
                installed_content::scope::parse(text, item.quoted(), self.context.world()?.recorded());
            if matches!(
                parsed,
                installed_content::scope::Scope::Keyword(_) | installed_content::scope::Scope::EventTarget { .. }
            ) {
                return Ok(
                    match evaluator
                        .enter_scope(&parsed, scopes)?
                        .map(|selected| selected.this)
                    {
                        Some(Scope::Country(country) | Scope::CountryLeader { country, .. }) => {
                            (Some(country), 0)
                        }
                        Some(Scope::State(state)) if state != NULL_STATE => (None, state),
                        _ => (None, 0),
                    },
                );
            }
            if let Some(country) = evaluator.scoped_country(value, scopes)? {
                return Ok((Some(country), 0));
            }
            let state = evaluator.scoped_state_id(value, scopes)? as u32;
            if state != 0 {
                return Ok((None, state));
            }
        }
        // 14036C6E0: invalid scoped value falls back to the selected state,
        // then selected country. A recognised missing selector stays invalid.
        Ok(match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => (Some(country), 0),
            Scope::State(state) if state != NULL_STATE => (None, state),
            _ => (None, 0),
        })
    }

    fn decision_name_key(&self, definition: &Decision, country: u32) -> Result<String, String> {
        let world = self.context.world()?;
        let appearance = world
            .frame()
            .complete::<tables::CountryAppearance>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country))
            .ok_or("CountryAppearance lacks decision-name cosmetic tag (140721730)")?;
        let politics = world
            .frame()
            .complete::<tables::Politics>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country))
            .ok_or("Politics lacks decision-name ruling party (140721730)")?;
        let ruling = politics
            .ruling_party
            .ok_or("decision-name ruling party is absent (140721730)")?;
        let party = world
            .frame()
            .complete::<tables::PoliticalParties>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country) && row.id == ruling)
            .ok_or("PoliticalParties lacks decision-name ruling ideology (140721730)")?;
        let utf8 = |bytes: &[u8]| {
            std::str::from_utf8(installed_content::parser::c_string(bytes))
                .map(str::to_owned)
                .map_err(|_| "decision-name key is not UTF-8".to_owned())
        };
        Ok(name_key(
            &utf8(&definition.name)?,
            &utf8(&appearance.cosmetic_tag)?,
            self.tag(country)?,
            &utf8(&party.token)?,
            |key| self.context.localisation.get(key).is_some(),
        ))
    }

    fn decision_title_scope(
        &self,
        definition: &Decision,
        row: &DecisionRow,
        owner: u32,
    ) -> Result<Scopes, String> {
        let country_target = if row.country_target == 0 {
            None
        } else {
            Some(
                self.context
                    .world()?
                    .recorded()
                    .slot(row.country_target)
                    .ok_or("CountryLinks lacks decision title target")?,
            )
        };
        let target = country_target.unwrap_or(row.numeric_target) as i32;
        let seed = self.decision_description_seed(definition, Some(row), owner, target);
        let (at, from) = self.seeded_decision_graph(
            owner,
            Some(match country_target {
                Some(country) => EventObject::Country(country),
                None => EventObject::State(row.numeric_target),
            }),
            seed,
        );
        let scopes = self.evaluator()?.event_scopes(at)?;
        let highlighted = definition.has_state_targets
            || self
                .evaluator()?
                .has_children(&definition.highlight_conditions)?;
        if highlighted && let Some(state) = self.highlight_state(definition, scopes)? {
            let mut graph = self.graph.borrow_mut();
            let index = graph.scopes.len();
            graph.scopes.push(EventScope::of(EventObject::State(state)));
            graph.scopes[from].from = Link::Scope(index);
        }
        self.evaluator()?.event_scopes(at)
    }

    fn decision_description_seed(
        &self,
        definition: &Decision,
        row: Option<&DecisionRow>,
        owner: u32,
        target: i32,
    ) -> Result<u32, &'static str> {
        let game_seed = self
            .context
            .world()
            .map_err(|_| "Clock decision-description seed is unavailable (1407290D0)")?
            .frame()
            .complete::<tables::Clock>()
            .map_err(|_| "Clock decision-description seed is unavailable (1407290D0)")?
            .first()
            .ok_or("Clock lacks decision-description seed (1407290D0)")?
            .seed;
        let additive = if definition.fixed_random_seed {
            None
        } else if let Some(row) = row {
            *row.description_seed.require("Decisions.description_seed")
                .map_err(|_| "Decisions.description_seed not recorded (CDecisionStatus random-count item +0x10; 1407290D0)")?
        } else {
            let world = self
                .context
                .world()
                .map_err(|_| DECISION_RANDOM_COUNT_GAP)?;
            let rows = world
                .frame()
                .complete::<tables::DecisionRandomItems>()
                .map_err(|_| DECISION_RANDOM_COUNT_GAP)?;
            // 140721170 returns the first matching definition/target; a
            // complete roster without a match contributes stored zero.
            Some(
                rows.iter()
                    .filter(|row| {
                        world.slot(row.country) == Some(owner)
                            && row.token.eq_ignore_ascii_case(&definition.id)
                            && row.target == target as u32
                    })
                    .min_by_key(|row| row.ordinal)
                    .map(|row| row.count),
            )
        };
        decision_seed(game_seed, definition, additive, target)
            .map_err(|_| "Decisions description_seed is unread (1407290D0)")
    }

    // 1407290D0 seeds country and FROM from the stored clock/count. A fresh
    // scope's FROM is itself; the second write then mixes its own seed again.
    fn seeded_decision_graph(
        &self,
        owner: u32,
        target: Option<EventObject>,
        seed: Result<u32, &'static str>,
    ) -> (usize, usize) {
        let mut graph = self.graph.borrow_mut();
        let at = graph.scopes.len();
        let mut own = EventScope::of(EventObject::Country(owner));
        let mut from = EventScope::of(target.unwrap_or(EventObject::Country(owner)));
        match seed {
            Ok(seed) => {
                own.random_seed = random::scope_seed(seed);
                own.random_counter = random::scope_counter(seed);
                from.random_seed = random::scope_seed(own.random_seed);
                from.random_counter = random::scope_counter(own.random_seed);
            }
            Err(why) => {
                // No substitute seed is consumed: copies carry this marker to
                // draw sites 14071CB37/14071D055, 1402F70C0 and 140A9CF9A/BD.
                own.random_seed_refusal = Some(why);
                from.random_seed_refusal = Some(why);
            }
        }
        if target.is_some() {
            own.from = Link::Scope(at + 1);
            graph.scopes.extend([own, from]);
            (at, at + 1)
        } else {
            graph.scopes.push(from);
            (at, at)
        }
    }

    fn fresh_country_scope(&self, owner: u32, target: Option<u32>) -> Result<Scopes, String> {
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            let mut scope = EventScope::of(EventObject::Country(owner));
            scope.random_seed = random::scope_seed(0);
            scope.random_counter = random::scope_counter(0);
            if let Some(state) = target {
                scope.from = Link::Scope(at + 1);
                graph.scopes.push(scope);
                graph.scopes.push(EventScope::of(EventObject::State(state)));
            } else {
                graph.scopes.push(scope);
            }
            at
        };
        self.evaluator()?.event_scopes(at)
    }

    fn highlight_state(
        &self,
        definition: &Decision,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        // 14071C840 takes THIS/FROM states before any scripted candidate list.
        if let Scope::State(state) = scopes.this
            && state != NULL_STATE
            && state != 0
        {
            return Ok(Some(state));
        }
        if let Some(Scope::State(state)) = scopes.from
            && state != NULL_STATE
            && state != 0
        {
            return Ok(Some(state));
        }
        let evaluator = self.evaluator()?;
        self.context
            .world()?
            .frame()
            .complete::<tables::States>()
            .map_err(|why| why.to_string())?;
        let owner = match scopes.this {
            Scope::Country(owner) | Scope::CountryLeader { country: owner, .. } => owner,
            _ => return Ok(None),
        };
        let mut candidates = std::collections::BTreeSet::new();
        let root = self.fresh_country_scope(owner, None)?;
        if evaluator
            .evaluate(&definition.target_root_trigger, root)
            .decided()?
        {
            // 140C1BD20 maintains ascending unique state IDs, independently of
            // authored array order, then checks is_valid_target in fresh scopes.
            for array in &definition.target_arrays {
                for object in evaluator.scoped_array_scopes(array, scopes)? {
                    if let Some(Scope::State(state)) = object
                        && state > 0
                        && state != NULL_STATE
                        && self.context.world()?.states.contains_key(&state)
                        && evaluator
                            .evaluate(
                                &definition.target_trigger,
                                self.fresh_country_scope(owner, Some(state))?,
                            )
                            .decided()?
                    {
                        candidates.insert(state);
                    }
                }
            }
            for operand in &definition.target_states {
                let state = evaluator.scoped_state_id(operand, scopes)? as u32;
                if state > 0
                    && state != NULL_STATE
                    && self.context.world()?.states.contains_key(&state)
                    && evaluator
                        .evaluate(
                            &definition.target_trigger,
                            self.fresh_country_scope(owner, Some(state))?,
                        )
                        .decided()?
                {
                    candidates.insert(state);
                }
            }
        }
        if candidates.is_empty() && evaluator.has_children(&definition.highlight_conditions)? {
            for &state in self
                .context
                .world()?
                .states
                .keys()
                .filter(|state| **state > 0)
            {
                let selected = evaluator.with_this(scopes, Scope::State(state))?;
                if evaluator
                    .evaluate(&definition.highlight_conditions, selected)
                    .decided()?
                {
                    candidates.insert(state);
                }
            }
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let graph = self.graph.borrow();
        let scope = &graph.scopes[scopes
            .cursor
            .ok_or("decision highlight lacks scope cursor")?];
        // 14071C840 draws on a copy, incrementing no caller seed word.
        let index = scope.random_draw()? as usize % candidates.len();
        Ok(candidates.into_iter().nth(index))
    }
}

#[cfg(test)]
mod politics_days {
    use super::*;
    use crate::{catalogue::tests::folder_catalogue, effect_description::tests::Fixture};
    use replay::{
        frame::TableView,
        rows::{PoliticalPartyRow, PoliticsRow, TableId, TableRows},
    };

    #[test]
    fn remove_days_uses_caller_title_scope_and_negates_each_branch_without_instance_reads() {
        let mut fixture = Fixture::new();
        let (catalogue, _folder) = folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
            ),
            (
                "localisation/test_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                    " ADD_DAYS_TO_DECISION:0 \"Add $DAYS|H$ to $DECISION|H$ in [Root.GetTag]\"\n",
                    " REMOVE_DAYS_TO_DECISION:0 \"Remove $DAYS|H$ from $DECISION|H$ in [Root.GetTag]\"\n",
                    " SYN_DECISION:0 \"Wrong base\"\n",
                    " SYN_TITLE:0 \"Wrong fallback\"\n",
                    " AAA_SYN_TITLE_group:0 \"Mission [From.GetTag]/[Prev.GetTag]/[Root.GetTag]\"\n"
                ),
            ),
        ]);
        catalogue
            .scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        fixture.catalogue = catalogue;
        *fixture.frame.tables[TableId::Politics.index()] =
            TableView::Complete(TableRows::Politics(vec![PoliticsRow {
                country: 100,
                ruling_party: Some(7),
                ..Default::default()
            }]));
        *fixture.frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 100,
                id: 7,
                token: b"group".to_vec(),
                ..Default::default()
            }]));
        for table in [TableId::Clock, TableId::Decisions] {
            *fixture.frame.tables[table.index()] = TableView::Unavailable {
                reason: "unused decision execution data".into(),
            };
        }
        let mut database = installed_content::decisions::Database::default();
        database.entries.insert(
            b"syn_decision".to_vec(),
            Decision {
                id: b"SYN_DECISION".to_vec(),
                name: b"SYN_TITLE".to_vec(),
                ..Default::default()
            },
        );
        let mut caller = EventScope::of(EventObject::Country(1));
        caller.root = Link::Scope(1);
        caller.from = Link::Scope(2);
        caller.random_seed = 123;
        caller.random_counter = 456;
        let mut scopes = EventScopes::new(vec![
            caller,
            EventScope::of(EventObject::Country(3)),
            EventScope::of(EventObject::Country(2)),
        ]);
        for (days, expected) in [
            (
                "-60",
                "Remove \u{11}H60\u{11}! from \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "60",
                "Add \u{11}H-60\u{11}! to \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "0",
                "Add \u{11}H0\u{11}! to \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "-2147483648",
                "Remove \u{11}H-2147483648\u{11}! from \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
            (
                "-074 days = invalid",
                "Remove \u{11}H60\u{11}! from \u{11}HMission BBB/AAA/CCC\u{11}! in CCC\n",
            ),
        ] {
            // BUL.txt:9289's reached decision/days block, using a synthetic id.
            let output = fixture.render(
                &format!("add_days_remove = {{ decision = SYN_DECISION days = {days} }}"),
                &mut scopes,
                Content {
                    decisions: Some(&database),
                    ..Default::default()
                },
            );
            assert_eq!(output.text, expected, "days={days}");
            assert!(output.refusals.is_empty(), "{:?}", output.refusals);
            assert_eq!(scopes.scopes[0], caller);
            assert_eq!(scopes.scopes.len(), 3);
        }
        let absent = fixture.render(
            "add_days_remove = { decision = ABSENT days = -60 }",
            &mut scopes,
            Content {
                decisions: Some(&database),
                ..Default::default()
            },
        );
        assert_eq!(absent.text, "");
        assert!(absent.refusals.is_empty());
    }
}
