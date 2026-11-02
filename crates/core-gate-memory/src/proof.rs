use crate::{
    manifest::{Family, invalid},
    readset::{Mode, ReadSet},
};
use replay::columns;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Serialize)]
pub struct SourceRead {
    pub family: &'static str,
    pub source: String,
    pub table: String,
    pub field: Option<String>,
    pub hand_kept_scope: Option<&'static str>,
}

#[derive(Clone)]
struct Token<'a> {
    text: &'a str,
    at: usize,
}

fn tokens(code: &str) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"//") {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at += 2;
            let mut depth = 1;
            while at < bytes.len() && depth != 0 {
                if bytes[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if bytes[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
            continue;
        }
        if bytes[at] == b'"' {
            at += 1;
            while at < bytes.len() {
                if bytes[at] == b'\\' {
                    at = (at + 2).min(bytes.len());
                } else if bytes[at] == b'"' {
                    at += 1;
                    break;
                } else {
                    at += 1;
                }
            }
        } else if bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' {
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
        } else {
            at += code[at..].chars().next().unwrap().len_utf8();
        }
        out.push(Token {
            text: &code[start..at],
            at: start,
        });
    }
    if let Some(at) = out.windows(9).position(|w| {
        w.iter()
            .map(|t| t.text)
            .eq(["#", "[", "cfg", "(", "test", ")", "]", "mod", "tests"])
    }) {
        out.truncate(at);
    }
    out
}

fn upstream(source: &Source, at: usize, table: &str) -> Option<&'static str> {
    if source.path == "crates/dashboard-base/src/products/country_history.rs" {
        if let Ok(legacy) = body(source.code, "fn observe(") {
            let start = legacy.as_ptr() as usize - source.code.as_ptr() as usize;
            if (start..start + legacy.len()).contains(&at) {
                return Some(
                    "legacy measure observation is outside the native identity replay path",
                );
            }
        }
    }
    match (source.family, source.path, table) {
        (
            "division-equipment",
            "crates/dashboard-army/src/products/army_content.rs",
            "Provinces",
        ) => Some("deployment_location is outside the division equipment replay"),
        (
            "division-equipment",
            "crates/dashboard-text/src/template_facts.rs",
            "CountryTemplateState" | "OrdersGroupCounterInputs",
        ) => Some("country_with and hq_with are outside the template helper used by this replay"),
        (
            "division-equipment",
            "crates/dashboard-base/src/products/equipment/columnar.rs",
            "VariantChanges" | "VariantModules" | "VariantUpgrades",
        ) => Some("variant naming is prepared upstream of the lead replay"),
        _ => None,
    }
}

fn extracted(source: &Source, at: usize, table: &str, field: Option<&str>) -> SourceRead {
    SourceRead {
        family: source.family,
        source: format!(
            "{}:{}",
            source.path,
            source.code[..at].bytes().filter(|&b| b == b'\n').count() + 1
        ),
        table: table.into(),
        field: field.map(str::to_owned),
        hand_kept_scope: upstream(source, at, table),
    }
}

fn row_types() -> BTreeMap<&'static str, Vec<&'static str>> {
    let mut types = BTreeMap::<_, Vec<_>>::new();
    for line in include_str!("../../replay-data/src/rows.rs").lines() {
        let Some((table, tail)) = line.split_once(" = ") else {
            continue;
        };
        let Some((_, tail)) = tail.split_once("=>") else {
            continue;
        };
        let Some((_, tail)) = tail.split_once('(') else {
            continue;
        };
        let Some((row, _)) = tail.split_once(')') else {
            continue;
        };
        if crate::manifest::table(table.trim()).is_ok() {
            types
                .entry(row.rsplit("::").next().unwrap())
                .or_default()
                .push(table.trim());
        }
    }
    types
}

fn row_fields(source: &Source, input: &[Token<'_>], out: &mut Vec<SourceRead>) {
    let types = row_types();
    let touched: BTreeSet<_> = out
        .iter()
        .filter(|r| r.family == source.family && r.source.starts_with(source.path))
        .map(|r| r.table.clone())
        .collect();
    let mut ends = vec![input.len(); input.len()];
    let mut stack = Vec::new();
    for (at, token) in input.iter().enumerate() {
        if token.text == "{" {
            stack.push(at);
        }
        if token.text == "}" {
            if let Some(start) = stack.pop() {
                ends[start] = at;
            }
        }
    }
    let mut scopes = Vec::new();
    for (at, token) in input.iter().enumerate() {
        while scopes.last().is_some_and(|&end| end <= at) {
            scopes.pop();
        }
        if token.text == "{" {
            scopes.push(ends[at]);
        }
        let mut binding = None;
        if token.text == "let" {
            let name = at + 1 + usize::from(input.get(at + 1).is_some_and(|t| t.text == "mut"));
            let end = (name..input.len())
                .find(|&p| input[p].text == ";")
                .unwrap_or(name);
            if let Some(row) = input[name..end].iter().find(|t| types.contains_key(t.text)) {
                binding = Some((
                    name,
                    end + 1,
                    scopes.last().copied().unwrap_or(input.len()),
                    row.text,
                ));
            }
        } else if token.text == "fn" {
            if let Some(start) = (at..input.len()).find(|&p| input[p].text == "{") {
                for p in at..start.saturating_sub(1) {
                    if input[p + 1].text != ":" {
                        continue;
                    }
                    let stop = (p + 2..start)
                        .find(|&q| matches!(input[q].text, "," | ")"))
                        .unwrap_or(start);
                    if let Some(row) = input[p + 2..stop]
                        .iter()
                        .find(|t| types.contains_key(t.text))
                    {
                        extract_members(
                            source,
                            input,
                            p,
                            start + 1,
                            ends[start],
                            &types[row.text],
                            &touched,
                            out,
                        );
                    }
                }
            }
        }
        if let Some((name, start, end, row)) = binding {
            extract_members(source, input, name, start, end, &types[row], &touched, out);
        }
    }
}

fn extract_members(
    source: &Source,
    input: &[Token<'_>],
    name: usize,
    start: usize,
    end: usize,
    tables: &[&str],
    touched: &BTreeSet<String>,
    out: &mut Vec<SourceRead>,
) {
    let name = input[name].text;
    let mut end = end;
    for at in start..end.saturating_sub(1) {
        if input[at].text == "let" && input[at + 1].text == name {
            end = at;
            break;
        }
    }
    for at in start.min(end)..end.saturating_sub(2) {
        if input[at].text != name || input[at + 1].text != "." {
            continue;
        }
        let mut member = input[at + 2].text.to_owned();
        let mut next = at + 3;
        while next + 1 < end {
            if input[next].text == "."
                && input[next + 1]
                    .text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                if input.get(next + 2).is_some_and(|t| t.text == "(") {
                    break;
                }
                member.push('.');
                member.push_str(input[next + 1].text);
                next += 2;
            } else if next + 2 < end && input[next].text == "[" && input[next + 2].text == "]" {
                member.push('[');
                member.push_str(input[next + 1].text);
                member.push(']');
                next += 3;
            } else {
                break;
            }
        }
        for &table in tables.iter().filter(|&&table| touched.contains(table)) {
            let id = crate::manifest::table(table).unwrap();
            for field in (0..id.schema().len()).filter_map(|field| id.field_name(field)) {
                let normalized: String = field.chars().filter(|c| !c.is_whitespace()).collect();
                if normalized == member
                    || normalized.starts_with(&format!("{member}."))
                    || normalized.starts_with(&format!("{member}["))
                {
                    out.push(extracted(source, input[at + 2].at, table, Some(field)));
                }
            }
        }
    }
}

pub fn source_reads() -> Vec<SourceRead> {
    let mut out = Vec::new();
    for source in SOURCES {
        let input = tokens(source.code);
        for window in input.windows(4) {
            if matches!(window[0].text, "TableId" | "tables")
                && window[1].text == ":"
                && window[2].text == ":"
                && crate::manifest::table(window[3].text).is_ok()
            {
                out.push(extracted(source, window[0].at, window[3].text, None));
            }
        }
        for at in 0..input.len().saturating_sub(10) {
            let tail = &input[at..];
            if !tail[1..4].iter().map(|t| t.text).eq([
                ":",
                ":",
                if tail[0].text == "columns" {
                    "field"
                } else {
                    "fields"
                },
            ]) || !tail[4..8]
                .iter()
                .map(|t| t.text)
                .eq(["(", "TableId", ":", ":"])
                || tail[9].text != ","
            {
                continue;
            }
            if tail[0].text == "columns" {
                if let Some(field) = tail[10]
                    .text
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                {
                    out.push(extracted(source, tail[10].at, tail[8].text, Some(field)));
                }
            } else if tail[0].text == "TableRead" {
                for token in tail[10..].iter().take_while(|t| t.text != "]") {
                    if let Some(field) = token
                        .text
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                    {
                        out.push(extracted(source, token.at, tail[8].text, Some(field)));
                    }
                }
            }
        }
        row_fields(source, &input, &mut out);
    }
    out
}

fn validate_sources(family: &Family, readset: &ReadSet) -> io::Result<()> {
    for text in family
        .readset
        .iter()
        .flat_map(|entry| [Some(entry.source.as_str()), entry.reason.as_deref()])
        .flatten()
        .chain(
            family
                .references
                .iter()
                .map(|reference| reference.source.as_str()),
        )
    {
        for tail in text.split("crates/").skip(1) {
            let Some((path, _)) = tail.split_once(".rs") else {
                continue;
            };
            let path = format!("crates/{path}.rs");
            if !SOURCES
                .iter()
                .any(|source| source.family == family.name && source.path == path)
            {
                return Err(invalid(format!(
                    "{}: cited helper {path} is absent from source extraction",
                    family.name
                )));
            }
        }
    }
    for read in source_reads()
        .into_iter()
        .filter(|r| r.family == family.name)
    {
        if read.hand_kept_scope.is_some() {
            continue;
        }
        let entry = readset
            .tables
            .iter()
            .find(|entry| format!("{:?}", entry.selected.table) == read.table)
            .ok_or_else(|| {
                invalid(format!(
                    "{}: source-derived access to {} at {} has no entry",
                    family.name, read.table, read.source
                ))
            })?;
        if entry.mode == Mode::None
            && READS
                .iter()
                .any(|p| p.family == family.name && p.table == read.table && p.access == "upstream")
        {
            continue;
        }
        if let Some(field) = read.field {
            let field_at = columns::field(entry.selected.table, &field)?;
            if !entry.selected.fields.contains(&field_at) {
                return Err(invalid(format!(
                    "{}: source-derived {}.{} at {} is missing",
                    family.name, read.table, field, read.source
                )));
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
pub struct SourcePolicy {
    family: &'static str,
    table: &'static str,
    mode: Mode,
    rows: crate::readset::Rows,
    hours: Option<crate::readset::Hours>,
    source: &'static str,
    evidence: Vec<String>,
}

fn compact(code: &str) -> String {
    let input = tokens(code);
    input
        .iter()
        .enumerate()
        .filter(|(at, token)| {
            !(token.text == ","
                && input
                    .get(at + 1)
                    .is_some_and(|next| matches!(next.text, ")" | "]" | "}")))
        })
        .map(|(_, token)| token.text)
        .collect()
}

fn body<'a>(code: &'a str, signature: &str) -> io::Result<&'a str> {
    let at = code
        .find(signature)
        .ok_or_else(|| invalid(format!("Source proof lost {signature}")))?;
    let start = at
        + code[at..]
            .find('{')
            .ok_or_else(|| invalid("Source proof lost body"))?;
    let mut depth = 0;
    for (offset, ch) in code[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(&code[start + 1..start + offset]);
                }
            }
            _ => {}
        }
    }
    Err(invalid("Unclosed source proof body"))
}

fn source_code(path: &str) -> io::Result<String> {
    SOURCES
        .iter()
        .find(|s| s.path == path)
        .map(|s| compact(s.code))
        .ok_or_else(|| invalid(format!("Missing policy source {path}")))
}

fn witness(code: &str, expression: &str) -> io::Result<String> {
    if !code.contains(expression) {
        return Err(invalid(format!("Source policy changed: {expression}")));
    }
    Ok(expression.into())
}

pub fn source_policies() -> io::Result<Vec<SourcePolicy>> {
    use crate::readset::{Hours, Rows};
    let mut out = Vec::new();
    let equipment =
        source_code("crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs")?;
    let diplomacy = source_code("crates/dashboard/src/products/event_feed/diplomacy.rs")?;
    let feed = source_code("crates/dashboard/src/products/event_feed/columnar.rs")?;
    let variants = source_code("crates/dashboard-base/src/products/equipment/columnar.rs")?;
    let ledger =
        source_code("crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs")?;
    let gate = compact(include_str!("readset.rs"));
    let equipment_rows = witness(&equipment, ">>32==TableId::CombatLogEquipmentasu128")?;
    witness(&gate, &format!("key{equipment_rows}"))?;
    out.push(SourcePolicy {
        family: "equipment_losses",
        table: "SliceReads",
        mode: Mode::Stream,
        rows: Rows::EquipmentSlices,
        hours: None,
        source: "equipment_losses/columnar.rs: slices",
        evidence: vec![equipment_rows],
    });
    let slice = body(&diplomacy, "fnreads_slice(")?;
    if slice != "read.table==TableId::RelationTuples.index()asu32&&read.sample_hour==hour" {
        return Err(invalid("Relation slice row/hour predicate changed"));
    }
    witness(&gate, "sample==hour")?;
    out.push(SourcePolicy {
        family: "event_feed",
        table: "SliceReads",
        mode: Mode::Snapshot,
        rows: Rows::RelationSlices,
        hours: Some(Hours::Every),
        source: "event_feed/diplomacy.rs: reads_slice",
        evidence: vec![slice.into()],
    });
    let guarantee = "(key>>16)asu32==engine_native::cguarantee_relation::TYPE";
    witness(&feed, guarantee)?;
    witness(&gate, guarantee)?;
    out.push(SourcePolicy {
        family: "event_feed",
        table: "RelationTuples",
        mode: Mode::Stream,
        rows: Rows::Guarantees,
        hours: None,
        source: "event_feed/columnar.rs: guaranteed",
        evidence: vec![guarantee.into()],
    });
    let presence = witness(
        &variants,
        "columns::complete(state,TableId::VariantChanges)&&columns::table(state,TableId::VariantChanges)?.live().ones().next().is_some()",
    )?;
    for family in ["equipment_losses", "production"] {
        out.push(SourcePolicy {
            family,
            table: "VariantChanges",
            mode: Mode::Presence,
            rows: Rows::All,
            hours: None,
            source: "equipment/columnar.rs: VariantChanges witness",
            evidence: vec![presence.clone()],
        });
    }
    let charged = body(&feed, "fncharged(")?;
    let charge =
        "matches!(kind,EventFeedKind::DecisionSelected|EventFeedKind::DecisionTimeoutEffect)";
    if charged != charge {
        return Err(invalid("Decision charge kinds changed"));
    }
    let mut evidence = vec![charged.into(), witness(&feed, "ifread_hour==hour.hour")?];
    let sources = "matches!(event.source,EventFeedSource::DecisionSelected|EventFeedSource::DecisionTimeoutEffect)";
    evidence.push(witness(&feed, sources)?);
    witness(&gate, sources)?;
    for table in ["CountryPowerChanges", "PoliticsCaptureCoverage"] {
        witness(
            body(&feed, "fncharges_at(")?,
            &format!("whole::<tables::{table}>(hour,format)"),
        )?;
        out.push(SourcePolicy {
            family: "event_feed",
            table,
            mode: Mode::Snapshot,
            rows: Rows::All,
            hours: Some(Hours::DecisionCharge),
            source: "event_feed/columnar.rs: packet, charged, draft, charges_at",
            evidence: evidence.clone(),
        });
    }
    let named = body(&ledger, "fnnamed(")?;
    witness(&gate, &format!("letnamed={named};"))?;
    let mut evidence = vec![named.into()];
    for expression in [
        "power.iter().any(|power|power.row.hour==at&&named(&power.row))||costs.iter().any(|cost|cost.row.day_hour==at&&cost.row.cost_raw!=0)",
        "[(hour,None),(hour.saturating_sub(1),held.as_ref())]",
        "complete&bit(TableId::PoliticsCaptureCoverage)==0||captures.changed.iter().any(|row|row.since_hour==hour&&row.through_hour==hour)",
    ] {
        evidence.push(witness(&ledger, expression)?);
    }
    for expression in [
        "(row.since_hour==row.through_hour).then_some(row.since_hour)",
        "(row.cost_raw!=0).then_some(row.day_hour)",
        "named.then_some(row.hour)",
        ".then_some(row.read_hour)",
        ".first((dated(hour),0))?",
        "!columns::complete(state,TableId::PoliticsCaptureCoverage)",
    ] {
        witness(&gate, expression)?;
    }
    for table in [
        "CountryCivilWars",
        "CountryCounterInputs",
        "CountryWars",
        "WarRelations",
        "ModifierDefinitions",
    ] {
        out.push(SourcePolicy {
            family: "politics_ledger",
            table,
            mode: Mode::Snapshot,
            rows: Rows::All,
            hours: Some(Hours::LedgerDated),
            source: "politics_ledger/facts.rs: named, wanted, discontinuous",
            evidence: evidence.clone(),
        });
    }
    Ok(out)
}

fn validate_policies(family: &Family, readset: &ReadSet) -> io::Result<()> {
    use crate::readset::Hours;
    for policy in source_policies()? {
        if policy.family != family.name {
            continue;
        }
        let entry = readset
            .tables
            .iter()
            .find(|e| format!("{:?}", e.selected.table) == policy.table);
        if entry.is_none_or(|e| {
            (e.mode != policy.mode
                && !(policy.family == "politics_ledger" && e.mode == Mode::Stream))
                || e.filter != policy.rows
                || (e.mode != Mode::Stream && e.hours != policy.hours)
        }) {
            return Err(invalid(format!(
                "{}: {} changes the source-derived row/hour rule at {}",
                policy.family, policy.table, policy.source
            )));
        }
    }
    if family.name == "politics_ledger" {
        for table in [
            "AdvisorRoles",
            "Autonomy",
            "CharacterPortraits",
            "Characters",
            "CountryAppearance",
            "IndustrialOrganisations",
            "LexicalTokens",
            "PoliticalParties",
            "Politics",
            "ReservedCountryNaming",
            "States",
            "UnitLeaders",
        ] {
            if readset
                .tables
                .iter()
                .find(|e| format!("{:?}", e.selected.table) == table)
                .is_none_or(|e| e.mode != Mode::Referenced)
            {
                return Err(invalid(format!(
                    "Ledger lookup {table} requires key hydration (items.rs:139-326,525; country_names.rs:787-808)"
                )));
            }
        }
        for table in [
            "CountryCivilWars",
            "CountryCounterInputs",
            "CountryWars",
            "WarRelations",
            "ModifierDefinitions",
        ] {
            if readset
                .tables
                .iter()
                .find(|e| format!("{:?}", e.selected.table) == table)
                .is_none_or(|e| {
                    !matches!(e.mode, Mode::Stream | Mode::Snapshot)
                        || (e.mode == Mode::Snapshot && e.hours != Some(Hours::LedgerDated))
                })
            {
                return Err(invalid(format!(
                    "Ledger snapshot {table} requires dated/held hours (facts.rs:876-889,994)"
                )));
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
pub struct Proof {
    pub family: &'static str,
    pub table: &'static str,
    pub access: &'static str,
    pub source: &'static str,
    pub fields: &'static [&'static str],
    pub all_fields: bool,
}

pub const READS: &[Proof] = &[
    Proof {
        family: "construction",
        table: "BuildingConstruction",
        access: "snapshot",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:306; crates/dashboard-construction/src/products/construction_log/facts.rs:354",
        fields: &[
            "country",
            "active_factories",
            "cost",
            "amount",
            "created_hour",
            "building_type",
            "target_flags",
            "target_location",
            "target_state",
            "repair_levels",
        ],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "ConstructionLineOutput",
        access: "snapshot",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:422; crates/dashboard-construction/src/products/construction_log/facts.rs:452",
        fields: &[
            "railway",
            "construction_raw",
            "repair_raw",
            "since_hour",
            "observed_since_creation",
            "linked",
            "ended",
            "exact",
            "repair_ticks",
            "completion",
        ],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard-world/src/countries.rs:595; crates/dashboard-construction/src/products/construction_log/facts.rs:289",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard-world/src/countries.rs:578; crates/dashboard-construction/src/products/construction_log/facts.rs:296",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "LexicalTokens",
        access: "lookup",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:482; crates/dashboard-construction/src/products/construction_log/facts.rs:522; crates/dashboard-construction/src/products/construction_log/facts.rs:541",
        fields: &["literal"],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "ProvinceBuildings",
        access: "snapshot",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:207; crates/dashboard-construction/src/products/construction_log/facts.rs:248",
        fields: &["level", "healthy_level"],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "StateBuildings",
        access: "snapshot",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:207; crates/dashboard-construction/src/products/construction_log/facts.rs:248",
        fields: &["level", "healthy_level"],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "Provinces",
        access: "lookup",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:368; crates/dashboard-construction/src/products/construction_log/facts.rs:401",
        fields: &["state"],
        all_fields: false,
    },
    Proof {
        family: "construction",
        table: "RailwayConstruction",
        access: "snapshot",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:391; crates/dashboard-construction/src/products/construction_log/facts.rs:399",
        fields: &[
            "country",
            "active_factories",
            "cost",
            "amount",
            "created_hour",
            "base_level",
            "target_level",
            "route",
        ],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "Campaign",
        access: "upstream",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:29",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "division-equipment",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:379; crates/dashboard-army/src/products/division_equipment/history/engine.rs:1193",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:418; crates/dashboard-army/src/products/division_equipment/history/engine.rs:1199",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "CountryAppearance",
        access: "snapshot",
        source: "crates/dashboard-army/src/products/division_equipment/history/engine.rs:1205; crates/dashboard-army/src/products/division_equipment/history/engine.rs:2716",
        fields: &["name_tag"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "CountryLocalSubunits",
        access: "snapshot",
        source: "crates/dashboard-text/src/subunit_requirements.rs:716",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "division-equipment",
        table: "DivisionEquipment",
        access: "snapshot",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:196",
        fields: &["variant_handle", "amount_raw"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "NewDivisionEquipment",
        access: "snapshot",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:196",
        fields: &["variant_handle", "amount_raw"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "Stockpiles",
        access: "snapshot",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:196",
        fields: &["variant_handle", "amount_raw"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "DivisionLandInputs",
        access: "snapshot",
        source: "crates/dashboard-world/src/divisions.rs:381",
        fields: &["manpower"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "DivisionStatus",
        access: "snapshot",
        source: "crates/dashboard-world/src/divisions.rs:363",
        fields: &["maximum_hit_points"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "Divisions",
        access: "snapshot",
        source: "crates/dashboard-world/src/divisions.rs:317",
        fields: &[
            "roster_owner",
            "logical_owner",
            "expeditionary_owner",
            "template",
            "hit_points",
            "reveal_skip",
        ],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "DivisionTemplateIcons",
        access: "lookup",
        source: "crates/dashboard-army/src/products/division_needs.rs:103; crates/dashboard-army/src/products/army_content.rs:244; crates/dashboard-text/src/template_facts.rs:76",
        fields: &[
            "priority_unit",
            "priority_unit_name",
            "custom_icon",
            "representative",
            "representative_local",
            "subunits",
            "country",
            "model_unreadable",
            "display_name",
            "settings",
        ],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "DivisionTemplates",
        access: "lookup",
        source: "crates/dashboard-text/src/template_facts.rs:67",
        fields: &["owner", "name", "name_key"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "LexicalTokens",
        access: "lookup",
        source: "crates/dashboard-army/src/products/division_equipment/history/engine.rs:960; crates/dashboard-text/src/table_access.rs:214; crates/dashboard-army/src/products/army_content.rs:279",
        fields: &["literal"],
        all_fields: false,
    },
    Proof {
        family: "division-equipment",
        table: "VariantDefinitions",
        access: "snapshot",
        source: "crates/dashboard-army/src/products/division_equipment/history/engine.rs:1025; crates/dashboard-army/src/products/division_equipment/history/engine.rs:1193",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "equipment_losses",
        table: "Campaign",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:850",
        fields: &["id"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "CombatLogEquipment",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:542; crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:702",
        fields: &["amount_raw"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "CombatLogState",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:384",
        fields: &["viewing_country_raw", "prune_observed"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:886; crates/dashboard-base/src/products/equipment/columnar.rs:608",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:831; crates/dashboard-base/src/products/equipment/columnar.rs:608",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "CountryAppearance",
        access: "lookup",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:622",
        fields: &["name_tag"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "DivisionCensus",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:244; crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:276",
        fields: &["grouped", "ungrouped"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "EquipmentDestruction",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:449",
        fields: &["amount_raw", "coverage", "unavailable_reason"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "LendLeaseEquipment",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:805",
        fields: &["amount_raw"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "LexicalTokens",
        access: "lookup",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:645",
        fields: &["literal"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "Players",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:404",
        fields: &["country"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "SimulationFeatures",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:425",
        fields: &["dlc_mask"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "SliceReads",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:648",
        fields: &["sample_hour"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "SunkConvoys",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:785",
        fields: &["convoys"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "SunkShips",
        access: "snapshot",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:747",
        fields: &["entries"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "VariantChanges",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:488",
        fields: &[],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "VariantDefinitions",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:521; crates/dashboard-base/src/products/equipment/columnar.rs:338",
        fields: &[
            "equipment_token",
            "creator_link",
            "origin_link",
            "append_variant_name",
            "display_name",
            "variant_name",
        ],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "VariantModules",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:413; crates/dashboard-base/src/products/equipment/columnar.rs:569",
        fields: &["slot", "definition_token", "definition_active"],
        all_fields: false,
    },
    Proof {
        family: "equipment_losses",
        table: "VariantUpgrades",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:421; crates/dashboard-base/src/products/equipment/columnar.rs:577",
        fields: &["definition_token", "level"],
        all_fields: false,
    },
    Proof {
        family: "event_feed",
        table: "Campaign",
        access: "upstream",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:69",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "event_feed",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:744; crates/dashboard-base/src/products/country_history.rs:805",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "event_feed",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:744; crates/dashboard-base/src/products/country_history.rs:806",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "event_feed",
        table: "CountryCounterInputs",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/country_history.rs:900",
        fields: &["owned_state_count"],
        all_fields: false,
    },
    Proof {
        family: "event_feed",
        table: "CountryExperienceGains",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/country_history/native.rs:314",
        fields: &["instance"],
        all_fields: false,
    },
    Proof {
        family: "event_feed",
        table: "CountryPowerChanges",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:550; crates/dashboard/src/products/event_feed/columnar.rs:566",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "event_feed",
        table: "EventFeedHistory",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:308; crates/dashboard/src/products/event_feed/columnar.rs:330",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "event_feed",
        table: "NuclearStrikes",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:744",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "event_feed",
        table: "PoliticsCaptureCoverage",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:550; crates/dashboard/src/products/event_feed/columnar.rs:566",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "event_feed",
        table: "RelationTuples",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:434; crates/dashboard/src/products/event_feed/columnar.rs:649",
        fields: &["first_country", "second_country", "start_hour", "cancel"],
        all_fields: false,
    },
    Proof {
        family: "event_feed",
        table: "SliceReads",
        access: "snapshot",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:397; crates/dashboard/src/products/event_feed/columnar.rs:421",
        fields: &["sample_hour"],
        all_fields: false,
    },
    Proof {
        family: "identities",
        table: "Campaign",
        access: "upstream",
        source: "crates/dashboard/src/prepared/driver/native/identities.rs:128",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "identities",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/country_history.rs:805",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "identities",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/country_history.rs:806",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "identities",
        table: "CountryCounterInputs",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/country_history.rs:900",
        fields: &["owned_state_count"],
        all_fields: false,
    },
    Proof {
        family: "identities",
        table: "CountryExperienceGains",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/country_history/native.rs:314",
        fields: &["instance"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "Campaign",
        access: "upstream",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:71",
        fields: &[],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "AdvisorRoles",
        access: "lookup",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:273",
        fields: &["idea", "slot"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "Autonomy",
        access: "lookup",
        source: "crates/installed-content-country-names/src/country_names.rs:790",
        fields: &["state"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "CharacterPortraits",
        access: "lookup",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:244",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "politics_ledger",
        table: "Characters",
        access: "lookup",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:182",
        fields: &[
            "name_token_id",
            "token",
            "template_present",
            "template_tag",
            "template_file",
            "template_line",
            "template_generated",
        ],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:803; crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:300",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryAppearance",
        access: "lookup",
        source: "crates/installed-content-country-names/src/country_names.rs:787",
        fields: &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryCivilWars",
        access: "snapshot",
        source: "crates/dashboard-text/src/identity.rs:146",
        fields: &["target"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryCommandPower",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:552",
        fields: &["allocated_command_power_raw"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryCounterInputs",
        access: "snapshot",
        source: "crates/dashboard-text/src/identity.rs:148",
        fields: &["owned_state_count"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:818; crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:502",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryPowerChanges",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:829",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryPowerDaily",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:839",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryPowerDailyCosts",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:844",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "politics_ledger",
        table: "CountryWars",
        access: "snapshot",
        source: "crates/dashboard-text/src/identity.rs:170",
        fields: &["enemies_read", "enemies"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "GovernmentChanges",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:834",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "politics_ledger",
        table: "IndustrialOrganisations",
        access: "lookup",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:292",
        fields: &["token", "owner", "name", "icon"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "LexicalTokens",
        access: "lookup",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:589; crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:635; crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:139; crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:184; crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:237",
        fields: &["literal"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "ModifierDefinitions",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:634; crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:326",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "politics_ledger",
        table: "PoliticalParties",
        access: "lookup",
        source: "crates/installed-content-country-names/src/country_names.rs:789",
        fields: &["token"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "Politics",
        access: "lookup",
        source: "crates/installed-content-country-names/src/country_names.rs:788",
        fields: &["ruling_party"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "PoliticsCaptureCoverage",
        access: "snapshot",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs:468",
        fields: &["epoch", "since_hour", "through_hour", "streams", "instance"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "ReservedCountryNaming",
        access: "lookup",
        source: "crates/installed-content-country-names/src/country_names.rs:556",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "politics_ledger",
        table: "States",
        access: "lookup",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:525",
        fields: &[],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "UnitLeaders",
        access: "lookup",
        source: "crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:170; crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs:204",
        fields: &["character_id", "category"],
        all_fields: false,
    },
    Proof {
        family: "politics_ledger",
        table: "WarRelations",
        access: "snapshot",
        source: "crates/dashboard-text/src/identity.rs:194",
        fields: &["start_hour", "end_hour"],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "Campaign",
        access: "upstream",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:31",
        fields: &[],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:504; crates/dashboard-base/src/products/equipment/columnar.rs:608",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "CountryAppearance",
        access: "lookup",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:622",
        fields: &["name_tag"],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:504; crates/dashboard-base/src/products/equipment/columnar.rs:608",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "LexicalTokens",
        access: "lookup",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:645",
        fields: &["literal"],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "ProductionLineOutput",
        access: "snapshot",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:519",
        fields: &[
            "country",
            "variant",
            "units",
            "since_hour",
            "ended",
            "exact",
            "spent_raw",
            "purpose",
        ],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "ProductionLines",
        access: "snapshot",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:217",
        fields: &["country"],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "RocketProductionOutput",
        access: "snapshot",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:519",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "production",
        table: "VariantChanges",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:488",
        fields: &[],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "VariantDefinitions",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:521; crates/dashboard-base/src/products/equipment/columnar.rs:338",
        fields: &[
            "equipment_token",
            "creator_link",
            "origin_link",
            "append_variant_name",
            "display_name",
            "variant_name",
        ],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "VariantModules",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:413; crates/dashboard-base/src/products/equipment/columnar.rs:569",
        fields: &["slot", "definition_token", "definition_active"],
        all_fields: false,
    },
    Proof {
        family: "production",
        table: "VariantUpgrades",
        access: "snapshot",
        source: "crates/dashboard-base/src/products/equipment/columnar.rs:421; crates/dashboard-base/src/products/equipment/columnar.rs:577",
        fields: &["definition_token", "level"],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "Campaign",
        access: "upstream",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:46",
        fields: &[],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "CharacterPortraits",
        access: "lookup",
        source: "crates/dashboard-special-projects/src/products/special_projects.rs:902",
        fields: &[],
        all_fields: true,
    },
    Proof {
        family: "projects",
        table: "Characters",
        access: "lookup",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:1064; crates/dashboard-special-projects/src/products/special_projects.rs:882",
        fields: &["name_token_id", "token"],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "Countries",
        access: "snapshot",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:545; crates/dashboard-special-projects/src/products/special_projects/facts.rs:782",
        fields: &["canonical_slot", "tag"],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "CountryLinks",
        access: "snapshot",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:550",
        fields: &["canonical_slot"],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "LexicalTokens",
        access: "snapshot",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:715; crates/dashboard-special-projects/src/products/special_projects.rs:934",
        fields: &["literal"],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "ProjectHistory",
        access: "snapshot",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:873; crates/dashboard-special-projects/src/products/special_projects/facts.rs:897",
        fields: &["prototype_reward_handle", "option_handle", "date"],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "ProjectPrograms",
        access: "lookup",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:1044; crates/dashboard-special-projects/src/products/special_projects.rs:864",
        fields: &["scientist"],
        all_fields: false,
    },
    Proof {
        family: "projects",
        table: "Projects",
        access: "snapshot",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:821",
        fields: &[
            "definition_id",
            "program_reference",
            "started",
            "state.current_offset",
            "state.not_completed",
            "state.prototype_progress",
        ],
        all_fields: false,
    },
];

pub fn validate(family: &Family, readset: &ReadSet) -> io::Result<()> {
    validate_sources(family, readset)?;
    validate_policies(family, readset)?;
    for rule in READ_RULES.iter().filter(|r| r.family == family.name) {
        let entry = readset
            .tables
            .iter()
            .find(|t| format!("{:?}", t.selected.table) == rule.table)
            .filter(|t| t.mode != Mode::None && t.selected.read_rule.as_deref() == Some(rule.rule));
        if entry.is_none() {
            return Err(invalid(format!(
                "{}: {} requires {} read evidence at {}",
                family.name, rule.table, rule.rule, rule.source
            )));
        }
    }
    for proof in reads().filter(|p| p.family == family.name && p.access != "upstream") {
        let entry = readset
            .tables
            .iter()
            .find(|t| format!("{:?}", t.selected.table) == proof.table)
            .filter(|t| t.mode != Mode::None)
            .ok_or_else(|| {
                invalid(format!(
                    "{}: {} at {} has no spool read-set entry",
                    family.name, proof.table, proof.source
                ))
            })?;
        if proof.all_fields && entry.selected.fields.len() != entry.selected.table.schema().len() {
            return Err(invalid(format!(
                "{}: incomplete snapshot at {}",
                family.name, proof.source
            )));
        }
        for field in proof.fields {
            if !entry
                .selected
                .fields
                .contains(&columns::field(entry.selected.table, field)?)
            {
                return Err(invalid(format!(
                    "{}: {}.{} at {} has no spool read-set field",
                    family.name, proof.table, field, proof.source
                )));
            }
        }
    }
    Ok(())
}

pub fn reads() -> impl Iterator<Item = &'static Proof> {
    READS.iter()
}

#[derive(Serialize)]
pub struct ReadRule {
    pub family: &'static str,
    pub table: &'static str,
    pub rule: &'static str,
    pub source: &'static str,
}

pub const READ_RULES: &[ReadRule] = &[
    ReadRule {
        family: "construction",
        table: "BuildingConstruction",
        rule: "supplied",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:608,611,614",
    },
    ReadRule {
        family: "construction",
        table: "RailwayConstruction",
        rule: "supplied",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:608,611,614",
    },
    ReadRule {
        family: "construction",
        table: "ConstructionLineOutput",
        rule: "supplied",
        source: "crates/dashboard-construction/src/products/construction_log/facts.rs:608,611,614",
    },
    ReadRule {
        family: "division-equipment",
        table: "DivisionEquipment",
        rule: "supplied",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:477",
    },
    ReadRule {
        family: "division-equipment",
        table: "Stockpiles",
        rule: "supplied",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:477",
    },
    ReadRule {
        family: "division-equipment",
        table: "VariantDefinitions",
        rule: "supplied",
        source: "crates/dashboard-army/src/products/division_equipment/history/facts.rs:477",
    },
    ReadRule {
        family: "equipment_losses",
        table: "SunkConvoys",
        rule: "supplied",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:976",
    },
    ReadRule {
        family: "equipment_losses",
        table: "LendLeaseEquipment",
        rule: "supplied",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:981",
    },
    ReadRule {
        family: "equipment_losses",
        table: "SliceReads",
        rule: "acquired",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:998",
    },
    ReadRule {
        family: "equipment_losses",
        table: "VariantDefinitions",
        rule: "supplied",
        source: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs:1018",
    },
    ReadRule {
        family: "event_feed",
        table: "Countries",
        rule: "acquired",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:529",
    },
    ReadRule {
        family: "event_feed",
        table: "CountryLinks",
        rule: "acquired",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:529",
    },
    ReadRule {
        family: "event_feed",
        table: "CountryExperienceGains",
        rule: "acquired",
        source: "crates/dashboard/src/products/event_feed/columnar.rs:529",
    },
    ReadRule {
        family: "identities",
        table: "Countries",
        rule: "acquired",
        source: "crates/dashboard/src/prepared/driver/native/identities.rs:169",
    },
    ReadRule {
        family: "identities",
        table: "CountryLinks",
        rule: "acquired",
        source: "crates/dashboard/src/prepared/driver/native/identities.rs:169",
    },
    ReadRule {
        family: "identities",
        table: "CountryExperienceGains",
        rule: "acquired",
        source: "crates/dashboard/src/prepared/driver/native/identities.rs:169",
    },
    ReadRule {
        family: "production",
        table: "ProductionLineOutput",
        rule: "supplied",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:448",
    },
    ReadRule {
        family: "production",
        table: "RocketProductionOutput",
        rule: "supplied",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:448",
    },
    ReadRule {
        family: "production",
        table: "VariantDefinitions",
        rule: "supplied",
        source: "crates/dashboard-base/src/prepared_production/columnar.rs:448",
    },
    ReadRule {
        family: "projects",
        table: "Projects",
        rule: "supplied",
        source: "crates/dashboard-special-projects/src/products/special_projects/facts.rs:1325",
    },
];

struct Source {
    family: &'static str,
    path: &'static str,
    code: &'static str,
}

const SOURCES: &[Source] = &[
    Source {
        family: "construction",
        path: "crates/dashboard-construction/src/products/construction_log/facts.rs",
        code: include_str!("../../dashboard-construction/src/products/construction_log/facts.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-army/src/products/division_equipment/history/facts.rs",
        code: include_str!("../../dashboard-army/src/products/division_equipment/history/facts.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-world/src/divisions.rs",
        code: include_str!("../../dashboard-world/src/divisions.rs"),
    },
    Source {
        family: "equipment_losses",
        path: "crates/dashboard-equipment-losses/src/products/equipment_losses/columnar.rs",
        code: include_str!(
            "../../dashboard-equipment-losses/src/products/equipment_losses/columnar.rs"
        ),
    },
    Source {
        family: "equipment_losses",
        path: "crates/dashboard-base/src/products/equipment/columnar.rs",
        code: include_str!("../../dashboard-base/src/products/equipment/columnar.rs"),
    },
    Source {
        family: "event_feed",
        path: "crates/dashboard/src/products/event_feed/columnar.rs",
        code: include_str!("../../dashboard/src/products/event_feed/columnar.rs"),
    },
    Source {
        family: "event_feed",
        path: "crates/dashboard/src/products/event_feed/diplomacy.rs",
        code: include_str!("../../dashboard/src/products/event_feed/diplomacy.rs"),
    },
    Source {
        family: "identities",
        path: "crates/dashboard/src/prepared/driver/native/identities.rs",
        code: include_str!("../../dashboard/src/prepared/driver/native/identities.rs"),
    },
    Source {
        family: "politics_ledger",
        path: "crates/dashboard-politics-ledger/src/products/politics_ledger/facts.rs",
        code: include_str!("../../dashboard-politics-ledger/src/products/politics_ledger/facts.rs"),
    },
    Source {
        family: "politics_ledger",
        path: "crates/dashboard-politics-ledger/src/products/politics_ledger/items.rs",
        code: include_str!("../../dashboard-politics-ledger/src/products/politics_ledger/items.rs"),
    },
    Source {
        family: "production",
        path: "crates/dashboard-base/src/prepared_production/columnar.rs",
        code: include_str!("../../dashboard-base/src/prepared_production/columnar.rs"),
    },
    Source {
        family: "production",
        path: "crates/dashboard-base/src/products/equipment/columnar.rs",
        code: include_str!("../../dashboard-base/src/products/equipment/columnar.rs"),
    },
    Source {
        family: "projects",
        path: "crates/dashboard-special-projects/src/products/special_projects/facts.rs",
        code: include_str!(
            "../../dashboard-special-projects/src/products/special_projects/facts.rs"
        ),
    },
    Source {
        family: "construction",
        path: "crates/dashboard-world/src/countries.rs",
        code: include_str!("../../dashboard-world/src/countries.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-army/src/products/army_content.rs",
        code: include_str!("../../dashboard-army/src/products/army_content.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-army/src/products/division_equipment/history/engine.rs",
        code: include_str!(
            "../../dashboard-army/src/products/division_equipment/history/engine.rs"
        ),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-army/src/products/division_needs.rs",
        code: include_str!("../../dashboard-army/src/products/division_needs.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-base/src/products/equipment/columnar.rs",
        code: include_str!("../../dashboard-base/src/products/equipment/columnar.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-text/src/subunit_requirements.rs",
        code: include_str!("../../dashboard-text/src/subunit_requirements.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-text/src/table_access.rs",
        code: include_str!("../../dashboard-text/src/table_access.rs"),
    },
    Source {
        family: "division-equipment",
        path: "crates/dashboard-text/src/template_facts.rs",
        code: include_str!("../../dashboard-text/src/template_facts.rs"),
    },
    Source {
        family: "event_feed",
        path: "crates/dashboard-base/src/products/country_history.rs",
        code: include_str!("../../dashboard-base/src/products/country_history.rs"),
    },
    Source {
        family: "event_feed",
        path: "crates/dashboard-base/src/products/country_history/native.rs",
        code: include_str!("../../dashboard-base/src/products/country_history/native.rs"),
    },
    Source {
        family: "identities",
        path: "crates/dashboard-base/src/products/country_history.rs",
        code: include_str!("../../dashboard-base/src/products/country_history.rs"),
    },
    Source {
        family: "identities",
        path: "crates/dashboard-base/src/products/country_history/native.rs",
        code: include_str!("../../dashboard-base/src/products/country_history/native.rs"),
    },
    Source {
        family: "politics_ledger",
        path: "crates/dashboard-base/src/products/idea_format.rs",
        code: include_str!("../../dashboard-base/src/products/idea_format.rs"),
    },
    Source {
        family: "politics_ledger",
        path: "crates/dashboard-text/src/identity.rs",
        code: include_str!("../../dashboard-text/src/identity.rs"),
    },
    Source {
        family: "politics_ledger",
        path: "crates/dashboard-world/src/naming.rs",
        code: include_str!("../../dashboard-world/src/naming.rs"),
    },
    Source {
        family: "politics_ledger",
        path: "crates/installed-content-country-names/src/country_names.rs",
        code: include_str!("../../installed-content-country-names/src/country_names.rs"),
    },
    Source {
        family: "projects",
        path: "crates/dashboard-special-projects/src/products/special_projects.rs",
        code: include_str!("../../dashboard-special-projects/src/products/special_projects.rs"),
    },
    Source {
        family: "projects",
        path: "crates/dashboard-special-projects/src/products/special_projects/emit.rs",
        code: include_str!(
            "../../dashboard-special-projects/src/products/special_projects/emit.rs"
        ),
    },
    Source {
        family: "equipment_losses",
        path: "crates/dashboard-world/src/naming.rs",
        code: include_str!("../../dashboard-world/src/naming.rs"),
    },
    Source {
        family: "production",
        path: "crates/dashboard-world/src/naming.rs",
        code: include_str!("../../dashboard-world/src/naming.rs"),
    },
];
