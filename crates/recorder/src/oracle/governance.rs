//! Governance comparisons follow native serializer predicates and full keys.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    io,
};

use installed_content::save::{Save, governance as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, TokenNames, complete, invalid, required_country_tag};

fn equal<T: Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

struct DebugRows<I>(I);

impl<I: Clone + Iterator> Debug for DebugRows<I>
where
    I::Item: Debug,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.0.clone()).finish()
    }
}

fn country<'a>(
    pack: &'a saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    id: u32,
) -> io::Result<(&'a [u8], &'a saved::Country)> {
    let tag = required_country_tag(id, links)?;
    pack.countries
        .get_key_value(tag)
        .map(|(tag, country)| (tag.as_slice(), country))
        .ok_or_else(|| {
            invalid(format!(
                "governance country {} absent from save",
                String::from_utf8_lossy(tag)
            ))
        })
}

fn label(tag: &[u8], field: &str) -> String {
    format!("{}.{}", String::from_utf8_lossy(tag), field)
}

fn literal<'a>(report: &mut Report, subject: &str, token: &'a [u8]) -> &'a [u8] {
    let end = token.iter().position(|&b| b == 0).unwrap_or(token.len());
    if end < token.len() {
        report.limitation("native text serialization ends at the first NUL; captured trailing bytes are not save-comparable", subject);
    }
    &token[..end]
}

fn compare_literal(report: &mut Report, subject: &str, captured: &[u8], saved: &[u8]) {
    let captured = literal(report, subject, captured);
    equal(report, subject, captured, saved);
}

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    compare_pack(frame, &save.governance, links, tokens, report)
}

fn compare_pack(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = true;
    macro_rules! attempt {
        ($name:literal, $compare:ident) => {
            complete &= report.attempt(concat!("governance ", $name), |report| {
                $compare(frame, pack, links, report)
            });
        };
    }
    attempt!("PoliticalParties", compare_parties);
    attempt!("Politics", compare_politics);
    attempt!("Politics.ruling_party", compare_ruling_party);
    attempt!("ActiveIdeas", compare_ideas);
    attempt!("Focus", compare_focus);
    attempt!("CompletedFocuses", compare_completed_focuses);
    attempt!("Research", compare_research);
    if frame.complete::<tables::ResearchModifiers>().is_ok() {
        report.limitation("research modifier operands and sources have synthetic coverage, but no fresh-save or live-rate oracle validation", "ResearchModifiers");
    }
    if frame.complete::<tables::ResearchInputs>().is_ok()
        || frame.complete::<tables::ResearchAppliedBonuses>().is_ok()
    {
        report.limitation("hourly research inputs and applied-bonus attribution have synthetic reader/codec coverage but no fresh-save oracle validation; the existing Research comparison covers only its own staggered tables", "ResearchInputs, ResearchAppliedBonuses");
    }
    attempt!("DecisionCounts", compare_decision_counts);
    attempt!("Decisions", compare_decisions);
    attempt!("DynamicModifiers", compare_modifiers);
    attempt!("PreferredTactics", compare_tactics);
    attempt!("PreferredTacticOperands", compare_tactic_operands);
    complete &= report.attempt("governance TechSharingGroups", |report| {
        compare_sharing_groups(frame, pack, links, tokens, report)
    });
    complete &= report.attempt("governance TechSharingMembers", |report| {
        compare_sharing(frame, pack, links, tokens, report)
    });
    attempt!("PowerBalances", compare_power_balance);
    attempt!("PowerBalanceMembers", compare_balance_members);
    attempt!("PowerBalanceOverrides", compare_balance_overrides);
    attempt!("PowerBalanceRanges", compare_balance_ranges);
    report.limitation(
        "saved country tags do not retain raw reference aliases",
        "governance country references",
    );
    if complete {
        Ok(())
    } else {
        Err(invalid(
            "governance comparisons incomplete; independent results retained",
        ))
    }
}

fn compare_parties(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let parties = complete::<tables::PoliticalParties>(frame)?;
    let mut captured = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut count = 0;
    for row in parties {
        let (tag, country) = country(pack, links, row.country)?;
        let subject = label(tag, "party");
        let token = literal(report, &subject, &row.token);
        if !captured.insert((tag, token)) || !ids.insert((tag, row.id)) {
            return Err(invalid("duplicate political party key or serialized token"));
        }
        match country
            .politics
            .as_ref()
            .and_then(|p| p.parties.get(token).map(|value| (p, value)))
        {
            Some((politics, saved)) => {
                equal(
                    report,
                    format!("{subject}.popularity"),
                    row.popularity,
                    saved.raw,
                );
                equal(
                    report,
                    format!("{subject}.{}.is_ruling", String::from_utf8_lossy(token)),
                    row.is_ruling,
                    u8::from(politics.ruling_party.as_deref() == Some(token)),
                );
                count += 1;
            }
            None => report.difference(subject, token, "missing saved political party"),
        }
    }
    for (tag, country) in &pack.countries {
        if let Some(saved) = &country.politics {
            for token in saved.parties.keys() {
                if !captured.contains(&(tag.as_slice(), token.as_slice())) {
                    report.difference(label(tag, "party"), "missing captured party", token);
                }
            }
        }
    }
    report.compared("PoliticalParties", count);
    report.limitation("party numeric IDs are lexical identifiers; saves compare their exact tokens and ruling membership, not the numeric registry allocation", "PoliticalParties");
    Ok(())
}

fn compare_politics(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let politics = complete::<tables::Politics>(frame)?;
    let mut captured = BTreeSet::new();
    let mut count = 0;
    for row in politics {
        let (tag, country) = country(pack, links, row.country)?;
        if !captured.insert(tag) {
            return Err(invalid("duplicate politics country"));
        }
        let subject = label(tag, "politics");
        let Some(saved) = &country.politics else {
            report.difference(subject, "present", "missing save politics");
            continue;
        };
        equal(
            report,
            format!("{subject}.last_election"),
            Some(row.last_election),
            saved
                .last_election
                .map(|value| value.engine_hour())
                .transpose()?,
        );
        equal(
            report,
            format!("{subject}.election_frequency"),
            Some(row.election_frequency),
            saved.election_frequency,
        );
        equal(
            report,
            format!("{subject}.elections_allowed"),
            Some(row.elections_allowed),
            saved.elections_allowed.map(u8::from),
        );
        equal(
            report,
            format!("{subject}.capital"),
            Some(row.capital),
            country.capital,
        );
        count += 1;
    }
    for (tag, country) in &pack.countries {
        if country.politics.is_some() && !captured.contains(tag.as_slice()) {
            report.difference(
                label(tag, "politics"),
                "missing captured country",
                "present",
            );
        }
    }
    report.compared("Politics", count);
    Ok(())
}

fn compare_ruling_party(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let parties = complete::<tables::PoliticalParties>(frame)?;
    let politics = complete::<tables::Politics>(frame)?;
    let mut ids = BTreeMap::new();
    for row in parties {
        let (tag, _) = country(pack, links, row.country)?;
        let token = literal(report, &label(tag, "party"), &row.token);
        if ids.insert((tag, row.id), token).is_some() {
            return Err(invalid("duplicate political party ID"));
        }
    }
    for row in politics {
        let (tag, country) = country(pack, links, row.country)?;
        let Some(saved) = &country.politics else {
            continue;
        };
        let ruling = row
            .ruling_party
            .map(|id| {
                ids.get(&(tag, id))
                    .copied()
                    .ok_or_else(|| invalid("ruling party ID is absent from party table"))
            })
            .transpose()?;
        equal(
            report,
            label(tag, "politics.ruling_party"),
            ruling,
            saved.ruling_party.as_deref(),
        );
    }
    Ok(())
}

fn compare_ideas(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let ideas = complete::<tables::ActiveIdeas>(frame)?;
    let mut captured: BTreeMap<&[u8], BTreeMap<u32, &[u8]>> = BTreeMap::new();
    for row in ideas {
        let (tag, _) = country(pack, links, row.country)?;
        let token = literal(report, &label(tag, "ideas"), &row.token);
        if captured
            .entry(tag)
            .or_default()
            .insert(row.ordinal, token)
            .is_some()
        {
            return Err(invalid("duplicate active-idea ordinal"));
        }
    }
    let mut count = 0;
    for (tag, country) in &pack.countries {
        if let Some(saved) = &country.politics {
            let actual = captured.remove(tag.as_slice()).unwrap_or_default();
            check_ordinals(actual.keys().copied())?;
            let actual = actual.values().copied().collect::<Vec<_>>();
            let expected = saved.ideas.iter().map(Vec::as_slice).collect::<Vec<_>>();
            equal(
                report,
                label(tag, "ideas.ordered_tokens"),
                &actual,
                &expected,
            );
            count += actual.len().min(expected.len());
        }
    }
    if !captured.is_empty() {
        return Err(invalid("active ideas have no saved politics container"));
    }
    report.compared("ActiveIdeas", count);
    Ok(())
}

fn check_ordinals(values: impl IntoIterator<Item = u32>) -> io::Result<()> {
    for (expected, actual) in values.into_iter().enumerate() {
        if usize::try_from(actual).ok() != Some(expected) {
            return Err(invalid(
                "governance array ordinals are not contiguous from zero",
            ));
        }
    }
    Ok(())
}

fn compare_focus(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::Focus>(frame)?;
    let mut captured = BTreeSet::new();
    let mut count = 0;
    let mut paused_checks = 0_usize;
    let mut focusless_paused_checks = 0_usize;
    for row in rows {
        let (tag, country) = country(pack, links, row.country)?;
        if !captured.insert(tag) {
            return Err(invalid("duplicate focus country"));
        }
        let subject = label(tag, "focus");
        let Some(saved) = &country.focus else {
            report.difference(subject, "present", "missing saved focus");
            continue;
        };
        for (field, actual, expected) in [
            ("tree", row.tree.as_deref(), country.focus_tree.as_deref()),
            ("current", row.current.as_deref(), saved.current.as_deref()),
            (
                "continuous",
                row.continuous.as_deref(),
                saved.continuous.as_deref(),
            ),
        ] {
            let subject = format!("{subject}.{field}");
            let actual = actual.map(|value| literal(report, &subject, value));
            equal(report, subject, actual, expected);
        }
        equal(
            report,
            format!("{subject}.progress"),
            row.progress,
            saved.progress.as_ref().map_or(0, |v| v.raw),
        );
        // 2DC4E0's `paused` write is the function's last statement and is
        // guarded only by `*(char *)(progress + 0xb0) == 0`. It is not nested
        // inside the `current` or `current_continuous` branches, which are
        // separate `if (pointer != 0)` writes earlier in the same body. The
        // comparison therefore applies to every country with a focus-progress
        // object, including the ones that have selected no focus at all.
        equal(
            report,
            format!("{subject}.paused"),
            row.paused == 0,
            saved.paused == Some(false),
        );
        paused_checks += 1;
        if saved.current.is_none() && saved.continuous.is_none() {
            focusless_paused_checks += 1;
        }
        if row.paused > 1 {
            report.limitation("focus serializer omits every nonzero paused byte; it cannot prove its exact nonzero bits", &subject);
        }
        if saved.paused == Some(true) {
            report.difference(
                format!("{subject}.paused"),
                "native writer emits false or omits",
                true,
            );
        }
        equal(
            report,
            format!("{subject}.completed_count"),
            i64::from(row.completed_count),
            saved.completed.len() as i64,
        );
        count += 1;
    }
    for (tag, country) in &pack.countries {
        if country.focus.is_some() && !captured.contains(tag.as_slice()) {
            report.difference(label(tag, "focus"), "missing captured country", "present");
        }
    }
    // The applicability of the paused comparison is a printed result, not an
    // assumption: 2DC4E0 guards that write on the native byte alone, so the
    // focus-less countries below are genuinely compared rather than skipped.
    report.limitation(
        "2DC4E0 writes paused only while the native +0xb0 byte is zero, and that write is not nested inside the selected-focus branches; an omitted paused therefore proves a nonzero byte for every country with a focus-progress object, without proving its exact bits",
        format!(
            "{paused_checks} paused comparisons, of which {focusless_paused_checks} are countries with neither a current nor a continuous focus"
        ),
    );
    report.compared("Focus.paused", paused_checks);
    report.compared("Focus", count);
    Ok(())
}

fn compare_completed_focuses(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let completed = complete::<tables::CompletedFocuses>(frame)?;
    let mut completions: BTreeMap<&[u8], BTreeMap<u32, &[u8]>> = BTreeMap::new();
    for row in completed {
        let (tag, _) = country(pack, links, row.country)?;
        let token = literal(report, &label(tag, "focus.completed"), &row.token);
        if completions
            .entry(tag)
            .or_default()
            .insert(row.ordinal, token)
            .is_some()
        {
            return Err(invalid("duplicate completed-focus ordinal"));
        }
    }
    let mut count = 0;
    for (tag, country) in &pack.countries {
        if let Some(saved) = &country.focus {
            let subject = label(tag, "focus");
            let actual = completions.remove(tag.as_slice()).unwrap_or_default();
            check_ordinals(actual.keys().copied())?;
            let actual = actual.values().copied().collect::<Vec<_>>();
            let expected = saved
                .completed
                .iter()
                .map(|focus| focus.token.as_slice())
                .collect::<Vec<_>>();
            equal(
                report,
                format!("{subject}.completed.ordered_tokens"),
                &actual,
                &expected,
            );
            count += actual.len().min(expected.len());
            if saved
                .completed
                .iter()
                .any(|focus| focus.originator.is_some())
            {
                report.limitation("joint focus originators are serialized separately but are outside the captured completed-focus field set", subject);
            }
        }
    }
    if !completions.is_empty() {
        return Err(invalid("completed focuses have no saved focus container"));
    }
    report.compared("CompletedFocuses", count);
    Ok(())
}

fn compare_research(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    // Tier-4 slot2; cadence table is the single source of truth.
    if !report.refresh_is_current(replay::rows::TableId::ResearchTechnologies, frame.hour) {
        for table in [
            replay::rows::TableId::ResearchTechnologies,
            replay::rows::TableId::ResearchSlots,
            replay::rows::TableId::TechnologyDefinitions,
        ] {
            report.limitation(
                &format!("Research family {}", replay::cadence::STAGGER_LIMITATION),
                format!(
                    "{table:?} refreshes on {} (oracle hour {})",
                    replay::cadence::refresh_text(table),
                    frame.hour
                ),
            );
        }
        return Ok(());
    }
    let definitions = complete::<tables::TechnologyDefinitions>(frame)?;
    let mut dictionary = BTreeMap::new();
    let mut tokens = BTreeSet::new();
    for row in definitions {
        let token = literal(report, "TechnologyDefinitions.id", &row.id);
        if dictionary
            .insert(row.technology_handle, (token, row.max_level))
            .is_some()
            || !tokens.insert(token)
        {
            return Err(invalid(
                "duplicate technology definition handle or serialized token",
            ));
        }
    }
    report.compared("TechnologyDefinitions", 0);
    report.limitation("technology definitions provide exact token joins; runtime handles, database ordinals, year, max level and authored cost are not directly certified by a text save", "TechnologyDefinitions");
    let mut complete = report.attempt("governance ResearchSlots", |report| {
        compare_research_slots(frame, pack, links, &dictionary, report)
    });
    complete &= report.attempt("governance ResearchTechnologies", |report| {
        compare_research_technologies(frame, pack, links, &dictionary, report)
    });
    if complete {
        Ok(())
    } else {
        Err(invalid(
            "research comparisons incomplete; independent results retained",
        ))
    }
}

fn compare_research_slots(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    dictionary: &BTreeMap<u32, (&[u8], i32)>,
    report: &mut Report,
) -> io::Result<()> {
    let slots = complete::<tables::ResearchSlots>(frame)?;
    // Serialized technology, research speed, banked days and applied days.
    type ResearchSlotValues<'a> = (&'a [u8], i64, i64, i64);
    let mut captured_slots: BTreeMap<&[u8], Vec<ResearchSlotValues<'_>>> = BTreeMap::new();
    let mut slot_indices: BTreeMap<&[u8], BTreeSet<u32>> = BTreeMap::new();
    for row in slots {
        let (tag, _) = country(pack, links, row.country)?;
        if row.technology_present > 1 {
            return Err(invalid("invalid research technology presence"));
        }
        if !slot_indices.entry(tag).or_default().insert(row.index) {
            return Err(invalid("duplicate research slot index"));
        }
        let token = if row.technology_handle == 0x2a4e {
            b"empty".as_slice()
        } else {
            dictionary
                .get(&row.technology_handle)
                .ok_or_else(|| invalid("research slot has no definition"))?
                .0
        };
        captured_slots.entry(tag).or_default().push((
            token,
            row.research_speed_raw,
            row.banked_days_raw,
            row.applied_days_raw,
        ));
    }
    let mut slot_count = 0;
    for (tag, country) in &pack.countries {
        if let Some(research) = &country.research {
            let mut actual = captured_slots.remove(tag.as_slice()).unwrap_or_default();
            if let Some(indices) = slot_indices.remove(tag.as_slice()) {
                check_ordinals(indices)?;
            }
            let mut expected = research
                .slots
                .iter()
                .map(|s| {
                    (
                        s.technology.as_slice(),
                        s.points_factor.as_ref().map_or(100_000, |v| v.raw),
                        s.points.as_ref().map_or(0, |v| v.raw),
                        s.used_saved_points.as_ref().map_or(0, |v| v.raw),
                    )
                })
                .collect::<Vec<_>>();
            // ECFF80 does not serialize the native slot index. Compare complete
            // entries with multiplicity, rather than inventing an ordinal join.
            actual.sort_unstable();
            expected.sort_unstable();
            if actual != expected {
                let native = slots
                    .iter()
                    .filter(|row| {
                        required_country_tag(row.country, links)
                            .is_ok_and(|owner| owner == tag.as_slice())
                    })
                    .map(|row| {
                        let token = if row.technology_handle == 0x2a4e {
                            b"empty".as_slice()
                        } else {
                            dictionary[&row.technology_handle].0
                        };
                        (token, row)
                    });
                // Preserve the comparison projection above, while reporting
                // every captured field and the exact saved optional operands.
                report.difference(
                    label(tag, "research_slots.values_with_multiplicity"),
                    DebugRows(native),
                    &research.slots,
                );
            }
            slot_count += actual.len().min(expected.len());
        }
    }
    if !captured_slots.is_empty() {
        return Err(invalid("research slots have no saved technology container"));
    }
    report.compared("ResearchSlots", slot_count);
    report.limitation("research slots serialize technology token and three fixed5 operands, but not native index or technology-pointer presence; complete entry multiplicity is compared", "ResearchSlots");
    Ok(())
}

fn compare_research_technologies(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    dictionary: &BTreeMap<u32, (&[u8], i32)>,
    report: &mut Report,
) -> io::Result<()> {
    let technologies = complete::<tables::ResearchTechnologies>(frame)?;
    // Active-slot omission coverage depends on ResearchSlots, but serialized
    // technology fields and positive-value omission checks do not.
    let active = complete::<tables::ResearchSlots>(frame).and_then(|slots| {
        let mut active = BTreeSet::new();
        for row in slots {
            if row.technology_present > 1 {
                return Err(invalid("invalid research technology presence"));
            }
            let (tag, _) = country(pack, links, row.country)?;
            if row.technology_present != 0 {
                active.insert((tag, row.technology_handle));
            }
        }
        Ok(active)
    });
    let mut captured_technologies = BTreeSet::new();
    let mut technology_count = 0;
    let mut omitted_count = 0;
    for row in technologies {
        let (tag, country) = country(pack, links, row.country)?;
        let (token, max_level) = dictionary
            .get(&row.technology_handle)
            .copied()
            .ok_or_else(|| invalid("research technology has no definition"))?;
        if !captured_technologies.insert((tag, token)) {
            return Err(invalid("duplicate research technology key"));
        }
        let subject = format!(
            "{}.technology.{}",
            String::from_utf8_lossy(tag),
            String::from_utf8_lossy(token)
        );
        let Some(research) = &country.research else {
            report.difference(subject, "captured", "missing saved technology container");
            continue;
        };
        if let Some(saved) = research.technologies.get(token) {
            equal(
                report,
                format!("{subject}.level"),
                row.level,
                saved.level.unwrap_or(0),
            );
            equal(
                report,
                format!("{subject}.research_points"),
                row.progress_raw,
                saved.progress.as_ref().map_or(0, |v| v.raw),
            );
            equal(
                report,
                format!("{subject}.ahead_reduction"),
                row.ahead_reduction_years_raw,
                saved.ahead_reduction.as_ref().map_or(0, |v| v.raw),
            );
            technology_count += 1;
        } else {
            // ED02A0 must emit every positive level/progress and every occupied
            // slot below max level. Its other predicate uses uncaptured bonuses.
            if row.level > 0
                || row.progress_raw > 0
                || (row.level < max_level
                    && active
                        .as_ref()
                        .is_ok_and(|active| active.contains(&(tag, row.technology_handle))))
            {
                report.difference(
                    subject,
                    "serializer-required technology",
                    "missing saved technology",
                );
            }
            omitted_count += 1;
        }
    }
    for (tag, country) in &pack.countries {
        if let Some(research) = &country.research {
            for token in research.technologies.keys() {
                if !captured_technologies.contains(&(tag.as_slice(), token.as_slice())) {
                    report.difference(
                        label(tag, "technology"),
                        "missing captured technology",
                        token,
                    );
                }
            }
        }
    }
    report.compared("ResearchTechnologies", technology_count);
    report.limitation("ED02A0 filters the technology census; omitted records' nonpositive level/progress and ahead-reduction bits are not directly serialized", format!("ResearchTechnologies: {omitted_count} omitted records"));
    report.limitation("technology classifier and independent researched-vector membership are not serialized directly; saved level is not substituted for either", "ResearchTechnologies");
    active.map(|_| ()).map_err(|error| invalid(format!("active-slot omission checks unavailable after serialized technology comparisons: {error}")))
}

fn compare_decision_counts(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let counts = complete::<tables::DecisionCounts>(frame)?;
    let mut captured = BTreeSet::new();
    let mut count = 0;
    for row in counts {
        let (tag, country) = country(pack, links, row.country)?;
        if !captured.insert(tag) {
            return Err(invalid("duplicate decision count country"));
        }
        let Some(saved) = &country.decisions else {
            report.difference(label(tag, "decisions"), "captured", "missing saved status");
            continue;
        };
        let expected = [
            saved.taken.len(),
            saved.containers[0].len(),
            saved.containers[1].len(),
            saved.containers[2].len(),
            saved.containers[3].len(),
            saved.containers[4].len(),
        ];
        let actual = [
            row.taken,
            row.re_enable,
            row.remove,
            row.timed,
            row.targeted,
            row.targeted_timed,
        ]
        .map(|v| v as usize);
        equal(report, label(tag, "decisions.counts"), actual, expected);
        count += 1;
    }
    for (tag, country) in &pack.countries {
        if country.decisions.is_some() && !captured.contains(tag.as_slice()) {
            report.difference(
                label(tag, "decisions"),
                "missing captured counts",
                "present",
            );
        }
    }
    report.compared("DecisionCounts", count);
    Ok(())
}

fn compare_decisions(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::Decisions>(frame)?;
    let mut keys = BTreeSet::new();
    let mut row_count = 0;
    for row in rows {
        let (tag, country) = country(pack, links, row.country)?;
        if row.container > 4 || !keys.insert((tag, row.container, row.ordinal)) {
            return Err(invalid("invalid or duplicate decision source key"));
        }
        let subject = label(tag, &format!("decisions.{}.{}", row.container, row.ordinal));
        let Some(saved) = country
            .decisions
            .as_ref()
            .and_then(|d| d.containers[row.container as usize].get(row.ordinal as usize))
        else {
            report.difference(subject, "captured decision", "missing saved decision");
            continue;
        };
        compare_literal(
            report,
            &format!("{subject}.token"),
            &row.token,
            &saved.token,
        );
        equal(report, format!("{subject}.days"), row.days, saved.days);
        if let Some(state) = saved.lifecycle {
            equal(report, format!("{subject}.lifecycle"), row.lifecycle, state);
        }
        if let Some(target) = &saved.target {
            let native_country = (row.country_target as i32) > 0;
            equal(
                report,
                format!("{subject}.target_kind"),
                row.target_kind,
                if native_country { 1 } else { 2 },
            );
            match target {
                saved::Target::Country(tag) => {
                    if native_country {
                        equal(
                            report,
                            format!("{subject}.target_country"),
                            required_country_tag(row.country_target, links)?,
                            tag.as_slice(),
                        );
                    } else {
                        report.difference(format!("{subject}.target"), row.numeric_target, tag);
                    }
                }
                saved::Target::Numeric(value) => {
                    if native_country {
                        report.difference(format!("{subject}.target"), "country target", value);
                    } else {
                        equal(
                            report,
                            format!("{subject}.target_numeric"),
                            row.numeric_target,
                            *value,
                        );
                    }
                }
            }
            equal(
                report,
                format!("{subject}.ignore"),
                row.ignore,
                u8::from(saved.ignore.unwrap()),
            );
        }
        row_count += 1;
    }
    for (tag, country) in &pack.countries {
        if let Some(saved) = &country.decisions {
            for (container, rows) in saved.containers.iter().enumerate() {
                for ordinal in 0..rows.len() {
                    if !keys.contains(&(tag.as_slice(), container as u8, ordinal as u32)) {
                        report.difference(
                            label(tag, &format!("decisions.{container}.{ordinal}")),
                            "missing captured decision",
                            "present",
                        );
                    }
                }
            }
        }
    }
    report.compared("Decisions", row_count);
    report.limitation("target serializer selects the positive signed country reference or the numeric word; the unused target word is not save-comparable; cooldown rows do not serialize lifecycle/target/ignore", "Decisions");
    Ok(())
}

fn compare_modifiers(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DynamicModifiers>(frame)?;
    let mut keys = BTreeSet::new();
    let mut count = 0;
    for row in rows {
        let (tag, country) = country(pack, links, row.country)?;
        if !keys.insert((tag, row.ordinal)) {
            return Err(invalid("duplicate dynamic modifier ordinal"));
        }
        let subject = label(tag, &format!("dynamic_modifiers.{}", row.ordinal));
        let Some(saved) = country.dynamic_modifiers.get(row.ordinal as usize) else {
            report.difference(subject, "captured modifier", "missing saved modifier");
            continue;
        };
        compare_literal(
            report,
            &format!("{subject}.token"),
            &row.token,
            &saved.token,
        );
        equal(
            report,
            format!("{subject}.enabled"),
            row.enabled,
            u8::from(saved.enabled),
        );
        equal(
            report,
            format!("{subject}.values"),
            row.values.as_slice(),
            saved
                .values
                .iter()
                .map(|v| v.raw)
                .collect::<Vec<_>>()
                .as_slice(),
        );
        if let Some(state) = saved.scope_state {
            equal(report, format!("{subject}.state"), row.scope_state, state);
        } else if row.scope_state > 0 {
            report.difference(
                format!("{subject}.state"),
                row.scope_state,
                "missing positive state",
            );
        }
        if let Some(country) = &saved.scope_country {
            if (row.scope_country as i32) > 0 {
                equal(
                    report,
                    format!("{subject}.country"),
                    required_country_tag(row.scope_country, links)?,
                    country.as_slice(),
                );
            } else {
                report.difference(format!("{subject}.country"), row.scope_country, country);
            }
        } else if (row.scope_country as i32) > 0 {
            report.difference(
                format!("{subject}.country"),
                row.scope_country,
                "missing positive country",
            );
        }
        if let Some(days) = saved.days {
            equal(report, format!("{subject}.days"), row.days_remaining, days);
        } else if row.days_remaining >= 0 {
            report.difference(
                format!("{subject}.days"),
                row.days_remaining,
                "missing nonnegative days",
            );
        }
        count += 1;
    }
    for (tag, country) in &pack.countries {
        for ordinal in 0..country.dynamic_modifiers.len() {
            if !keys.contains(&(tag.as_slice(), ordinal as u32)) {
                report.difference(
                    label(tag, &format!("dynamic_modifiers.{ordinal}")),
                    "missing captured modifier",
                    "present",
                );
            }
        }
    }
    report.compared("DynamicModifiers", count);
    report.limitation("dynamic-modifier save omission proves days<0 or scope<=0, but cannot distinguish the native omitted values; full raw bits remain captured", "DynamicModifiers");
    Ok(())
}

fn compare_tactics(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let tactics = complete::<tables::PreferredTactics>(frame)?;
    let mut tactic_countries = BTreeSet::new();
    let mut tactic_count = 0;
    for row in tactics {
        let (tag, country) = country(pack, links, row.country)?;
        if !tactic_countries.insert(tag) {
            return Err(invalid("duplicate preferred tactic country"));
        }
        equal(
            report,
            label(tag, "preferred_tactic.id"),
            Some(row.tactic_id),
            country.preferred_tactic,
        );
        if country.preferred_tactic.is_some() {
            tactic_count += 1;
        }
    }
    for (tag, country) in &pack.countries {
        if country.preferred_tactic.is_some() && !tactic_countries.contains(tag.as_slice()) {
            report.difference(
                label(tag, "preferred_tactic"),
                "missing captured tactic",
                country.preferred_tactic,
            );
        }
    }
    report.compared("PreferredTactics", tactic_count);
    report.limitation("preferred_tactic saves the signed native tactic ID; the definition token is not serialized", "PreferredTactics");
    Ok(())
}

fn compare_tactic_operands(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let operands = complete::<tables::PreferredTacticOperands>(frame)?;
    for row in operands {
        country(pack, links, row.country)?;
    }
    report.compared("PreferredTacticOperands", 0);
    report.limitation("preferred-tactic modifier key, metadata, factor, accumulator value and entry-presence are runtime operands without a direct save representation", format!("PreferredTacticOperands: {} rows", operands.len()));
    Ok(())
}

fn compare_sharing_groups(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    names: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let groups = complete::<tables::TechSharingGroups>(frame)?;
    let mut saved_groups = BTreeMap::new();
    for group in &pack.tech_sharing {
        if saved_groups.insert(group.token.as_slice(), group).is_some() {
            return Err(invalid("duplicate saved technology-sharing ID"));
        }
    }
    equal(
        report,
        "TechSharingGroups.count",
        groups.len(),
        saved_groups.len(),
    );
    let mut captured_groups = BTreeMap::new();
    let mut matched = BTreeSet::new();
    let mut group_count = 0;
    let mut unresolved_groups = 0;
    for row in groups {
        if captured_groups.insert(row.id, row).is_some() {
            return Err(invalid("duplicate captured technology-sharing ID"));
        }
        let Some(token) = names.get(&row.id) else {
            unresolved_groups += 1;
            report.limitation(
                "technology-sharing group token is absent from the recorded lexical dictionary; group identity, per-group saved member count and ordered member joins cannot be compared",
                format!("TechSharingGroups.{}.id; TechSharingMembers.{}.group/ordinal", row.id, row.id),
            );
            continue;
        };
        if token.contains(&0) {
            unresolved_groups += 1;
            report.limitation(
                "technology-sharing token contains NUL and its concrete serializer target is unproved; group identity, saved member count and ordered member joins have unavailable mapping",
                format!("TechSharingGroups.{}.id; TechSharingMembers.{}.group/ordinal", row.id, row.id),
            );
            continue;
        }
        let token = *token;
        if !matched.insert(token) {
            return Err(invalid(
                "duplicate captured technology-sharing serialized token",
            ));
        }
        match saved_groups.get(token) {
            Some(saved) => {
                equal(
                    report,
                    "TechSharingGroups.member_count",
                    row.member_count as usize,
                    saved.countries.len(),
                );
                group_count += 1;
            }
            None => report.difference("TechSharingGroups", token, "missing saved group"),
        }
    }
    if unresolved_groups == 0 {
        for token in saved_groups.keys() {
            if !matched.contains(token) {
                report.difference("TechSharingGroups", "missing captured group", token);
            }
        }
    }
    report.compared("TechSharingGroups", group_count);
    report.limitation("technology-sharing saves exact group tokens and ordered member bonuses; base bonus and numeric lexical ID are not serialized", "TechSharingGroups");
    Ok(())
}

fn compare_sharing(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let groups = complete::<tables::TechSharingGroups>(frame)?;
    let members = complete::<tables::TechSharingMembers>(frame)?;
    let mut saved_groups = BTreeMap::new();
    for group in &pack.tech_sharing {
        if saved_groups.insert(group.token.as_slice(), group).is_some() {
            return Err(invalid("duplicate saved technology-sharing ID"));
        }
    }
    let mut captured_groups = BTreeMap::new();
    let mut resolved_groups = BTreeMap::new();
    for row in groups {
        if captured_groups.insert(row.id, row).is_some() {
            return Err(invalid("duplicate captured technology-sharing ID"));
        }
        if let Some(&token) = names.get(&row.id).filter(|token| !token.contains(&0))
            && let Some(saved) = saved_groups.get(token)
        {
            resolved_groups.insert(row.id, *saved);
        }
    }
    equal(
        report,
        "TechSharingMembers.count",
        members.len(),
        pack.tech_sharing
            .iter()
            .map(|group| group.countries.len())
            .sum::<usize>(),
    );
    let mut keys = BTreeSet::new();
    let mut ordinals: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut actual_members = Vec::with_capacity(members.len());
    let mut member_count = 0;
    for row in members {
        if !captured_groups.contains_key(&row.group) {
            return Err(invalid("technology-sharing member has no group"));
        }
        if !keys.insert((row.group, row.ordinal)) {
            return Err(invalid("duplicate technology-sharing member ordinal"));
        }
        ordinals.entry(row.group).or_default().insert(row.ordinal);
        let country = required_country_tag(row.country, links)?;
        actual_members.push((country, row.bonus));
        let Some(group) = resolved_groups.get(&row.group) else {
            continue;
        };
        let Some(saved) = group
            .countries
            .get(row.ordinal as usize)
            .zip(group.bonuses.get(row.ordinal as usize))
        else {
            report.difference(
                "TechSharingMembers",
                (row.group, row.ordinal),
                "missing saved member",
            );
            continue;
        };
        equal(
            report,
            "TechSharingMembers.country",
            country,
            saved.0.as_slice(),
        );
        equal(report, "TechSharingMembers.bonus", row.bonus, saved.1.raw);
        member_count += 1;
    }
    for (&id, row) in &captured_groups {
        let ordinals = ordinals.remove(&id).unwrap_or_default();
        equal(
            report,
            format!("TechSharingGroups.{id}.captured_member_count"),
            ordinals.len(),
            row.member_count as usize,
        );
        check_ordinals(ordinals)?;
    }
    for (&id, group) in &resolved_groups {
        for ordinal in 0..group.countries.len() {
            if !keys.contains(&(id, ordinal as u32)) {
                report.difference(
                    "TechSharingMembers",
                    "missing captured member",
                    (&group.token, ordinal),
                );
            }
        }
    }
    // Project the complete native and saved member censuses directly. This
    // checks the country/bonus pairs without inventing unknown group joins.
    let mut expected_members = pack
        .tech_sharing
        .iter()
        .flat_map(|group| {
            group
                .countries
                .iter()
                .zip(&group.bonuses)
                .map(|(country, bonus)| (country.as_slice(), bonus.raw))
        })
        .collect::<Vec<_>>();
    actual_members.sort_unstable();
    expected_members.sort_unstable();
    equal(
        report,
        "TechSharingMembers.country_bonus_multiset",
        actual_members,
        expected_members,
    );
    report.compared("TechSharingMembers", member_count);
    Ok(())
}

fn compare_power_balance(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::PowerBalances>(frame)?;
    let Some(saved) = &pack.power_balances else {
        if !rows.is_empty() {
            report.difference("PowerBalances", "captured state", "missing save manager");
        }
        report.compared("PowerBalances", 0);
        return Ok(());
    };
    let mut records = BTreeSet::new();
    let mut count = 0;
    for row in rows {
        if !records.insert(row.ordinal) {
            return Err(invalid("duplicate power-balance ordinal"));
        }
        let Some(saved) = saved.get(row.ordinal as usize) else {
            report.difference("PowerBalances", row.ordinal, "missing saved record");
            continue;
        };
        equal(
            report,
            "PowerBalances.value",
            row.value_raw,
            saved.value.raw,
        );
        for (name, actual, expected) in [
            ("template", &row.balance, &saved.template),
            ("left", &row.left, &saved.left),
            ("right", &row.right, &saved.right),
            ("trending", &row.trending, &saved.trending),
        ] {
            compare_literal(
                report,
                &format!("PowerBalances.{}.{name}", row.ordinal),
                actual,
                expected,
            );
        }
        count += 1;
    }
    for ordinal in 0..saved.len() {
        if !records.contains(&(ordinal as u32)) {
            report.difference("PowerBalances", "missing captured record", ordinal);
        }
    }
    report.compared("PowerBalances", count);
    Ok(())
}

fn compare_balance_members(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let members = complete::<tables::PowerBalanceMembers>(frame)?;
    let Some(saved) = &pack.power_balances else {
        if !members.is_empty() {
            report.difference(
                "PowerBalanceMembers",
                "captured state",
                "missing save manager",
            );
        }
        report.compared("PowerBalanceMembers", 0);
        return Ok(());
    };
    let mut member_keys = BTreeSet::new();
    let mut member_count = 0;
    for row in members {
        if !member_keys.insert((row.record, row.ordinal)) {
            return Err(invalid("orphan or duplicate power-balance member"));
        }
        let Some(tag) = saved
            .get(row.record as usize)
            .and_then(|r| r.countries.get(row.ordinal as usize))
        else {
            report.difference(
                "PowerBalanceMembers",
                (row.record, row.ordinal),
                "missing saved member",
            );
            continue;
        };
        equal(
            report,
            "PowerBalanceMembers.country",
            required_country_tag(row.country, links)?,
            tag.as_slice(),
        );
        member_count += 1;
    }
    for (record, saved) in saved.iter().enumerate() {
        for ordinal in 0..saved.countries.len() {
            if !member_keys.contains(&(record as u32, ordinal as u32)) {
                report.difference(
                    "PowerBalanceMembers",
                    "missing captured member",
                    (record, ordinal),
                );
            }
        }
    }
    report.compared("PowerBalanceMembers", member_count);
    report.limitation("power-balance member daily/weekly accumulator operands, presence and metadata flags are not serialized by the balance writer", "PowerBalanceMembers");
    Ok(())
}

fn compare_balance_overrides(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let overrides = complete::<tables::PowerBalanceOverrides>(frame)?;
    let Some(saved) = &pack.power_balances else {
        if !overrides.is_empty() {
            report.difference(
                "PowerBalanceOverrides",
                "captured state",
                "missing save manager",
            );
        }
        report.compared("PowerBalanceOverrides", 0);
        return Ok(());
    };
    let mut override_keys = BTreeSet::new();
    let mut override_count = 0;
    for row in overrides {
        if !override_keys.insert((row.record, row.ordinal)) {
            return Err(invalid("orphan or duplicate power-balance override"));
        }
        let Some(saved) = saved
            .get(row.record as usize)
            .and_then(|r| r.overrides.get(row.ordinal as usize))
        else {
            report.difference(
                "PowerBalanceOverrides",
                (row.record, row.ordinal),
                "missing saved override",
            );
            continue;
        };
        compare_literal(report, "PowerBalanceOverrides.side", &row.side, &saved.side);
        compare_literal(report, "PowerBalanceOverrides.gfx", &row.gfx, &saved.gfx);
        override_count += 1;
    }
    for (record, saved) in saved.iter().enumerate() {
        for ordinal in 0..saved.overrides.len() {
            if !override_keys.contains(&(record as u32, ordinal as u32)) {
                report.difference(
                    "PowerBalanceOverrides",
                    "missing captured override",
                    (record, ordinal),
                );
            }
        }
    }
    report.compared("PowerBalanceOverrides", override_count);
    Ok(())
}

fn compare_balance_ranges(
    frame: &DecodedFrame,
    _pack: &saved::Pack,
    _links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let ranges = complete::<tables::PowerBalanceRanges>(frame)?;
    let records = complete::<tables::PowerBalances>(frame)?
        .iter()
        .map(|row| row.ordinal)
        .collect::<BTreeSet<_>>();
    let mut keys = BTreeSet::new();
    for row in ranges {
        if !records.contains(&row.record) || !keys.insert((row.record, row.ordinal)) {
            return Err(invalid("orphan or duplicate power-balance range"));
        }
    }
    report.compared("PowerBalanceRanges", 0);
    report.limitation("power-balance active-range cache and range vector are not serialized; side tokens are not substitutes for range identity", format!("PowerBalanceRanges: {} rows", ranges.len()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            ActiveIdeaRow, CompletedFocusRow, DecisionCountsRow, DecisionRow, DynamicModifierRow,
            LexicalTokenRow, PreferredTacticRow, ResearchSlotRow, ResearchTechnologyRow, TableId,
            TableRows, TechSharingGroupRow, TechSharingMemberRow, TechnologyDefinitionRow,
        },
    };

    fn compare_sharing_fixture(
        frame: &DecodedFrame,
        pack: &saved::Pack,
        links: &BTreeMap<u32, &CountryRow>,
        report: &mut Report,
    ) -> io::Result<()> {
        let tokens = super::super::lexical_tokens(frame, report)?;
        compare_sharing_groups(frame, pack, links, &tokens, report)?;
        compare_sharing(frame, pack, links, &tokens, report)
    }

    fn fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
        let country = CountryRow {
            id: 11,
            canonical_slot: 7,
            tag: b"AAA".to_vec(),
            name: Vec::new(),
            political_power_raw: None,
            command_power_raw: 0,
            stability_raw: 0,
            war_support_raw: 0,
            convoys_raw: None,
        };
        let frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            }),
        };
        let pack = saved::Pack {
            countries: BTreeMap::from([(b"AAA".to_vec(), saved::Country::default())]),
            ..Default::default()
        };
        (frame, pack, country)
    }

    #[test]
    fn unavailable_siblings_retain_independent_comparisons_and_aggregate_failure() {
        let (mut frame, _, country) = fixture();
        let links = BTreeMap::from([(11, &country)]);
        let document = installed_content::parser::parse(
            br#"countries={AAA={
            politics={ideas={idea}} focus={completed=focus}
            preferred_tactic=7 technology={technologies={technology={research_points=0.00002}}}
        }}"#,
        )
        .unwrap();
        let pack = saved::parse(&document.items).unwrap();
        frame.tables[TableId::ActiveIdeas.index()] =
            TableView::Complete(TableRows::ActiveIdeas(vec![ActiveIdeaRow {
                country: 11,
                ordinal: 0,
                token: b"different_idea".to_vec(),
            }]));
        frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
                country: 11,
                ordinal: 0,
                token: b"focus".to_vec(),
            }]));
        frame.tables[TableId::PreferredTactics.index()] =
            TableView::Complete(TableRows::PreferredTactics(vec![PreferredTacticRow {
                country: 11,
                tactic_id: 8,
                token: Vec::new(),
            }]));
        frame.tables[TableId::TechnologyDefinitions.index()] =
            TableView::Complete(TableRows::TechnologyDefinitions(vec![
                TechnologyDefinitionRow {
                    technology_handle: 5,
                    id: b"technology".to_vec(),
                    ..Default::default()
                },
            ]));
        frame.tables[TableId::ResearchTechnologies.index()] =
            TableView::Complete(TableRows::ResearchTechnologies(vec![
                ResearchTechnologyRow {
                    country: 11,
                    technology_handle: 5,
                    progress_raw: 1,
                    ..Default::default()
                },
            ]));
        for table in [
            TableId::PoliticalParties,
            TableId::Focus,
            TableId::ResearchSlots,
            TableId::DecisionCounts,
            TableId::DynamicModifiers,
            TableId::TechSharingMembers,
            TableId::PowerBalanceMembers,
        ] {
            frame.tables[table.index()] = TableView::Unavailable {
                reason: "injected sibling fault".into(),
            };
        }
        let mut report = Report::default();
        assert!(compare_pack(&frame, &pack, &links, &BTreeMap::new(), &mut report).is_err());
        for table in [
            "ActiveIdeas",
            "CompletedFocuses",
            "PreferredTactics",
            "ResearchTechnologies",
        ] {
            assert_eq!(report.compared_tables.get(table), Some(&1), "{table}");
        }
        for table in [
            "Decisions",
            "PreferredTacticOperands",
            "TechSharingGroups",
            "PowerBalanceOverrides",
            "PowerBalanceRanges",
        ] {
            assert!(report.compared_tables.contains_key(table), "{table}");
        }
        for subject in [
            "ideas.ordered_tokens",
            "preferred_tactic.id",
            "technology.technology.research_points",
        ] {
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains(subject)),
                "{subject}"
            );
        }
        assert!(
            report
                .failed_comparisons
                .contains_key("governance PoliticalParties")
        );
        assert!(
            report
                .failed_comparisons
                .contains_key("governance ResearchSlots")
        );
        assert!(
            report
                .failed_comparisons
                .contains_key("governance PowerBalanceMembers")
        );
    }

    #[test]
    fn idle_research_slot_is_not_equivalent_to_a_new_assignment() {
        let (mut frame, _, country) = fixture();
        let links = BTreeMap::from([(11, &country)]);
        let document = installed_content::parser::parse(
            br#"countries={AAA={technology={
            slots={selected_technology={points_factor=1.35}}
        }}}"#,
        )
        .unwrap();
        let pack = saved::parse(&document.items).unwrap();
        let dictionary = BTreeMap::from([(5, (b"selected_technology".as_slice(), 1))]);
        frame.tables[TableId::ResearchSlots.index()] =
            TableView::Complete(TableRows::ResearchSlots(vec![ResearchSlotRow {
                country: 11,
                index: 0,
                technology_handle: 0x2a4e,
                technology_present: 0,
                research_speed_raw: 100_000,
                banked_days_raw: 0,
                applied_days_raw: 0,
            }]));
        let mut report = Report::default();
        compare_research_slots(&frame, &pack, &links, &dictionary, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        for field in [
            "country: 11",
            "index: 0",
            "technology_handle: 10830",
            "technology_present: 0",
            "research_speed_raw: 100000",
            "banked_days_raw: 0",
            "applied_days_raw: 0",
            "points: None",
            "used_saved_points: None",
            "points_factor: Some(Decimal { raw: 135000",
        ] {
            assert!(report.differences[0].contains(field), "{field}");
        }
        let TableView::Complete(TableRows::ResearchSlots(rows)) =
            &mut frame.tables[TableId::ResearchSlots.index()]
        else {
            unreachable!()
        };
        rows[0].technology_handle = 5;
        rows[0].technology_present = 1;
        rows[0].research_speed_raw = 135_000;
        let mut report = Report::default();
        compare_research_slots(&frame, &pack, &links, &dictionary, &mut report).unwrap();
        assert!(report.differences.is_empty());
    }

    fn sharing_fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
        let (mut frame, mut pack, country) = fixture();
        pack.tech_sharing = [b"first".as_slice(), b"second"]
            .into_iter()
            .enumerate()
            .map(|(index, token)| saved::TechSharing {
                token: token.to_vec(),
                countries: vec![b"AAA".to_vec()],
                bonuses: vec![
                    installed_content::save::Decimal::parse(if index == 0 {
                        b"0.00001"
                    } else {
                        b"0.00002"
                    })
                    .unwrap(),
                ],
            })
            .collect();
        frame.tables[TableId::TechSharingGroups.index()] =
            TableView::Complete(TableRows::TechSharingGroups(vec![
                TechSharingGroupRow {
                    id: 0x10001,
                    member_count: 1,
                    base_bonus: -7,
                },
                TechSharingGroupRow {
                    id: 0x20001,
                    member_count: 1,
                    base_bonus: 8,
                },
            ]));
        frame.tables[TableId::TechSharingMembers.index()] =
            TableView::Complete(TableRows::TechSharingMembers(vec![
                TechSharingMemberRow {
                    group: 0x10001,
                    ordinal: 0,
                    country: 11,
                    bonus: 1,
                },
                TechSharingMemberRow {
                    group: 0x20001,
                    ordinal: 0,
                    country: 11,
                    bonus: 2,
                },
            ]));
        (frame, pack, country)
    }

    #[test]
    fn sharing_complete_names_preserve_order_exact_bonuses_and_empty_groups() {
        let (mut frame, _, country) = fixture();
        let second = CountryRow {
            id: 12,
            canonical_slot: 8,
            tag: b"BBB".to_vec(),
            ..country.clone()
        };
        let links = BTreeMap::from([(11, &country), (12, &second)]);
        let document = installed_content::parser::parse(
            br#"countries={AAA={} BBB={}}
            tech_sharing_group={id=first countries={AAA BBB} bonuses={-21474.83649 0}}
            tech_sharing_group={id=second}
        "#,
        )
        .unwrap();
        let pack = saved::parse(&document.items).unwrap();
        frame.tables[TableId::TechSharingGroups.index()] =
            TableView::Complete(TableRows::TechSharingGroups(vec![
                TechSharingGroupRow {
                    id: 0x10001,
                    member_count: 2,
                    base_bonus: -123,
                },
                TechSharingGroupRow {
                    id: 0x20001,
                    member_count: 0,
                    base_bonus: 456,
                },
            ]));
        frame.tables[TableId::TechSharingMembers.index()] =
            TableView::Complete(TableRows::TechSharingMembers(vec![
                TechSharingMemberRow {
                    group: 0x10001,
                    ordinal: 0,
                    country: 11,
                    bonus: -2_147_483_649,
                },
                TechSharingMemberRow {
                    group: 0x10001,
                    ordinal: 1,
                    country: 12,
                    bonus: 0,
                },
            ]));
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 0x10001,
                    literal: b"first".to_vec(),
                },
                LexicalTokenRow {
                    token: 0x20001,
                    literal: b"second".to_vec(),
                },
            ]));
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables.get("TechSharingGroups"), Some(&2));
        assert_eq!(report.compared_tables.get("TechSharingMembers"), Some(&2));
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("group token is absent"))
        );

        let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut frame.tables[TableId::LexicalTokens.index()]
        else {
            unreachable!()
        };
        rows[0].literal.extend_from_slice(b"\0unserialized_suffix");
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("TechSharingGroups"), Some(&1));
        assert_eq!(report.compared_tables.get("TechSharingMembers"), Some(&0));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("NUL") && reason.contains("unavailable mapping"))
        );
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("first NUL"))
        );

        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows[0].bonus += 1;
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("country_bonus_multiset"));
        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows[0].bonus -= 1;
        let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut frame.tables[TableId::LexicalTokens.index()]
        else {
            unreachable!()
        };
        rows[0].literal = b"first".to_vec();

        // The complete multiset is unchanged, but the saved pair order differs.
        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows.swap(0, 1);
        rows[0].ordinal = 0;
        rows[1].ordinal = 1;
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(
            report
                .differences
                .iter()
                .filter(|difference| difference.contains("TechSharingMembers.country:"))
                .count(),
            2
        );
        assert_eq!(
            report
                .differences
                .iter()
                .filter(|difference| difference.contains("TechSharingMembers.bonus:"))
                .count(),
            2
        );
        assert!(
            !report
                .differences
                .iter()
                .any(|difference| difference.contains("country_bonus_multiset"))
        );

        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows.swap(0, 1);
        rows[0].ordinal = 0;
        rows[1].ordinal = 1;
        rows[0].bonus += 1;
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(
            report
                .differences
                .iter()
                .filter(|difference| difference.contains("TechSharingMembers.bonus:"))
                .count(),
            1
        );
    }

    #[test]
    fn sharing_without_dictionary_checks_values_and_native_census() {
        let (mut frame, pack, country) = sharing_fixture();
        let links = BTreeMap::from([(11, &country)]);
        frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "LexicalTokensUnavailable: writer scheduling unproved".into(),
        };
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("group token is absent"))
        );

        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows[0].bonus += 1;
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("country_bonus_multiset"));

        let TableView::Complete(TableRows::TechSharingGroups(rows)) =
            &mut frame.tables[TableId::TechSharingGroups.index()]
        else {
            unreachable!()
        };
        rows[0].member_count += 1;
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("captured_member_count"))
        );

        frame.tables[TableId::TechSharingMembers.index()] = TableView::Unavailable {
            reason: "member read fault".into(),
        };
        assert!(compare_sharing_fixture(&frame, &pack, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn sharing_partial_names_do_not_fabricate_missing_groups_or_hide_known_group_differences() {
        let (mut frame, pack, country) = sharing_fixture();
        let links = BTreeMap::from([(11, &country)]);
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 0x10001,
                literal: b"first".to_vec(),
            }]));
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables.get("TechSharingGroups"), Some(&1));
        assert_eq!(report.compared_tables.get("TechSharingMembers"), Some(&1));

        // Preserve the aggregate pairs while moving them between native groups.
        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows[0].bonus = 2;
        rows[1].bonus = 1;
        let mut report = Report::default();
        compare_sharing_fixture(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("TechSharingMembers.bonus"));
    }

    #[test]
    fn sharing_unresolved_names_still_reject_orphans_and_gapped_ordinals() {
        let (mut frame, pack, country) = sharing_fixture();
        let links = BTreeMap::from([(11, &country)]);
        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows[0].ordinal = 1;
        assert!(compare_sharing_fixture(&frame, &pack, &links, &mut Report::default()).is_err());
        let TableView::Complete(TableRows::TechSharingMembers(rows)) =
            &mut frame.tables[TableId::TechSharingMembers.index()]
        else {
            unreachable!()
        };
        rows[0].ordinal = 0;
        rows[0].group = 1;
        assert!(compare_sharing_fixture(&frame, &pack, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn decisions_compare_signed_target_bits_and_detect_missing_source_rows() {
        let (mut frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(11, &country), (12, &country)]);
        let mut containers: [Vec<saved::Decision>; 5] = Default::default();
        containers[3].push(saved::Decision {
            token: b"decision".to_vec(),
            days: -4,
            lifecycle: Some(3),
            target: Some(saved::Target::Numeric(0x8000_0000)),
            ignore: Some(true),
        });
        pack.countries.get_mut(b"AAA".as_slice()).unwrap().decisions = Some(saved::Decisions {
            taken: Vec::new(),
            containers,
        });
        frame.tables[TableId::DecisionCounts.index()] =
            TableView::Complete(TableRows::DecisionCounts(vec![DecisionCountsRow {
                country: 11,
                targeted: 1,
                ..Default::default()
            }]));
        frame.tables[TableId::Decisions.index()] =
            TableView::Complete(TableRows::Decisions(vec![DecisionRow {
                country: 12,
                container: 3,
                ordinal: 0,
                token: b"decision".to_vec(),
                days: -4,
                lifecycle: 3,
                target_kind: 2,
                country_target: u32::MAX,
                numeric_target: 0x8000_0000,
                ignore: 1,
            }]));
        let mut report = Report::default();
        compare_decisions(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let TableView::Complete(TableRows::Decisions(rows)) =
            &mut frame.tables[TableId::Decisions.index()]
        else {
            unreachable!()
        };
        rows[0].numeric_target += 1;
        let mut report = Report::default();
        compare_decisions(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        frame.tables[TableId::Decisions.index()] =
            TableView::Complete(TableRows::Decisions(Vec::new()));
        let mut report = Report::default();
        compare_decisions(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        frame.tables[TableId::Decisions.index()] = TableView::Unavailable {
            reason: "fault".into(),
        };
        assert!(compare_decisions(&frame, &pack, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn modifier_omissions_check_predicates_without_fabricating_negative_defaults() {
        let (mut frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(11, &country)]);
        let saved = saved::DynamicModifier {
            token: b"modifier".to_vec(),
            scope_country: None,
            scope_state: None,
            days: None,
            enabled: false,
            values: vec![installed_content::save::Decimal::parse(b"-21474.83649").unwrap()],
        };
        pack.countries
            .get_mut(b"AAA".as_slice())
            .unwrap()
            .dynamic_modifiers
            .push(saved);
        frame.tables[TableId::DynamicModifiers.index()] =
            TableView::Complete(TableRows::DynamicModifiers(vec![DynamicModifierRow {
                country: 11,
                ordinal: 0,
                scope_country: u32::MAX,
                scope_state: -7,
                days_remaining: -2,
                enabled: 0,
                token: b"modifier".to_vec(),
                values: vec![-2_147_483_649],
            }]));
        let mut report = Report::default();
        compare_modifiers(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(!report.limitations.is_empty());
        let TableView::Complete(TableRows::DynamicModifiers(rows)) =
            &mut frame.tables[TableId::DynamicModifiers.index()]
        else {
            unreachable!()
        };
        rows[0].days_remaining = 0;
        let mut report = Report::default();
        compare_modifiers(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn ordered_rows_cannot_skip_or_duplicate_native_positions() {
        assert!(check_ordinals([0, 1, 2]).is_ok());
        assert!(check_ordinals([0, 2]).is_err());
        assert!(check_ordinals([0, 0]).is_err());
        assert!(check_ordinals([1]).is_err());
    }

    #[test]
    fn one_native_unit_and_literal_suffixes_have_explicit_outcomes() {
        let mut report = Report::default();
        equal(&mut report, "fixed", -2_147_483_649_i64, -2_147_483_648_i64);
        compare_literal(&mut report, "token", b"name\0native_tail", b"name");
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.limitations.len(), 1);
    }

    #[test]
    fn staggered_off_hours_skips_research_and_prints_refresh_hour() {
        let (mut frame, pack, country) = fixture();
        frame.hour = 1;
        let mut report = Report::default();
        compare_research(
            &frame,
            &pack,
            &BTreeMap::from([(11, &country)]),
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains(replay::cadence::STAGGER_LIMITATION)),
            "{:?}",
            report.limitations.keys()
        );
    }
}
