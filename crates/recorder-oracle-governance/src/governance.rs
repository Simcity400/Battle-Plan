
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    io,
};

use content_reader_save::{Save, governance as saved};
use replay_data::{rows::CountryRow, rows::Recorded, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{
    Report, TokenNames, complete, invalid, required_country_tag, reverse_spellings, token_spelling,
};

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
    let end = content_reader_parser::parser::c_string(token).len();
    if end < token.len() {
        report.limitation("native text serialization ends at the first NUL; captured trailing bytes are not save-comparable", subject);
    }
    &token[..end]
}

fn compare_literal(report: &mut Report, subject: &str, captured: &[u8], saved: &[u8]) {
    let captured = literal(report, subject, captured);
    equal(report, subject, captured, saved);
}

pub fn compare(
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
    complete &= report.attempt("governance ActiveIdeas", |report| {
        compare_ideas(frame, pack, links, tokens, report)
    });
    attempt!("TimedIdeas", compare_timed_ideas);
    attempt!("Focus", compare_focus);
    attempt!("CompletedFocuses", compare_completed_focuses);
    complete &= report.attempt("governance doctrines", |report| {
        doctrines::compare_doctrines(frame, pack, links, tokens, report)
    });
    complete &= report.attempt("governance Research", |report| {
        compare_research(frame, pack, links, tokens, report)
    });
    if let Ok(rows) = frame.complete::<tables::ResearchModifiers>() {
        report.limitation("research modifier operands and sources outside the separately compared grant inventory have synthetic coverage, but no fresh-save or live-rate oracle validation", "ResearchModifiers");
        let mut rosters = BTreeMap::<u32, Vec<_>>::new();
        for row in rows
            .iter()
            .filter(|row| row.kind == replay_data::rows::ResearchModifierRow::SHARING_ROSTER)
        {
            required_country_tag(row.country, links)?;
            rosters.entry(row.country).or_default().push(row);
        }
        for (owner, rows) in &rosters {
            let counts: Vec<_> = rows.iter().filter(|row| row.raw == -1).collect();
            if counts.len() != 1
                || counts
                    .first()
                    .is_some_and(|row| row.count as usize + 1 != rows.len())
                || rows.iter().any(|row| {
                    row.index != u32::MAX
                        || row.technology_handle != 0
                        || (row.raw != -1 && u32::try_from(row.raw).is_err())
                })
            {
                report.difference(
                    format!("ResearchModifiers sharing roster {owner}"),
                    "invalid count/identity witness",
                    "one count witness and its native group IDs",
                );
            }
        }
        report.compared("ResearchModifiers.SharingRoster self-check", rosters.len());
        report.limitation("the country technology-status sharing roster is not serialized; the global sharing members cannot certify this separate roster. Count/identity checks need matched native observations for membership, including an idle country", "ResearchModifiers.SharingRoster");
    }
    if frame.complete::<tables::ResearchInputs>().is_ok()
        || frame.complete::<tables::ResearchAppliedBonuses>().is_ok()
    {
        report.limitation("sliced research inputs and applied-bonus attribution have synthetic reader/codec coverage but no fresh-save oracle validation; rows keep their read hour in SliceReads, and the existing Research comparison covers only its own staggered tables", "ResearchInputs, ResearchAppliedBonuses");
    }
    attempt!("DecisionCounts", compare_decision_counts);
    attempt!("Decisions", compare_decisions);
    attempt!("DecisionRandomItems", compare_decision_random_items);
    attempt!("DynamicModifiers", compare_modifiers);
    attempt!("PreferredTactics", compare_tactics);
    attempt!("CountryTacticUnlocks", compare_tactic_unlocks);
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
                let party = format!("{subject}.{}", String::from_utf8_lossy(token));
                equal(
                    report,
                    format!("{party}.is_ruling"),
                    row.is_ruling,
                    u8::from(politics.ruling_party.as_deref() == Some(token)),
                );
                let owner = links
                    .get(&row.country)
                    .map(|country| country.id)
                    .ok_or_else(|| invalid("political party owner absent from countries"))?;
                equal(report, format!("{party}.tag"), row.tag, owner);
                let name = politics.names.get(token).cloned().unwrap_or_default();
                match name.default {
                    Some(default) => equal(
                        report,
                        format!("{party}.default"),
                        row.name_default,
                        u8::from(default),
                    ),
                    None => report.difference(
                        format!("{party}.default"),
                        row.name_default,
                        "default omitted from save",
                    ),
                }
                if let Some((saved_name, saved_long)) = &name.keys {
                    compare_literal(report, &format!("{party}.name"), &row.name, saved_name);
                    compare_literal(
                        report,
                        &format!("{party}.long_name"),
                        &row.long_name,
                        saved_long,
                    );
                }
                count += 1;
            }
            None => report.difference(subject, token, "missing saved political party"),
        }
    }
    for (tag, country) in report.canonical_countries(&pack.countries) {
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
    report.limitation("the game writes a party's name and long-name keys only when its default flag is clear; a default-named party's stored keys are not save-comparable", "PoliticalParties");
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
            format!("{subject}.capital"),
            Some(row.capital),
            country.capital,
        );
        equal(
            report,
            format!("{subject}.original_capital"),
            Some(row.original_capital),
            country.original_capital,
        );
        if let Some(&interval) = report.recorded(
            &row.election_interval,
            "Politics.election_interval",
            &subject,
        ) {
            equal(
                report,
                format!("{subject}.election_frequency"),
                Some(interval),
                saved.election_frequency,
            );
        }
        if let Recorded::Value(has_elections) = row.has_elections {
            equal(
                report,
                format!("{subject}.has_elections"),
                Some(has_elections),
                saved
                    .elections_allowed
                    .zip(saved.election_frequency)
                    .map(|(allowed, frequency)| allowed && frequency > 0),
            );
        }
        count += 1;
    }
    for (tag, country) in report.canonical_countries(&pack.countries) {
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
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let ideas = complete::<tables::ActiveIdeas>(frame)?;
    let spellings = reverse_spellings(tokens);
    let mut captured: BTreeMap<&[u8], BTreeMap<u32, &[u8]>> = BTreeMap::new();
    for row in ideas {
        let (tag, _) = country(pack, links, row.country)?;
        let token = literal(report, &label(tag, "ideas"), &row.token);
        let token = token_spelling(tokens, &spellings, token, "active idea")?;
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
    for (tag, country) in report.canonical_countries(&pack.countries) {
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

fn compare_timed_ideas(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut captured = BTreeMap::<&[u8], BTreeMap<u32, (&[u8], i32)>>::new();
    for row in complete::<tables::TimedIdeas>(frame)? {
        let (tag, _) = country(pack, links, row.country)?;
        let token = literal(report, &label(tag, "timed_idea"), &row.token);
        if captured
            .entry(tag)
            .or_default()
            .insert(row.ordinal, (token, row.days))
            .is_some()
        {
            return Err(invalid("duplicate timed-idea ordinal"));
        }
    }
    let mut count = 0;
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let actual = captured.remove(tag.as_slice()).unwrap_or_default();
        check_ordinals(actual.keys().copied())?;
        let expected = country
            .politics
            .as_ref()
            .map(|politics| politics.timed_ideas.as_slice())
            .unwrap_or_default();
        equal(
            report,
            label(tag, "timed_idea.ordered_tokens_and_days"),
            actual.values().copied().collect::<Vec<_>>(),
            expected
                .iter()
                .map(|idea| (idea.token.as_slice(), idea.days))
                .collect::<Vec<_>>(),
        );
        count += actual.len().min(expected.len());
    }
    report.compared("TimedIdeas", count);
    Ok(())
}

fn compare_decision_random_items(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut captured = BTreeMap::<&[u8], BTreeMap<u32, (&[u8], i32, u32)>>::new();
    for row in complete::<tables::DecisionRandomItems>(frame)? {
        let (tag, _) = country(pack, links, row.country)?;
        let token = literal(
            report,
            &label(tag, "decision_status.random_item"),
            &row.token,
        );
        if captured
            .entry(tag)
            .or_default()
            .insert(row.ordinal, (token, row.count, row.target))
            .is_some()
        {
            return Err(invalid("duplicate decision random-item ordinal"));
        }
    }
    let mut count = 0;
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let actual = captured.remove(tag.as_slice()).unwrap_or_default();
        check_ordinals(actual.keys().copied())?;
        let expected = country
            .decisions
            .as_ref()
            .map(|decisions| decisions.random_items.as_slice())
            .unwrap_or_default();
        equal(
            report,
            label(tag, "decision_status.random_item.ordered_entries"),
            actual.values().copied().collect::<Vec<_>>(),
            expected
                .iter()
                .map(|item| (item.token.as_slice(), item.count, item.target as u32))
                .collect::<Vec<_>>(),
        );
        count += actual.len().min(expected.len());
    }
    report.compared("DecisionRandomItems", count);
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
    for (tag, country) in report.canonical_countries(&pack.countries) {
        if country.focus.is_some() && !captured.contains(tag.as_slice()) {
            report.difference(label(tag, "focus"), "missing captured country", "present");
        }
    }
    report.limitation(
        "the focus writer writes paused only while the native byte is zero, and that write is not nested inside the selected-focus branches; an omitted paused therefore proves a nonzero byte for every country with a focus-progress object, without proving its exact bits",
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
    for (tag, country) in report.canonical_countries(&pack.countries) {
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

mod research;
use research::compare_research;
#[cfg(test)]
use research::compare_research_slots;

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
    for (tag, country) in report.canonical_countries(&pack.countries) {
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
        if let Some(&Some(seed)) = report.recorded(
            &row.description_seed,
            "Decisions.description_seed",
            format!("{subject}.description_seed"),
        ) {
            if row.container < 3 {
                return Err(invalid("untargeted decision has a description seed"));
            }
            let target = if (row.country_target as i32) > 0 {
                links
                    .get(&row.country_target)
                    .ok_or_else(|| invalid("decision seed target has no CountryLinks row"))?
                    .canonical_slot as i32
            } else {
                row.numeric_target as i32
            };
            let items = &country.decisions.as_ref().unwrap().random_items;
            let token = literal(report, &subject, &row.token);
            let saved_seed = items
                .iter()
                .find(|item| item.token.eq_ignore_ascii_case(token) && item.target == target)
                .map(|item| item.count);
            equal(
                report,
                format!("{subject}.description_seed"),
                seed,
                saved_seed,
            );
        }
        row_count += 1;
    }
    for (tag, country) in report.canonical_countries(&pack.countries) {
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
    report.limitation("description seeds are captured only for demanded targeted decision rows; unrelated random_item entries and undemanded seed lookups are outside this comparison", "Decisions.description_seed");
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
    for (tag, country) in report.canonical_countries(&pack.countries) {
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
    for (tag, country) in report.canonical_countries(&pack.countries) {
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

fn compare_tactic_unlocks(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::CountryTacticUnlocks>(frame)?;
    let statuses = pack
        .doctrine
        .as_ref()
        .ok_or_else(|| invalid("tactic unlock comparison requires the saved doctrine block"))?;
    let census: BTreeMap<u32, &CountryRow> = links
        .values()
        .map(|country| (country.canonical_slot, *country))
        .collect();
    let mut captured = BTreeMap::new();
    for row in rows {
        let country = links
            .get(&row.country)
            .ok_or_else(|| invalid("captured tactic unlock country has no country link"))?;
        if captured
            .insert(country.canonical_slot, row.unlocked_tactic_ids.as_slice())
            .is_some()
        {
            return Err(invalid("duplicate captured tactic unlock country"));
        }
    }
    let ids = |ids: &[i32]| ids.iter().map(|&id| i64::from(id)).collect::<Vec<_>>();
    for (&slot, country) in &census {
        let tag = country.tag.as_slice();
        let status = usize::try_from(slot)
            .ok()
            .and_then(|slot| statuses.get(slot));
        if let Some(status) = status
            && status.country.as_deref() != Some(tag)
        {
            report.difference(
                label(tag, "doctrine status country"),
                String::from_utf8_lossy(tag),
                status.country.as_deref().map(String::from_utf8_lossy),
            );
        }
        match (captured.get(&slot), status) {
            (Some(actual), Some(status)) => equal(
                report,
                label(tag, "enable_tactic"),
                actual.iter().map(|&id| i64::from(id)).collect(),
                ids(&status.enable_tactic),
            ),
            (Some(actual), None) => report.difference(
                label(tag, "enable_tactic"),
                actual,
                "no saved doctrine status for its slot",
            ),
            (None, status) => report.difference(
                label(tag, "enable_tactic"),
                "no captured row for a census country",
                status.map(|status| ids(&status.enable_tactic)),
            ),
        }
    }
    let mut excluded = 0;
    for (slot, status) in statuses.iter().enumerate() {
        if u32::try_from(slot).is_ok_and(|slot| census.contains_key(&slot)) {
            continue;
        }
        if status.enable_tactic.is_empty() {
            excluded += 1;
        } else {
            report.difference(
                format!("doctrine status {slot}.enable_tactic"),
                "slot outside the captured country census",
                ids(&status.enable_tactic),
            );
        }
    }
    report.compared("CountryTacticUnlocks", rows.len());
    if excluded > 0 {
        report.limitation(
            "the doctrine array has a status for every country slot, and the census records every non-null canonical country slot; a status whose slot is outside that census holds no live country and is compared only when it holds unlocks",
            format!("CountryTacticUnlocks: {excluded} empty statuses of slots outside the census"),
        );
    }
    Ok(())
}

mod doctrines;
mod sharing;
use sharing::{compare_sharing, compare_sharing_groups};

mod balance;
use balance::{
    compare_balance_members, compare_balance_overrides, compare_balance_ranges,
    compare_power_balance,
};

#[cfg(test)]
mod tests;
