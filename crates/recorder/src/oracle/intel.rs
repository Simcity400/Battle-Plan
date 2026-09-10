//! Save comparisons for the intel pack's proved serialized fields.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::{GameDate, intel as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, TableSchema, tables},
};

use super::{Report, TokenNames, complete, invalid, required_country_tag};

#[derive(Clone, Copy)]
pub(super) struct ScopeKinds {
    pub global: u8,
    pub country: u8,
    pub state: u8,
}

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    scopes: ScopeKinds,
    report: &mut Report,
) -> io::Result<()> {
    let mut completed = report.attempt("IntelligenceAgencies", |report| {
        compare_agencies(frame, save, countries, report)
    });
    completed &= report.attempt("Subjects", |report| {
        compare_subjects(frame, save, countries, report)
    });
    completed &= report.attempt("Capitulation", |report| {
        compare_capitulation(frame, save, countries, report)
    });
    completed &= report.attempt("ScriptFlags", |report| {
        compare_flags(frame, save, countries, tokens, scopes.country, report)
    });
    completed &= report.attempt("OperationTokens", |report| {
        compare_operation_tokens(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("OperativeMemberships", |report| limited::<tables::OperativeMemberships>(
        frame,
        report,
        "operative membership and scalar save mappings are not proved; active agency count alone does not verify operative identities, roster kinds, XP, skill, capture or action fields",
    ));
    completed &= report.attempt("Cryptology", |report| limited::<tables::Cryptology>(
        frame,
        report,
        "cryptology constructor and loader establish native storage but the complete serialized target, scalar and omission mapping is not proved",
    ));
    completed &= report.attempt("IntelNetworks", |report| limited::<tables::IntelNetworks>(
        frame,
        report,
        "native subnetwork selectors and optional strength have no proved save mapping; negative selectors retain their raw value with absent strength, which is not a saved or inferred zero",
    ));
    completed &= report.attempt("Autonomy", |report| limited::<tables::Autonomy>(
        frame,
        report,
        "current autonomy status has no proved save mapping; subject-relation stored definition and freedom do not verify current score, bounds, adjacent states or last-change bits",
    ));
    completed &= report.attempt("ScriptVariables", |report| {
        compare_variables(frame, save, countries, scopes, report)
    });
    completed &= report.attempt("Operations", |report| limited::<tables::Operations>(
        frame,
        report,
        "operation instance fields have no complete proved save mapping; the raw definition token is retained, while its exact authored-name mapping remains unproved and no operation name is recorded",
    ));
    completed &= report.attempt("OperationAssignments", |report| limited::<tables::OperationAssignments>(
        frame,
        report,
        "operation assignment slots, including empty native slots, have no proved save census and scalar mapping",
    ));
    report.limitation(
        "country tags cannot distinguish raw authenticated country-reference aliases",
        "intel, subjects, capitulation, flags, variables and operation-token country joins",
    );
    if completed {
        Ok(())
    } else {
        Err(invalid(
            "intel table comparisons are incomplete; individual failures are reported",
        ))
    }
}

fn limited<T: TableSchema>(
    frame: &DecodedFrame,
    report: &mut Report,
    reason: &str,
) -> io::Result<()> {
    let rows = complete::<T>(frame)?;
    report.compared(&format!("{:?}", T::ID), 0);
    report.limitation(reason, format!("{} recorded rows", rows.len()));
    Ok(())
}

fn country<'a>(save: &'a saved::Pack, tag: &[u8]) -> io::Result<&'a saved::Country> {
    save.countries.get(tag).ok_or_else(|| {
        invalid(format!(
            "intel table references unknown saved country {}",
            String::from_utf8_lossy(tag)
        ))
    })
}

fn text(tag: &[u8]) -> String {
    String::from_utf8_lossy(tag).into_owned()
}

fn equal<T: std::fmt::Debug + PartialEq>(report: &mut Report, field: &str, actual: T, expected: T) {
    if actual != expected {
        report.difference(field, actual, expected);
    }
}

fn saved_hour(date: Option<GameDate>) -> io::Result<Option<i32>> {
    date.map(GameDate::engine_hour).transpose()
}

fn token<'a>(tokens: &TokenNames<'a>, id: u32) -> Option<&'a [u8]> {
    tokens.get(&id).copied()
}

fn missing_token_reason(frame: &DecodedFrame) -> &'static str {
    if matches!(
        &frame.tables[replay::rows::TableId::LexicalTokens.index()],
        replay::frame::TableView::Complete(_)
    ) {
        "complete recorded lexical dictionary has no entry for a token resolved by this serializer; the dependent name and fields remain unverified"
    } else {
        "recorded lexical dictionary is unavailable; the dependent token name and fields remain unverified"
    }
}

fn compare_agencies(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for actual in complete::<tables::IntelligenceAgencies>(frame)? {
        let tag = required_country_tag(actual.country, countries)?;
        if !seen.insert(tag) {
            return Err(invalid("duplicate resolved agency country"));
        }
        let subject = format!("{}.intelligence_agency", text(tag));
        let Some(expected) = &country(save, tag)?.agency else {
            report.limitation(
                "missing serialized agency payload cannot yet prove a null native agency pointer",
                subject,
            );
            continue;
        };
        matched += 1;
        report.scalar(
            &subject,
            "upgrade_progress",
            Some(actual.upgrade_progress_raw),
            Some(&expected.upgrade_progress),
        );
        report.scalar(
            &subject,
            "defense",
            Some(actual.defense_raw),
            Some(&expected.defense),
        );
        equal(
            report,
            &format!("{subject}.active_count"),
            actual.active_count,
            expected.active_count,
        );
        equal(
            report,
            &format!("{subject}.unknown_f0"),
            actual.unknown_f0,
            expected.unknown_f0,
        );
        equal(
            report,
            &format!("{subject}.admission_ceiling"),
            actual.admission_ceiling,
            expected.admission_ceiling,
        );
    }
    for (tag, expected) in &save.countries {
        if expected.agency.is_some() && !seen.contains(tag.as_slice()) {
            report.difference(
                format!("{}.intelligence_agency", text(tag)),
                "missing",
                "present",
            );
        }
    }
    report.compared("IntelligenceAgencies", matched);
    report.limitation("agency max_operative_count names the opaque +0xF0 dword; comparison does not establish its gameplay meaning", "IntelligenceAgencies.unknown_f0");
    Ok(())
}

fn compare_subjects(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::Subjects>(frame)? {
        let first = required_country_tag(actual.overlord, countries)?;
        let second = required_country_tag(actual.subject, countries)?;
        country(save, first)?;
        country(save, second)?;
        if first == second || recorded.insert((first, second), actual).is_some() {
            return Err(invalid("self or duplicate resolved subject relation"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for (owner, country) in &save.countries {
        let Some(subjects) = country.diplomacy.as_ref().and_then(|d| d.subjects.as_ref()) else {
            continue;
        };
        for expected in subjects {
            let key = (expected.first.as_slice(), expected.second.as_slice());
            if expected.roster_owner != *owner
                || expected.first != *owner
                || expected.target != expected.second
                || key.0 == key.1
                || !save.countries.contains_key(key.1)
                || !seen.insert(key)
            {
                return Err(invalid(
                    "saved subject relation has inconsistent ownership, endpoint or duplicate pair",
                ));
            }
            let label = format!("{}.puppet.{}", text(key.0), text(key.1));
            let Some(actual) = recorded.remove(&key) else {
                report.difference(&label, "missing", "present");
                continue;
            };
            matched += 1;
            equal(
                report,
                &format!("{label}.state"),
                Some(actual.state.as_slice()),
                expected.state.as_deref(),
            );
            report.scalar(
                &label,
                "value",
                Some(actual.autonomy_ratio),
                Some(&expected.autonomy_ratio),
            );
            equal(
                report,
                &format!("{label}.cancel"),
                actual.cancel != 0,
                expected.cancel,
            );
            if actual.cancel > 1 {
                report.limitation("yes/no subject cancel preserves truth but cannot certify noncanonical raw flag bits", &label);
            }
            if expected.cancel {
                report.limitation(
                    "cancelled subject relation omits its retained native start-hour word",
                    &label,
                );
            } else {
                equal(
                    report,
                    &format!("{label}.start_date"),
                    Some(actual.start_hour),
                    saved_hour(expected.start_date)?,
                );
            }
            let emitted_end = (actual.start_hour < actual.end_hour).then_some(actual.end_hour);
            equal(
                report,
                &format!("{label}.end_date"),
                emitted_end,
                saved_hour(expected.end_date)?,
            );
            if expected.end_date.is_none() {
                report.limitation("subject end-date omission verifies only end_hour <= start_hour, not the retained raw end-hour value", &label);
            }
        }
    }
    for ((owner, target), _) in recorded {
        let label = format!("{}.puppet.{}", text(owner), text(target));
        if save.countries[owner]
            .diplomacy
            .as_ref()
            .and_then(|d| d.subjects.as_ref())
            .is_some()
        {
            report.difference(label, "present", "missing");
        } else {
            report.limitation("missing serialized active-relations payload does not establish a complete subject census", label);
        }
    }
    report.compared("Subjects", matched);
    Ok(())
}

fn compare_capitulation(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    const DATE_SENTINEL: i32 = 0x29c77f8;
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for actual in complete::<tables::Capitulation>(frame)? {
        let tag = required_country_tag(actual.country, countries)?;
        if !seen.insert(tag) {
            return Err(invalid("duplicate resolved capitulation country"));
        }
        let label = format!("{}.diplomacy", text(tag));
        let Some(expected) = &country(save, tag)?.diplomacy else {
            report.limitation(
                "missing serialized diplomacy payload cannot verify capitulation state",
                label,
            );
            continue;
        };
        matched += 1;
        equal(
            report,
            &format!("{label}.capitulated"),
            actual.capitulated != 0,
            expected.capitulated.unwrap_or(false),
        );
        if actual.capitulated > 1 {
            report.limitation(
                "yes/no capitulation flag cannot certify noncanonical raw byte bits",
                &label,
            );
        }
        equal(
            report,
            &format!("{label}.capitulated_date"),
            (actual.capitulated != 0).then_some(actual.capitulated_hour),
            saved_hour(expected.capitulated_date)?,
        );
        if expected.capitulated_date.is_none() {
            report.limitation(
                "inactive capitulation omits its retained native capitulation-hour word",
                &label,
            );
        }
        equal(
            report,
            &format!("{label}.last_surrender_date"),
            (actual.last_surrender_hour != DATE_SENTINEL).then_some(actual.last_surrender_hour),
            saved_hour(expected.last_surrender_date)?,
        );
        equal(
            report,
            &format!("{label}.faction_join_date"),
            (actual.faction_join_hour != DATE_SENTINEL).then_some(actual.faction_join_hour),
            saved_hour(expected.faction_join_date)?,
        );
        if let Some(legitimacy) = &expected.legitimacy {
            report.scalar(
                &label,
                "legitimacy",
                Some(actual.legitimacy_raw),
                Some(legitimacy),
            );
        } else {
            report.limitation("legitimacy is serialized only for a positive exile host; omission cannot verify its native qword", &label);
        }
        equal(
            report,
            &format!("{label}.exile_army_leaders"),
            Some(actual.exile_army_leaders),
            expected.exile_army_leaders,
        );
    }
    for (tag, country) in &save.countries {
        if country.diplomacy.is_some() && !seen.contains(tag.as_slice()) {
            report.difference(format!("{}.diplomacy", text(tag)), "missing", "present");
        }
    }
    report.compared("Capitulation", matched);
    Ok(())
}

fn compare_flags(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    country_scope_kind: u8,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::<(_, _), Vec<_>>::new();
    let mut unresolved = BTreeSet::new();
    let mut raw_keys = BTreeSet::new();
    let mut country_counts = BTreeMap::<&[u8], usize>::new();
    for actual in complete::<tables::ScriptFlags>(frame)? {
        if actual.scope_kind != country_scope_kind {
            report.limitation("global and state flag save-root correspondence has not been proved; country flag coverage does not verify those scopes", format!("scope {}:{} token {}", actual.scope_kind, actual.scope_id, actual.token));
            continue;
        }
        let tag = required_country_tag(actual.scope_id, countries)?;
        country(save, tag)?;
        if !raw_keys.insert((tag, actual.token)) {
            return Err(invalid("duplicate resolved country and native flag token"));
        }
        *country_counts.entry(tag).or_default() += 1;
        let Some(name) = token(tokens, actual.token) else {
            unresolved.insert(tag);
            report.limitation(
                missing_token_reason(frame),
                format!("{} flag token {}", text(tag), actual.token),
            );
            continue;
        };
        if name.contains(&0) {
            unresolved.insert(tag);
            report.limitation(
                "flag-name formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; literal identity and dependent flag fields remain unverified",
                format!("{} flag token {}", text(tag), actual.token),
            );
            continue;
        }
        recorded.entry((tag, name)).or_default().push(actual);
    }
    let mut matched = 0;
    for (tag, expected) in &save.countries {
        let Some(flags) = &expected.flags else {
            continue;
        };
        equal(
            report,
            &format!("{}.flags count", text(tag)),
            country_counts.get(tag.as_slice()).copied().unwrap_or(0),
            flags.len(),
        );
        let mut expected_names = BTreeMap::<&[u8], Vec<_>>::new();
        for expected in flags {
            expected_names
                .entry(&expected.name)
                .or_default()
                .push(expected);
        }
        for (name, expected) in expected_names {
            let label = format!("{}.flags.{}", text(tag), text(name));
            let Some(actual) = recorded.remove(&(tag.as_slice(), name)) else {
                if unresolved.contains(tag.as_slice()) {
                    report.limitation("saved flag cannot be joined while this country's recorded flag names are unavailable", &label);
                } else {
                    report.difference(&label, "missing", "present");
                }
                continue;
            };
            matched += actual.len().min(expected.len());
            if actual.len() > 1 || expected.len() > 1 {
                report.limitation(
                    "distinct flag token ordinals can emit the same literal name; comparison verifies exact field multiplicities but cannot distinguish those native identities or their ordering",
                    &label,
                );
            }
            let mut actual_values = actual
                .iter()
                .map(|row| {
                    (
                        row.value,
                        row.set_hour,
                        (row.remaining_days > 0).then_some(row.remaining_days),
                    )
                })
                .collect::<Vec<_>>();
            let mut expected_values = expected
                .iter()
                .map(|row| Ok((row.value, row.date.engine_hour()?, row.remaining_days)))
                .collect::<io::Result<Vec<_>>>()?;
            if expected.iter().any(|row| row.remaining_days.is_none()) {
                report.limitation("omitted flag duration verifies nonpositive days only; distinct retained values such as 0 and -1 cannot be distinguished", &label);
            }
            actual_values.sort_unstable();
            expected_values.sort_unstable();
            if unresolved.contains(tag.as_slice()) && actual_values.len() < expected_values.len() {
                for value in actual_values {
                    if let Ok(index) = expected_values.binary_search(&value) {
                        expected_values.remove(index);
                    } else {
                        report.difference(&label, value, "no matching saved flag occurrence");
                    }
                }
                report.limitation("saved flag occurrences cannot all be joined while this scope has unresolved token names", label);
            } else {
                equal(report, &label, actual_values, expected_values);
            }
        }
    }
    for ((tag, name), _) in recorded {
        let label = format!("{}.flags.{}", text(tag), text(name));
        if save.countries[tag].flags.is_some() {
            report.difference(label, "present", "missing");
        } else {
            report.limitation("missing serialized flag-manager payload does not yet establish an empty native scope", label);
        }
    }
    report.compared("ScriptFlags", matched);
    Ok(())
}

fn multiset<'a>(values: impl IntoIterator<Item = &'a [u8]>) -> BTreeMap<&'a [u8], usize> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum VariableScope<'a> {
    Global,
    Country(&'a [u8]),
    State(u32),
}

impl VariableScope<'_> {
    fn label(self) -> String {
        match self {
            Self::Global => "global.variables".into(),
            Self::Country(tag) => format!("{}.variables", text(tag)),
            Self::State(id) => format!("state.{id}.variables"),
        }
    }
}

fn compare_variables(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    scopes: ScopeKinds,
    report: &mut Report,
) -> io::Result<()> {
    if scopes.global == scopes.country
        || scopes.global == scopes.state
        || scopes.country == scopes.state
    {
        return Err(invalid("variable scope discriminants are not distinct"));
    }
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::ScriptVariables>(frame)? {
        let scope = if actual.scope_kind == scopes.global {
            if actual.scope_id != 0 {
                return Err(invalid("global variable scope has a nonzero id"));
            }
            VariableScope::Global
        } else if actual.scope_kind == scopes.country {
            let tag = required_country_tag(actual.scope_id, countries)?;
            country(save, tag)?;
            VariableScope::Country(tag)
        } else if actual.scope_kind == scopes.state {
            if save
                .state_variables
                .as_ref()
                .is_some_and(|states| !states.contains_key(&actual.scope_id))
            {
                return Err(invalid(format!(
                    "variable scope references unknown saved state {}",
                    actual.scope_id
                )));
            }
            VariableScope::State(actual.scope_id)
        } else {
            return Err(invalid(format!(
                "unknown variable scope kind {}",
                actual.scope_kind
            )));
        };
        if recorded.insert(scope, actual).is_some() {
            return Err(invalid("duplicate resolved variable scope"));
        }
    }
    let mut expected_scopes = BTreeMap::from([(VariableScope::Global, &save.global_variables)]);
    for (tag, country) in &save.countries {
        expected_scopes.insert(VariableScope::Country(tag), &country.variables);
    }
    if let Some(states) = &save.state_variables {
        for (&id, variables) in states {
            expected_scopes.insert(VariableScope::State(id), variables);
        }
    }
    let mut matched = 0;
    for (scope, expected) in expected_scopes {
        let actual = recorded.remove(&scope);
        let label = scope.label();
        let Some(expected) = expected else {
            if actual.is_some() {
                report.limitation(
                    "serialized variables payload is absent at its proved scope; generic pointer-writer omission behavior is not proved, so absence does not certify an empty native container",
                    label,
                );
            }
            continue;
        };
        let Some(actual) = actual else {
            report.difference(&label, "missing", "present");
            continue;
        };
        matched += 1;
        let mut values = BTreeMap::<&[u8], Vec<i64>>::new();
        let mut unresolved_literal = false;
        for (index, entry) in actual.entries.iter().enumerate() {
            // BB9D60 passes a C-string pointer. The concrete stream target is
            // not proved, so do not infer a prefix projection from that ABI.
            if entry.name.contains(&0) {
                unresolved_literal = true;
                report.limitation(
                    "variable-name formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; this full native key and value remain unverified",
                    format!("{label} entry {index}"),
                );
                continue;
            }
            let name = entry.name.as_slice();
            if name == b"random" {
                report.limitation(
                    "a native variable key emits the reserved random metadata name; its identity and value cannot be distinguished from metadata in the save",
                    format!("{label} entry {index}"),
                );
                continue;
            }
            values.entry(name).or_default().push(entry.value_raw);
        }
        let mut expected_values = BTreeMap::<&[u8], Vec<&installed_content::save::Decimal>>::new();
        for entry in &expected.entries {
            expected_values
                .entry(&entry.name)
                .or_default()
                .push(&entry.value);
        }
        for (name, mut expected) in expected_values {
            let field = format!("{label}.{}", text(name));
            let Some(mut actual) = values.remove(name) else {
                if unresolved_literal {
                    report.limitation("saved variable occurrences cannot all be joined while this scope has unproved literal formatting", field);
                } else {
                    report.difference(field, "missing", "present");
                }
                continue;
            };
            actual.sort_unstable();
            expected.sort_unstable_by_key(|value| value.raw);
            if unresolved_literal && actual.len() < expected.len() {
                for actual in actual {
                    if let Ok(index) = expected.binary_search_by_key(&actual, |value| value.raw) {
                        report.scalar(&field, "value", Some(actual), Some(expected.remove(index)));
                    } else {
                        report.difference(&field, actual, "no matching saved variable occurrence");
                    }
                }
                report.limitation("saved variable occurrences cannot all be joined while this scope has unproved literal formatting", field);
                continue;
            }
            equal(
                report,
                &format!("{field} occurrences"),
                actual.len(),
                expected.len(),
            );
            for (actual, expected) in actual.into_iter().zip(expected) {
                report.scalar(&field, "value", Some(actual), Some(expected));
            }
        }
        for (name, values) in values {
            report.difference(format!("{label}.{}", text(name)), values, "missing");
        }
        if expected.reserved_entries != 0 {
            report.limitation(
                "CVariables random metadata is outside the recorded variable entries; same-named native keys cannot be separated from it in saves",
                format!("{label}: {} reserved entries", expected.reserved_entries),
            );
        }
    }
    for (scope, _) in recorded {
        report.limitation(
            "serialized states root is absent; state variable save mapping is proved but this input cannot verify the native state scope",
            scope.label(),
        );
    }
    report.compared("ScriptVariables", matched);
    report.limitation(
        "variable serializer sorts a copy of literal keys; native hash-table traversal order is not verified",
        "ScriptVariables",
    );
    Ok(())
}

fn compare_operation_tokens(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    let mut owners_with_targets = BTreeSet::new();
    let mut null_target_owners = BTreeSet::new();
    for actual in complete::<tables::OperationTokens>(frame)? {
        let owner = required_country_tag(actual.owner, countries)?;
        country(save, owner)?;
        owners_with_targets.insert(owner);
        if actual.target == 0 {
            if !null_target_owners.insert(owner) {
                return Err(invalid("duplicate resolved null operation-token target"));
            }
            report.limitation(
                "operation-token native null-target serialization is not proved",
                format!("{} tokens {:?}", text(owner), actual.tokens),
            );
            continue;
        }
        let target = required_country_tag(actual.target, countries)?;
        country(save, target)?;
        let mut names = Vec::new();
        let mut missing_ids = BTreeMap::<u32, usize>::new();
        let mut name_ids = BTreeMap::<&[u8], BTreeSet<u32>>::new();
        for &id in &actual.tokens {
            if let Some(name) = token(tokens, id) {
                if name.contains(&0) {
                    report.limitation(
                        "operation-token formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; this literal occurrence remains unverified",
                        format!("{}.operation_assets.{} token {id}", text(owner), text(target)),
                    );
                    continue;
                }
                names.push(name);
                name_ids.entry(name).or_default().insert(id);
            } else {
                *missing_ids.entry(id).or_default() += 1;
            }
        }
        let missing = actual.tokens.len() - names.len();
        if !missing_ids.is_empty() {
            report.limitation(
                missing_token_reason(frame),
                format!(
                    "{}.operation_assets.{}: raw id counts {missing_ids:?}",
                    text(owner),
                    text(target)
                ),
            );
        }
        for (name, ids) in name_ids {
            if ids.len() > 1 {
                report.limitation(
                    "distinct operation-token ordinals emit the same literal name; save comparison verifies name multiplicity but cannot distinguish those native identities",
                    format!("{}.operation_assets.{} name {} ids {ids:?}", text(owner), text(target), text(name)),
                );
            }
        }
        if recorded
            .insert((owner, target), (actual.tokens.len(), names, missing))
            .is_some()
        {
            return Err(invalid("duplicate resolved operation-token target"));
        }
    }
    let mut matched = 0;
    for (owner, country) in &save.countries {
        if let Some(manager) = &country.operation_tokens {
            // 13F2BA0 emits operation_assets for a nonempty native target map;
            // its callback 13F09E0 omits each empty token vector independently.
            equal(
                report,
                &format!("{}.operation_assets presence", text(owner)),
                owners_with_targets.contains(owner.as_slice()),
                manager.operation_assets.is_some(),
            );
        }
        let Some(expected) = country
            .operation_tokens
            .as_ref()
            .and_then(|manager| manager.operation_assets.as_ref())
        else {
            continue;
        };
        for expected in expected {
            let label = format!(
                "{}.operation_assets.{}",
                text(owner),
                text(&expected.target)
            );
            if !save.countries.contains_key(&expected.target) {
                return Err(invalid("unknown saved operation-token target country"));
            }
            let Some(actual) = recorded.remove(&(owner.as_slice(), expected.target.as_slice()))
            else {
                report.difference(&label, "missing", "present");
                continue;
            };
            let (count, names, missing) = actual;
            let expected_names = multiset(expected.names.iter().map(Vec::as_slice));
            if missing == 0 {
                matched += 1;
                equal(report, &label, multiset(names), expected_names);
            } else {
                equal(
                    report,
                    &format!("{label} count"),
                    count,
                    expected.names.len(),
                );
                for (name, count) in multiset(names) {
                    let saved_count = expected_names.get(name).copied().unwrap_or(0);
                    if count > saved_count {
                        report.difference(
                            format!("{label}.{} minimum known occurrences", text(name)),
                            count,
                            saved_count,
                        );
                    }
                }
            }
        }
    }
    for ((owner, target), (count, _, _)) in recorded {
        let label = format!("{}.operation_assets.{}", text(owner), text(target));
        if count == 0 {
            report.limitation("empty native operation-token target vectors are omitted by saves, so native map-entry presence cannot be verified", label);
        } else if save.countries[owner].operation_tokens.is_some() {
            report.difference(label, "present", "missing");
        } else {
            report.limitation("missing serialized token-manager payload cannot yet prove a null native manager pointer", label);
        }
    }
    report.compared("OperationTokens", matched);
    report.limitation("operation-token serializer sorts a copy by authored name; only exact names and multiplicities can be verified, not native vector order", "OperationTokens");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::TableView;
    use replay::rows::{TableId, TableRows};

    const SCOPES: ScopeKinds = ScopeKinds {
        global: 0,
        country: 1,
        state: 2,
    };

    fn frame(table: TableId, rows: TableRows) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            }),
            metrics: Default::default(),
        };
        frame.tables[table.index()] = TableView::Complete(rows);
        frame
    }

    fn parsed(input: &[u8]) -> saved::Pack {
        saved::parse(&installed_content::parser::parse(input).unwrap().items).unwrap()
    }

    fn germany() -> CountryRow {
        CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn agency_oracle_catches_one_raw_unit_and_refuses_incomplete_tables() {
        let pack = parsed(b"countries={GER={intelligence_agency={defense=90071992547.40993 max_operative_count=-1 usable_operative_slots=-2}}}");
        let country = germany();
        let links = BTreeMap::from([(101, &country), (102, &country)]);
        let mut actual = replay::rows::IntelligenceAgencyRow {
            country: 102,
            defense_raw: 9_007_199_254_740_993,
            unknown_f0: u32::MAX,
            admission_ceiling: -2,
            ..Default::default()
        };
        let table = TableId::IntelligenceAgencies;
        let mut report = Report::default();
        compare_agencies(
            &frame(table, TableRows::IntelligenceAgencies(vec![actual.clone()])),
            &pack,
            &links,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        actual.defense_raw -= 1;
        let mut report = Report::default();
        compare_agencies(
            &frame(table, TableRows::IntelligenceAgencies(vec![actual])),
            &pack,
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        let mut unavailable = frame(table, TableRows::empty(table));
        unavailable.tables[table.index()] = TableView::Unavailable {
            reason: "source read failed".into(),
        };
        assert!(compare_agencies(&unavailable, &pack, &links, &mut Report::default()).is_err());
        let mut report = Report::default();
        compare_agencies(
            &frame(table, TableRows::empty(table)),
            &pack,
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn independent_tables_continue_after_failures_and_preserve_partial_comparisons() {
        let pack = parsed(b"countries={GER={intelligence_agency={defense=0.00001 max_operative_count=0 usable_operative_slots=0}}}");
        let country = germany();
        let links = BTreeMap::from([(101, &country), (102, &country)]);
        let first = replay::rows::IntelligenceAgencyRow {
            country: 101,
            defense_raw: 2,
            ..Default::default()
        };
        let duplicate = replay::rows::IntelligenceAgencyRow {
            country: 102,
            ..first.clone()
        };
        let mut frame = frame(
            TableId::IntelligenceAgencies,
            TableRows::IntelligenceAgencies(vec![first, duplicate]),
        );
        for table in [
            TableId::IntelNetworks,
            TableId::ScriptVariables,
            TableId::OperationAssignments,
        ] {
            frame.tables[table.index()] = TableView::Unavailable {
                reason: format!("{table:?} source failed"),
            };
        }
        let mut report = Report::default();
        assert!(compare(&frame, &pack, &links, &BTreeMap::new(), SCOPES, &mut report).is_err());
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.scalar_checks, 2);
        for table in [
            "IntelligenceAgencies",
            "IntelNetworks",
            "ScriptVariables",
            "OperationAssignments",
        ] {
            assert!(report.failed_comparisons.contains_key(table), "{table}");
        }
        assert_eq!(report.compared_tables.get("Operations"), Some(&0));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("no operation name is recorded"))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("country-reference aliases"))
        );
    }

    #[test]
    fn flag_duration_omission_checks_predicate_without_certifying_sentinel() {
        let pack = parsed(b"countries={GER={flags={x={value=-32768 date=1936.1.1.1}}}}");
        let country = germany();
        let links = BTreeMap::from([(101, &country)]);
        let tokens = BTreeMap::from([(u32::MAX, b"x".as_slice())]);
        let date = GameDate::parse(b"1936.1.1.1")
            .unwrap()
            .engine_hour()
            .unwrap();
        for days in [i16::MIN, -1, 0, 1] {
            let mut report = Report::default();
            let recorded = frame(
                TableId::ScriptFlags,
                TableRows::ScriptFlags(vec![replay::rows::ScriptFlagRow {
                    scope_kind: 9,
                    scope_id: 101,
                    token: u32::MAX,
                    value: i16::MIN,
                    set_hour: date,
                    remaining_days: days,
                }]),
            );
            compare_flags(&recorded, &pack, &links, &tokens, 9, &mut report).unwrap();
            assert_eq!(report.differences.len(), usize::from(days > 0));
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("nonpositive days only"))
            );
        }
    }

    #[test]
    fn token_comparison_preserves_multiplicity_and_literal_bytes() {
        let expected = multiset([b"a".as_slice(), b"\xff".as_slice(), b"a".as_slice()]);
        assert_eq!(
            expected,
            multiset([b"\xff".as_slice(), b"a".as_slice(), b"a".as_slice()])
        );
        assert_ne!(expected, multiset([b"\xff".as_slice(), b"a".as_slice()]));
        let tokens = BTreeMap::from([(u32::MAX, b"\xff".as_slice())]);
        assert_eq!(token(&tokens, u32::MAX).unwrap(), b"\xff");
        assert!(token(&tokens, 0).is_none());
    }

    #[test]
    fn repeated_flag_names_preserve_field_multisets_and_limit_unproved_nul_names() {
        let pack = parsed(b"countries={GER={flags={x={value=-3 date=1936.1.1.1} x={value=4 date=1936.1.2.1 days=5} y={value=9 date=1936.1.3.1}}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let names = BTreeMap::from([
            (0, b"x".as_slice()),
            (1, b"x".as_slice()),
            (2, b"y\0unproved".as_slice()),
        ]);
        let hour = |date| GameDate::parse(date).unwrap().engine_hour().unwrap();
        let mut rows = vec![
            replay::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 0,
                value: -3,
                set_hour: hour(b"1936.1.1.1"),
                remaining_days: -1,
            },
            replay::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 1,
                value: 4,
                set_hour: hour(b"1936.1.2.1"),
                remaining_days: 5,
            },
            replay::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 2,
                value: -999,
                set_hour: -99,
                remaining_days: -9,
            },
        ];
        rows.reverse();
        let mut report = Report::default();
        compare_flags(
            &frame(TableId::ScriptFlags, TableRows::ScriptFlags(rows.clone())),
            &pack,
            &links,
            &names,
            SCOPES.country,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&2));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("distinct flag token ordinals"))
        );
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("concrete stream behavior for interior NUL is unproved")
        }));
        rows[1].value -= 1;
        let mut report = Report::default();
        compare_flags(
            &frame(TableId::ScriptFlags, TableRows::ScriptFlags(rows)),
            &pack,
            &links,
            &names,
            SCOPES.country,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn token_dictionary_gaps_are_distinct_from_unavailability_and_reserved_ids() {
        let pack = parsed(b"countries={GER={flags={\"\"={value=0 date=1936.1.1.1} known={value=1 date=1936.1.1.1}}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let names = BTreeMap::from([(0, b"".as_slice()), (1, b"known".as_slice())]);
        let date = GameDate::parse(b"1936.1.1.1")
            .unwrap()
            .engine_hour()
            .unwrap();
        let rows = [0, 1]
            .into_iter()
            .map(|token| replay::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token,
                value: token as i16,
                set_hour: date,
                remaining_days: 0,
            })
            .collect();
        let mut recorded = frame(TableId::ScriptFlags, TableRows::ScriptFlags(rows));
        let mut report = Report::default();
        compare_flags(
            &recorded,
            &pack,
            &links,
            &names,
            SCOPES.country,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&2));

        let partial_names = BTreeMap::from([(0, b"".as_slice())]);
        let mut report = Report::default();
        compare_flags(
            &recorded,
            &pack,
            &links,
            &partial_names,
            SCOPES.country,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&1));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("complete recorded lexical dictionary has no entry"))
        );

        recorded.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "source failed".into(),
        };
        let mut report = Report::default();
        compare_flags(
            &recorded,
            &pack,
            &links,
            &BTreeMap::new(),
            SCOPES.country,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&0));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("recorded lexical dictionary is unavailable"))
        );
    }

    #[test]
    fn missing_lexical_names_limit_value_joins_but_still_check_counts_and_known_values() {
        let pack = parsed(b"countries={GER={flags={x={value=1 date=1936.1.1.1}} tokens={operation_assets={FRA={b b}}}} FRA={}}");
        let ger = germany();
        let fra = CountryRow {
            id: 201,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(101, &ger), (201, &fra)]);
        let flags = frame(
            TableId::ScriptFlags,
            TableRows::ScriptFlags(vec![replay::rows::ScriptFlagRow {
                scope_kind: 1,
                scope_id: 101,
                token: 7,
                set_hour: -100,
                value: -9,
                remaining_days: 3,
            }]),
        );
        let mut report = Report::default();
        compare_flags(&flags, &pack, &links, &BTreeMap::new(), 1, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("dependent name and fields remain unverified"))
        );
        let empty = frame(TableId::ScriptFlags, TableRows::ScriptFlags(vec![]));
        let mut report = Report::default();
        compare_flags(&empty, &pack, &links, &BTreeMap::new(), 1, &mut report).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("flags count"))
        );
        let rows = frame(
            TableId::OperationTokens,
            TableRows::OperationTokens(vec![replay::rows::OperationTokensRow {
                owner: 101,
                target: 201,
                tokens: vec![1, 2],
            }]),
        );
        let mut report = Report::default();
        compare_operation_tokens(&rows, &pack, &links, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("dependent name and fields remain unverified"))
        );
        let mut report = Report::default();
        let partial = BTreeMap::from([(1, b"a".as_slice())]);
        compare_operation_tokens(&rows, &pack, &links, &partial, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("minimum known occurrences"));
    }

    #[test]
    fn operation_token_oracle_checks_outer_presence_and_empty_target_omission() {
        let pack = parsed(b"countries={GER={tokens={operation_assets={FRA={a a z}}}} FRA={}}");
        let ger = germany();
        let fra = CountryRow {
            id: 201,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(101, &ger), (201, &fra)]);
        let names = BTreeMap::from([(1, b"a".as_slice()), (2, b"z".as_slice())]);
        let table = TableId::OperationTokens;
        let mut row = replay::rows::OperationTokensRow {
            owner: 101,
            target: 201,
            tokens: vec![2, 1, 1],
        };
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        row.tokens.pop();
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        row.tokens.clear();
        let pack = parsed(b"countries={GER={tokens={operation_assets={}}} FRA={}}");
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("empty native operation-token target"))
        );
        let pack = parsed(b"countries={GER={tokens={}} FRA={}}");
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn operation_token_aliases_and_unproved_nul_names_keep_known_multiplicities() {
        let pack = parsed(b"countries={GER={tokens={operation_assets={FRA={x x y}}}} FRA={}}");
        let ger = germany();
        let fra = CountryRow {
            id: 201,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(101, &ger), (201, &fra)]);
        let names = BTreeMap::from([
            (0, b"x".as_slice()),
            (1, b"x".as_slice()),
            (2, b"y\0unproved".as_slice()),
        ]);
        let table = TableId::OperationTokens;
        let mut row = replay::rows::OperationTokensRow {
            owner: 101,
            target: 201,
            tokens: vec![1, 2, 0],
        };
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("distinct operation-token ordinals"))
        );
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("concrete stream behavior for interior NUL is unproved")
        }));
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("lexical dictionary has no entry"))
        );
        row.tokens[1] = 0;
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn variables_check_exact_literals_and_limit_unproved_nul_formatting() {
        let pack = parsed(b"countries={GER={variables={random=1 bar=-0.00001 foo=90071992547.40993 \"\"=0 \"\xff\"=-1.00001 unproved=999}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger), (102, &ger)]);
        let table = TableId::ScriptVariables;
        let mut row = replay::rows::ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 102,
            entries: [
                (b"foo".as_slice(), 9_007_199_254_740_993),
                (b"bar".as_slice(), -1),
                (b"".as_slice(), 0),
                (b"random".as_slice(), 1234),
                (b"unproved\0hidden".as_slice(), -5678),
                (b"\xff".as_slice(), -100_001),
            ]
            .into_iter()
            .map(|(name, value_raw)| replay::rows::ScriptVariableEntry {
                name: name.to_vec(),
                value_raw,
            })
            .collect(),
        };
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(vec![row.clone()])),
            &pack,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 4);
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&1));
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("concrete stream behavior for interior NUL is unproved")
        }));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("reserved random metadata name"))
        );
        row.entries.reverse();
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(vec![row.clone()])),
            &pack,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        row.entries.last_mut().unwrap().value_raw -= 1;
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(vec![row])),
            &pack,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        let mut unavailable = frame(table, TableRows::empty(table));
        unavailable.tables[table.index()] = TableView::Unavailable {
            reason: "native variable source failed".into(),
        };
        assert!(
            compare_variables(&unavailable, &pack, &links, SCOPES, &mut Report::default()).is_err()
        );
    }

    #[test]
    fn absent_variable_payloads_do_not_certify_empty_containers() {
        let table = TableId::ScriptVariables;
        let rows = [(SCOPES.global, 0), (SCOPES.country, 101), (SCOPES.state, 7)]
            .into_iter()
            .map(|(scope_kind, scope_id)| replay::rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                entries: vec![],
            })
            .collect();
        let recorded = frame(table, TableRows::ScriptVariables(rows));
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let absent = parsed(b"countries={GER={}}");
        let mut report = Report::default();
        compare_variables(&recorded, &absent, &links, SCOPES, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&0));
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("serialized variables payload is absent at its proved scope")
        }));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("serialized states root is absent"))
        );

        let empty =
            parsed(b"variables={} countries={GER={variables={}}} states={7={variables={}}}");
        let mut report = Report::default();
        compare_variables(&recorded, &empty, &links, SCOPES, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&3));
    }

    #[test]
    fn variable_scope_kinds_separate_global_country_and_full_width_state_ids() {
        let pack = parsed(b"variables={same=-0.00001} countries={GER={variables={same=0.00002}}} states={0={variables={same=90071992547.40993}} 4294967295={variables={same=-90071992547.40993}} 7={variables={}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger), (102, &ger)]);
        let table = TableId::ScriptVariables;
        let mut rows = [
            (SCOPES.global, 0, Some(-1)),
            (SCOPES.country, 102, Some(2)),
            (SCOPES.state, 0, Some(9_007_199_254_740_993)),
            (SCOPES.state, u32::MAX, Some(-9_007_199_254_740_993)),
            (SCOPES.state, 7, None),
        ]
        .into_iter()
        .map(
            |(scope_kind, scope_id, value)| replay::rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                entries: value
                    .into_iter()
                    .map(|value_raw| replay::rows::ScriptVariableEntry {
                        name: b"same".to_vec(),
                        value_raw,
                    })
                    .collect(),
            },
        )
        .collect::<Vec<_>>();
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(rows.clone())),
            &pack,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 4);
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&5));
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("save-root correspondence has not been proved"))
        );

        rows[2].entries[0].value_raw -= 1;
        rows[3].entries[0].value_raw += 1;
        rows.pop();
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(rows.clone())),
            &pack,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 3);

        rows[0].scope_id = 1;
        assert!(
            compare_variables(
                &frame(table, TableRows::ScriptVariables(rows.clone())),
                &pack,
                &links,
                SCOPES,
                &mut Report::default()
            )
            .is_err()
        );
        rows[0].scope_id = 0;
        rows[3].scope_id = 123;
        assert!(
            compare_variables(
                &frame(table, TableRows::ScriptVariables(rows)),
                &pack,
                &links,
                SCOPES,
                &mut Report::default()
            )
            .is_err()
        );
    }
}
