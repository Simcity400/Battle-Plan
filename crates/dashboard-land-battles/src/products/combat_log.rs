//! Retained ledger and complete observer detail tables at a selected hour.
use super::land_battles::LandBattleCountryFilter;
use dashboard_base::{
    catalogue,
    prepared::query::progressive,
    products::{Product, ReplyWarnings, check_reply_budget, equipment},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::{LinkTags, check_cancel};
use replay::campaign::SeekResult;
use replay_data::rows::{tables, *};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;
pub struct CombatLog;
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CombatLogParams {
    pub branch_id: String,
    pub hour: i32,
    pub countries: LandBattleCountryFilter,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogGroup {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogManpower {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
    pub reason: i32,
    pub date: i32,
    pub losses: [i32; 3],
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogEquipment {
    pub tag: String,
    pub group: String,
    pub month: u32,
    pub ordinal: Option<u32>,
    pub reason: i32,
    pub date: i32,
    pub kind: u8,
    pub allow_zero: bool,
    pub name: Option<String>,
    pub variant: Option<String>,
    pub amount_raw: Option<String>,
    pub missing_source: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogTemplate {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
    pub date: i32,
    pub template: String,
    pub name: Option<String>,
    pub wins: i32,
    pub total: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogBattle {
    pub tag: String,
    pub group: String,
    pub ordinal: u32,
    pub id: i32,
    pub attacker: bool,
    pub date: i32,
    pub province: u32,
    pub flags: [u8; 4],
    pub manpower: [i32; 2],
    pub references: i32,
    pub details: Vec<LogDetailTable>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogDetailTable {
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogActiveSide {
    pub combat: String,
    pub attacker: bool,
    pub terminal: bool,
    pub progress_raw: String,
    pub damage_raw: String,
    pub activity_groups: usize,
    pub leaders: usize,
    pub loss_source: Option<String>,
    pub details: Vec<LogDetailTable>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CombatLogAvailability {
    pub groups: Option<String>,
    pub manpower: Option<String>,
    pub months: Option<String>,
    pub own_equipment: Option<String>,
    pub enemy_equipment: Option<String>,
    pub templates: Option<String>,
    pub battles: Option<String>,
    pub active: Option<String>,
    pub terminal: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CombatLogReply {
    pub hour: i32,
    pub groups: Vec<LogGroup>,
    pub manpower: Vec<LogManpower>,
    pub equipment: Vec<LogEquipment>,
    pub templates: Vec<LogTemplate>,
    pub battles: Vec<LogBattle>,
    pub active: Vec<LogActiveSide>,
    pub availability: CombatLogAvailability,
    pub warnings: Vec<String>,
}
impl ReplyWarnings for CombatLogReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}
fn unavailable(frame: &DecodedFrame, sources: &[TableId]) -> Option<String> {
    let missing: Vec<_> = sources
        .iter()
        .filter_map(|id| match &*frame.tables[id.index()] {
            TableView::Complete(_) => None,
            TableView::Unavailable { reason } => Some(format!("{id:?}: {reason}")),
            TableView::NotRecorded { mode } => {
                Some(format!("{id:?}: not recorded in a {mode} recording"))
            }
            TableView::Truncated { .. } => Some(format!("{id:?}: incomplete read")),
        })
        .collect();
    (!missing.is_empty()).then(|| missing.join("; "))
}
fn matches_country(filter: &LandBattleCountryFilter, tag: &str, hour: i32) -> bool {
    filter
        .tags
        .as_ref()
        .is_none_or(|tags| tags.iter().any(|t| t == tag))
        && (filter.membership.is_none()
            || !filter
                .unobserved
                .iter()
                .any(|s| s.first_hour <= hour && hour <= s.last_hour)
                && filter.membership.as_ref().is_some_and(|runs| {
                    runs.iter()
                        .any(|r| r.tag == tag && r.first_hour <= hour && hour <= r.last_hour)
                }))
}
fn matches_observer(
    countries: impl Iterator<Item = u32>,
    links: &LinkTags,
    filter: &LandBattleCountryFilter,
    hour: i32,
) -> bool {
    // As in the battle rankings, unknown participants cannot exclude a side.
    let mut empty = true;
    for country in countries {
        empty = false;
        if links
            .get(country)
            .is_none_or(|tag| matches_country(filter, tag, hour))
        {
            return true;
        }
    }
    empty
}
fn detail(
    tables: &mut BTreeMap<&'static str, LogDetailTable>,
    title: &'static str,
    columns: &[&str],
    row: Vec<String>,
) {
    tables
        .entry(title)
        .or_insert_with(|| LogDetailTable {
            title: title.into(),
            columns: columns.iter().map(|s| (*s).into()).collect(),
            rows: Vec::new(),
        })
        .rows
        .push(row);
}
fn fixed(raw: i64) -> String {
    let v = raw.unsigned_abs();
    let fraction = format!("{:05}", v % 100_000);
    let fraction = fraction.trim_end_matches('0');
    format!(
        "{}{}{}{}",
        if raw < 0 { "-" } else { "" },
        v / 100_000,
        if fraction.is_empty() { "" } else { "." },
        fraction
    )
}
struct ObserverLabels<'a> {
    country: &'a mut dyn FnMut(u32) -> String,
    equipment: &'a mut dyn FnMut(u64) -> String,
    template: &'a mut dyn FnMut(u64) -> String,
}
fn observer(
    combat: u64,
    attacker: bool,
    terminal: bool,
    facts: impl IntoIterator<Item = (u8, u64, [u64; 5])>,
    losses: Result<(i32, &[LandEquipmentLoss]), String>,
    labels: &mut ObserverLabels<'_>,
) -> io::Result<LogActiveSide> {
    use CombatLogObserverKind as K;
    let ObserverLabels {
        country,
        equipment,
        template,
    } = labels;
    let mut result = LogActiveSide {
        combat: combat.to_string(),
        attacker,
        terminal,
        progress_raw: "0".into(),
        damage_raw: "0".into(),
        activity_groups: 0,
        leaders: 0,
        loss_source: None,
        details: Vec::new(),
    };
    let mut tables = BTreeMap::new();
    let mut state = false;
    let mut participants = false;
    for (kind, item, [a, b, c, d, e]) in facts {
        let ordinal = item.to_string();
        match kind {
            k if k == K::State as u8 => {
                state = true;
                result.damage_raw = (a as i64).to_string();
                result.progress_raw = (b as i64).to_string();
                detail(
                    &mut tables,
                    "Observer state",
                    &["Damage", "Progress", "Flags", "Snow", "Win"],
                    vec![
                        fixed(a as i64),
                        fixed(b as i64),
                        c.to_string(),
                        (c & 1 != 0).to_string(),
                        (c & 2 != 0).to_string(),
                    ],
                );
            }
            k if k == K::Side as u8 => detail(
                &mut tables,
                "Side",
                &["Air factor", "Leader identity"],
                vec![fixed(b as i64), c.to_string()],
            ),
            k if k == K::Pool as u8 => detail(
                &mut tables,
                "Equipment pool membership",
                &["Pool", "Present in save", "Allow zero entries"],
                vec![pool(item), (a != 0).to_string(), (b != 0).to_string()],
            ),
            k if k == K::Equipment as u8 => detail(
                &mut tables,
                "Side equipment",
                &["Pool", "Order", "Design / identity", "Amount"],
                vec![pool(item), a.to_string(), equipment(b), fixed(c as i64)],
            ),
            k if k == K::Modifier as u8 => detail(
                &mut tables,
                "Modifier hours",
                &["Order", "Modifier index", "Hours"],
                vec![
                    a.to_string(),
                    (b as i32).to_string(),
                    (c as i32).to_string(),
                ],
            ),
            k if k == K::Country as u8 => {
                participants = true;
                detail(
                    &mut tables,
                    "Countries",
                    &["Order", "Country / identity"],
                    vec![a.to_string(), country(b as u32)],
                );
            }
            k if k == K::Counters as u8 => {
                for (i, v) in [a, b, c, d, e].into_iter().enumerate() {
                    detail(
                        &mut tables,
                        "Stored counters",
                        &["Index", "Hours"],
                        vec![(item + i as u64).to_string(), (v as i32).to_string()],
                    );
                }
            }
            k if k == K::Leader as u8 => {
                result.leaders += 1;
                detail(
                    &mut tables,
                    "Leader hours",
                    &["Order", "Leader identity", "Hours"],
                    vec![ordinal, a.to_string(), (b as i32).to_string()],
                );
            }
            k if k == K::Damage as u8 => detail(
                &mut tables,
                "Damage attribution",
                &["Order", "From", "Receiver", "To", "Damage"],
                vec![
                    ordinal,
                    country(a as u32),
                    country(b as u32),
                    country(c as u32),
                    fixed(d as i64),
                ],
            ),
            k if k == K::Activity as u8 => {
                result.activity_groups += 1;
                detail(
                    &mut tables,
                    "Activity groups",
                    &["Order", "Group identity", "Dealer", "Taker"],
                    vec![ordinal, a.to_string(), country(b as u32), country(c as u32)],
                );
            }
            k if k == K::ActivityTemplate as u8 => detail(
                &mut tables,
                "Activity templates",
                &["Activity", "Order", "Template / identity"],
                vec![ordinal, a.to_string(), template(b)],
            ),
            k if k == K::ActivityDamage as u8 => detail(
                &mut tables,
                "Activity division damage",
                &["Activity", "Order", "Division identity", "Strength damage"],
                vec![ordinal, a.to_string(), b.to_string(), fixed(c as i64)],
            ),
            k if k == K::ActivityEquipment as u8 => {
                if a == u64::MAX {
                    detail(
                        &mut tables,
                        "Activity equipment membership",
                        &["Activity", "Present in save", "Allow zero entries"],
                        vec![ordinal, (b != 0).to_string(), (c != 0).to_string()],
                    );
                } else {
                    detail(
                        &mut tables,
                        "Activity damaged equipment",
                        &["Activity", "Order", "Design / identity", "Amount"],
                        vec![ordinal, a.to_string(), equipment(b), fixed(c as i64)],
                    );
                }
            }
            _ => {
                return Err(io::Error::other(format!(
                    "Unsupported Combat Log observer kind {kind}"
                )));
            }
        }
    }
    if !state {
        return Err(io::Error::other("Combat Log observer state missing"));
    }
    if !participants {
        detail(
            &mut tables,
            "Countries",
            &["Order", "Country / identity"],
            vec![String::new(), "Unknown participants".into()],
        );
    }
    match losses {
        Ok((manpower, entries)) => {
            detail(
                &mut tables,
                "Aggregate losses",
                &["Manpower"],
                vec![manpower.to_string()],
            );
            for (ordinal, loss) in entries.iter().enumerate() {
                detail(
                    &mut tables,
                    "Aggregate equipment losses",
                    &["Order", "Design / identity", "Amount"],
                    vec![
                        ordinal.to_string(),
                        equipment(loss.variant),
                        fixed(loss.amount_raw),
                    ],
                );
            }
        }
        Err(reason) => result.loss_source = Some(reason),
    }
    result.details = tables.into_values().collect();
    Ok(result)
}
fn pool(kind: u64) -> String {
    match kind {
        0 => "Lost".into(),
        1 => "Captured by enemy".into(),
        2 => "Recovered".into(),
        _ => format!("Pool {kind}"),
    }
}
impl Product for CombatLog {
    const KEY: &'static str = "combat-log-history";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = CombatLogParams;
    type Reply = CombatLogReply;
    fn types() -> Vec<String> {
        vec![
            CombatLogParams::decl(),
            CombatLogReply::decl(),
            LogGroup::decl(),
            LogManpower::decl(),
            LogEquipment::decl(),
            LogTemplate::decl(),
            LogBattle::decl(),
            LogDetailTable::decl(),
            LogActiveSide::decl(),
            CombatLogAvailability::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        if let Some(tags) = &p.countries.tags {
            for tag in tags {
                dashboard_base::products::validate_tag(tag)?;
            }
        }
        if let Some(runs) = &p.countries.membership {
            for r in runs {
                dashboard_base::products::validate_tag(&r.tag)?;
            }
        }
        Ok(())
    }
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = p
            .branch_id
            .parse::<u64>()
            .map_err(|_| io::Error::other("Invalid branch"))?;
        let mut selected = vec![
            TableId::CombatLogGroups,
            TableId::CombatLogMonths,
            TableId::CombatLogManpower,
            TableId::CombatLogTemplates,
            TableId::CombatLogReferences,
            TableId::CombatLogBattles,
            TableId::CombatLogEnemyEquipment,
            TableId::CombatLogBattleSides,
            TableId::CombatLogBattleEquipment,
            TableId::CombatLogBattleModifiers,
            TableId::CombatLogBattleCountries,
            TableId::CombatLogObservers,
            TableId::LandCombatProgress,
            TableId::LandCombatResults,
            TableId::DivisionTemplates,
            TableId::DivisionTemplateIcons,
            TableId::CombatLogEquipment,
        ];
        selected.extend([TableId::Countries, TableId::CountryLinks]);
        let frame =
            match source
                .replay_ref()
                .seek_selected_with(branch, p.hour, Some(&selected), || check_cancel(cancelled))?
            {
                SeekResult::Frame(f) => f,
                SeekResult::Gap { reason, .. } => return Err(io::Error::other(reason)),
            };
        let mut links = LinkTags::default();
        links.update(&frame)?;
        let mut warnings = Vec::new();
        let cat = context.content.as_ref().and_then(|options| {
            match catalogue::catalogue(source, options) {
                Ok(cat) => Some(cat),
                Err(why) => {
                    warnings.push(format!("Combat Log labels unavailable: {why}"));
                    None
                }
            }
        });
        // Reuse the shared supplying-read lookup and frame cache. Naming operands
        // belong to that read, including its country appearance and lexer; an
        // off-slot frame alone cannot seed a naming history.
        let naming = progressive::latest_read(
            source,
            branch,
            frame.hour,
            TableId::VariantDefinitions,
            replay_data::cadence::STAGGER_PERIOD,
            &equipment::VariantCatalogue::TABLES,
            cancelled,
        )?;
        let naming = naming.as_deref().and_then(|read| {
            Some((
                read,
                equipment::Tags::new(
                    read.complete::<tables::Countries>().ok()?,
                    read.complete::<tables::CountryLinks>().ok()?,
                    read.complete::<tables::CountryAppearance>().ok(),
                ),
                read.complete::<tables::VariantDefinitions>().ok()?,
                read.complete::<tables::LexicalTokens>().ok()?,
            ))
        });
        let mut missing = BTreeSet::new();
        let mut name = |variant: u64| {
            let value = naming
                .as_ref()
                .and_then(|(read, tags, definitions, lexical)| {
                    let at = definitions.binary_search_by_key(&variant, |r| r.id).ok()?;
                    equipment::variant_names(
                        &equipment::raw_variant(&definitions[at], read.hour, tags, lexical),
                        cat.as_deref().map(|c| c as &dyn equipment::Texts),
                    )
                    .0
                });
            if value.is_none() {
                missing.insert(variant);
            }
            value
        };
        let mut equipment_label = |variant: u64| {
            name(variant).map_or_else(
                || format!("Name unavailable ({variant})"),
                |n| format!("{n} ({variant})"),
            )
        };
        let owner = |country: u32| {
            links.get(country).map_or_else(
                || p.countries.tags.is_none() && p.countries.membership.is_none(),
                |tag| matches_country(&p.countries, tag, frame.hour),
            )
        };
        let tag = |country: u32| {
            links
                .get(country)
                .cloned()
                .unwrap_or_else(|| format!("Country {country}"))
        };
        let mut unresolved = BTreeSet::new();
        let mut country = |id: u32| {
            if links.get(id).is_none() {
                unresolved.insert(id);
            }
            format!("{} ({id})", tag(id))
        };
        macro_rules! rows {
            ($t:ident) => {
                frame.complete::<tables::$t>().unwrap_or(&[])
            };
        }
        let availability = CombatLogAvailability {
            groups: unavailable(&frame, &[TableId::CombatLogGroups]),
            manpower: unavailable(&frame, &[TableId::CombatLogManpower]),
            months: unavailable(&frame, &[TableId::CombatLogMonths]),
            own_equipment: unavailable(&frame, &[TableId::CombatLogEquipment]),
            enemy_equipment: unavailable(&frame, &[TableId::CombatLogEnemyEquipment]),
            templates: unavailable(&frame, &[TableId::CombatLogTemplates]),
            battles: unavailable(
                &frame,
                &[
                    TableId::CombatLogReferences,
                    TableId::CombatLogBattles,
                    TableId::CombatLogBattleSides,
                    TableId::CombatLogBattleEquipment,
                    TableId::CombatLogBattleModifiers,
                    TableId::CombatLogBattleCountries,
                ],
            ),
            active: unavailable(&frame, &[TableId::CombatLogObservers]),
            terminal: unavailable(&frame, &[TableId::LandCombatResults]),
        };
        let mut reply = CombatLogReply {
            hour: frame.hour,
            groups: Vec::new(),
            manpower: Vec::new(),
            equipment: Vec::new(),
            templates: Vec::new(),
            battles: Vec::new(),
            active: Vec::new(),
            availability,
            warnings: Vec::new(),
        };
        for g in rows!(CombatLogGroups) {
            let label = country(g.country);
            if owner(g.country) {
                reply.groups.push(LogGroup {
                    tag: label,
                    group: g.group.to_string(),
                    ordinal: g.ordinal,
                });
            }
        }
        for r in rows!(CombatLogManpower).iter().filter(|r| owner(r.country)) {
            reply.manpower.push(LogManpower {
                tag: tag(r.country),
                group: r.group.to_string(),
                ordinal: r.ordinal,
                reason: r.reason,
                date: r.date,
                losses: [r.losses_0, r.losses_1, r.losses_2],
            });
        }
        // Each detail table is indexed once by its join key, in table order.
        let mut enemy_equipment = BTreeMap::<_, Vec<_>>::new();
        for e in rows!(CombatLogEnemyEquipment) {
            check_cancel(cancelled)?;
            enemy_equipment
                .entry((e.country, e.group, e.month))
                .or_default()
                .push(e);
        }
        let mut own_equipment = BTreeMap::<_, Vec<_>>::new();
        for e in rows!(CombatLogEquipment) {
            check_cancel(cancelled)?;
            own_equipment
                .entry((e.country, e.group, e.kind, e.date))
                .or_default()
                .push(e);
        }
        for m in rows!(CombatLogMonths).iter().filter(|r| owner(r.country)) {
            check_cancel(cancelled)?;
            let before = reply.equipment.len();
            let missing_source = if m.kind == 5 {
                reply.availability.enemy_equipment.clone()
            } else {
                reply.availability.own_equipment.clone()
            };
            let mut add = |ordinal, variant: Option<u64>, amount: Option<i64>| {
                reply.equipment.push(LogEquipment {
                    tag: tag(m.country),
                    group: m.group.to_string(),
                    month: m.ordinal,
                    ordinal,
                    reason: m.reason,
                    date: m.date,
                    kind: m.kind,
                    allow_zero: m.allow_zero,
                    name: variant.map(&mut equipment_label),
                    variant: variant.map(|v| v.to_string()),
                    amount_raw: amount.map(|v| v.to_string()),
                    missing_source: missing_source.clone(),
                })
            };
            if m.kind == 5 {
                for e in enemy_equipment
                    .get(&(m.country, m.group, m.ordinal))
                    .into_iter()
                    .flatten()
                {
                    check_cancel(cancelled)?;
                    add(Some(e.ordinal), Some(e.variant), Some(e.amount_raw));
                }
            } else {
                for (ordinal, e) in own_equipment
                    .get(&(m.country, m.group, m.kind, m.date))
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    check_cancel(cancelled)?;
                    add(
                        Some(ordinal as u32),
                        Some((e.variant as u64) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE)),
                        Some(e.amount_raw),
                    );
                }
            }
            if before == reply.equipment.len() {
                reply.equipment.push(LogEquipment {
                    tag: tag(m.country),
                    group: m.group.to_string(),
                    month: m.ordinal,
                    ordinal: None,
                    reason: m.reason,
                    date: m.date,
                    kind: m.kind,
                    allow_zero: m.allow_zero,
                    name: None,
                    variant: None,
                    amount_raw: None,
                    missing_source,
                });
            }
        }
        // Templates are named as the template views name them at this hour:
        // the current display name, localised for its composition country.
        let mut template_names = BTreeMap::new();
        let mut missing_templates = BTreeSet::new();
        let mut template_name = |id: u64| -> Option<String> {
            let name = template_names
                .entry(id)
                .or_insert_with(|| {
                    use dashboard_text::template_facts::{TemplateNaming, display_naming};
                    let name = match (display_naming(&*frame, id).ok()?, cat.as_ref()) {
                        (TemplateNaming::Text(text), _) => text,
                        (naming, Some(cat)) => naming.localise(&cat.localisation).ok()?,
                        (TemplateNaming::Localised { .. }, None) => return None,
                    };
                    catalogue::plain_text(&name).ok()
                })
                .clone();
            if name.is_none() {
                missing_templates.insert(id);
            }
            name
        };
        for t in rows!(CombatLogTemplates)
            .iter()
            .filter(|r| owner(r.country))
        {
            check_cancel(cancelled)?;
            let label = template_name(t.template);
            reply.templates.push(LogTemplate {
                tag: tag(t.country),
                group: t.group.to_string(),
                ordinal: t.ordinal,
                date: t.date,
                template: t.template.to_string(),
                name: label,
                wins: t.wins,
                total: t.total,
            });
        }
        let mut template = |id: u64| {
            template_name(id).map_or_else(
                || format!("Name unavailable ({id})"),
                |n| format!("{n} ({id})"),
            )
        };
        if reply.availability.battles.is_none() {
            let mut battles = BTreeMap::new();
            for b in rows!(CombatLogBattles) {
                check_cancel(cancelled)?;
                battles.entry(b.id).or_insert(b);
            }
            let mut sides = BTreeMap::<_, Vec<_>>::new();
            for s in rows!(CombatLogBattleSides) {
                check_cancel(cancelled)?;
                sides.entry(s.battle).or_default().push(s);
            }
            let mut battle_equipment = BTreeMap::<_, Vec<_>>::new();
            for e in rows!(CombatLogBattleEquipment) {
                check_cancel(cancelled)?;
                battle_equipment.entry(e.battle).or_default().push(e);
            }
            let mut modifiers = BTreeMap::<_, Vec<_>>::new();
            for m in rows!(CombatLogBattleModifiers) {
                check_cancel(cancelled)?;
                modifiers.entry(m.battle).or_default().push(m);
            }
            let mut countries = BTreeMap::<_, Vec<_>>::new();
            for c in rows!(CombatLogBattleCountries) {
                check_cancel(cancelled)?;
                countries.entry(c.battle).or_default().push(c);
            }
            for r in rows!(CombatLogReferences)
                .iter()
                .filter(|r| owner(r.country))
            {
                check_cancel(cancelled)?;
                let b = battles.get(&(r.battle as u32)).copied().ok_or_else(|| {
                    io::Error::other("CombatLogReferences: retained record missing")
                })?;
                let mut details = BTreeMap::new();
                detail(
                    &mut details,
                    "Flags",
                    &[
                        "Defender won",
                        "Snow",
                        "Overrun",
                        "Player is attacker",
                        "Stored bytes",
                    ],
                    vec![
                        (b.flags[0] != 0).to_string(),
                        (b.flags[1] != 0).to_string(),
                        (b.flags[2] != 0).to_string(),
                        (b.flags[3] != 0).to_string(),
                        format!("{:?}", b.flags),
                    ],
                );
                let mut manpower = [0; 2];
                for s in sides.get(&b.id).into_iter().flatten() {
                    if s.side > 1 {
                        return Err(io::Error::other("CombatLogBattleSides: invalid side"));
                    }
                    manpower[s.side as usize] = s.manpower;
                    detail(
                        &mut details,
                        "Sides",
                        &[
                            "Side",
                            "Manpower lost",
                            "Air factor",
                            "Leader identity",
                            "Pool flags",
                        ],
                        vec![
                            side(s.side == 0).into(),
                            s.manpower.to_string(),
                            fixed(s.air_factor_raw),
                            s.leader.to_string(),
                            s.pool_flags.to_string(),
                        ],
                    );
                    for kind in 0..3 {
                        let flags = (s.pool_flags >> (kind * 2)) & 3;
                        detail(
                            &mut details,
                            "Equipment pool membership",
                            &["Side", "Pool", "Present in save", "Allow zero entries"],
                            vec![
                                side(s.side == 0).into(),
                                pool(kind as u64),
                                (flags & 1 != 0).to_string(),
                                (flags & 2 != 0).to_string(),
                            ],
                        );
                    }
                }
                for e in battle_equipment.get(&b.id).into_iter().flatten() {
                    check_cancel(cancelled)?;
                    detail(
                        &mut details,
                        "Equipment",
                        &["Side", "Pool", "Order", "Design / identity", "Amount"],
                        vec![
                            side(e.side == 0).into(),
                            pool(e.kind as u64),
                            e.ordinal.to_string(),
                            equipment_label(e.variant),
                            fixed(e.amount_raw),
                        ],
                    );
                }
                for m in modifiers.get(&b.id).into_iter().flatten() {
                    detail(
                        &mut details,
                        "Modifier hours",
                        &["Side", "Order", "Modifier index", "Hours"],
                        vec![
                            side(m.side == 0).into(),
                            m.ordinal.to_string(),
                            m.modifier.to_string(),
                            m.hours.to_string(),
                        ],
                    );
                }
                for c in countries.get(&b.id).into_iter().flatten() {
                    detail(
                        &mut details,
                        "Countries",
                        &["Side", "Order", "Country / identity"],
                        vec![
                            side(c.side == 0).into(),
                            c.ordinal.to_string(),
                            country(c.country),
                        ],
                    );
                }
                reply.battles.push(LogBattle {
                    tag: tag(r.country),
                    group: r.group.to_string(),
                    ordinal: r.ordinal,
                    id: r.battle,
                    attacker: r.attacker,
                    date: b.date,
                    province: b.province,
                    flags: b.flags,
                    manpower,
                    references: b.reference_count,
                    details: details.into_values().collect(),
                });
            }
        }
        let mut active: BTreeMap<(u64, bool), Vec<_>> = BTreeMap::new();
        for r in rows!(CombatLogObservers) {
            check_cancel(cancelled)?;
            active.entry((r.combat, r.attacker)).or_default().push(r);
        }
        let progress = frame
            .complete::<tables::LandCombatProgress>()
            .map_err(|e| format!("LandCombatProgress: {e}"))
            .map(|rows| {
                let mut by_combat = BTreeMap::new();
                for row in rows {
                    by_combat.entry(row.combat).or_insert(row);
                }
                by_combat
            });
        for ((combat, attacker), mut operands) in active {
            check_cancel(cancelled)?;
            operands.sort_unstable_by_key(|row| row.logical_order());
            if !matches_observer(
                operands
                    .iter()
                    .filter(|r| r.kind == CombatLogObserverKind::Country as u8)
                    .map(|r| r.identity as u32),
                &links,
                &p.countries,
                frame.hour,
            ) {
                continue;
            }
            let losses = progress.as_ref().map_err(Clone::clone).and_then(|rows| {
                let progress = rows
                    .get(&combat)
                    .copied()
                    .ok_or_else(|| "LandCombatProgress: observer combat missing".to_string())?;
                let mp = if attacker {
                    &progress.attacker_manpower_loss
                } else {
                    &progress.defender_manpower_loss
                };
                let equipment = if attacker {
                    &progress.attacker_equipment_losses
                } else {
                    &progress.defender_equipment_losses
                };
                Ok((
                    *mp.require("manpower")
                        .map_err(|e| format!("LandCombatProgress: {e}"))?,
                    equipment
                        .require("equipment")
                        .map_err(|e| format!("LandCombatProgress: {e}"))?
                        .as_slice(),
                ))
            });
            reply.active.push(observer(
                combat,
                attacker,
                false,
                operands.into_iter().map(|r| r.fact()),
                losses,
                &mut ObserverLabels {
                    country: &mut country,
                    equipment: &mut equipment_label,
                    template: &mut template,
                },
            )?);
        }
        for result in rows!(LandCombatResults) {
            check_cancel(cancelled)?;
            for attacker in [true, false] {
                let operands: Vec<_> = result
                    .observers
                    .iter()
                    .filter(|r| r.attacker == attacker)
                    .collect();
                if operands.is_empty()
                    || !matches_observer(
                        operands
                            .iter()
                            .filter(|r| r.kind == CombatLogObserverKind::Country as u8)
                            .map(|r| r.values[1] as u32),
                        &links,
                        &p.countries,
                        frame.hour,
                    )
                {
                    continue;
                }
                let loss = &result.losses[usize::from(!attacker)];
                reply.active.push(observer(
                    result.combat,
                    attacker,
                    true,
                    operands.into_iter().map(|r| (r.kind, r.item, r.values)),
                    Ok((loss.manpower, &loss.equipment)),
                    &mut ObserverLabels {
                        country: &mut country,
                        equipment: &mut equipment_label,
                        template: &mut template,
                    },
                )?);
            }
        }
        if !unresolved.is_empty() {
            warnings.push(format!(
                "{} Combat Log country references have no recorded country binding: {:?}",
                unresolved.len(),
                unresolved
            ));
        }
        if !missing.is_empty() {
            warnings.push(format!(
                "{} recorded equipment designs have no resolved label",
                missing.len()
            ));
        }
        if !missing_templates.is_empty() {
            warnings.push(format!(
                "{} retained or activity templates have no recorded name at this hour",
                missing_templates.len()
            ));
        }
        reply.warnings = warnings;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
fn side(attacker: bool) -> &'static str {
    if attacker { "Attacker" } else { "Defender" }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> DecodedFrame {
        let mut f = dashboard_base::test_fixtures::source(1, 60_000_001, None);
        for id in [
            TableId::CombatLogGroups,
            TableId::CombatLogMonths,
            TableId::CombatLogManpower,
            TableId::CombatLogTemplates,
            TableId::CombatLogReferences,
            TableId::CombatLogBattles,
            TableId::CombatLogEnemyEquipment,
            TableId::CombatLogBattleSides,
            TableId::CombatLogBattleEquipment,
            TableId::CombatLogBattleModifiers,
            TableId::CombatLogBattleCountries,
            TableId::CombatLogObservers,
            TableId::LandCombatProgress,
            TableId::LandCombatResults,
            TableId::DivisionTemplates,
            TableId::DivisionTemplateIcons,
            TableId::CombatLogEquipment,
        ] {
            *f.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
        *f.tables[TableId::CombatLogGroups.index()] =
            TableView::Complete(TableRows::CombatLogGroups(vec![CombatLogGroupRow {
                country: 11,
                group: 7 << 32 | 53,
                ordinal: 0,
            }]));
        *f.tables[TableId::CombatLogMonths.index()] =
            TableView::Complete(TableRows::CombatLogMonths(vec![CombatLogMonthRow {
                country: 11,
                group: 7,
                kind: 5,
                date: 60_000_000,
                allow_zero: true,
                ..Default::default()
            }]));
        *f.tables[TableId::CombatLogManpower.index()] =
            TableView::Complete(TableRows::CombatLogManpower(vec![CombatLogManpowerRow {
                country: 11,
                group: 7,
                losses_0: i32::MIN,
                losses_1: -7,
                losses_2: i32::MAX,
                ..Default::default()
            }]));
        *f.tables[TableId::CombatLogReferences.index()] =
            TableView::Complete(TableRows::CombatLogReferences(vec![
                CombatLogReferenceRow {
                    country: 11,
                    group: 7,
                    battle: 42,
                    attacker: true,
                    ..Default::default()
                },
            ]));
        *f.tables[TableId::CombatLogBattles.index()] =
            TableView::Complete(TableRows::CombatLogBattles(vec![CombatLogBattleRow {
                id: 42,
                reference_count: 1,
                flags: [1, 2, 3, 4],
                ..Default::default()
            }]));
        *f.tables[TableId::CombatLogBattleSides.index()] =
            TableView::Complete(TableRows::CombatLogBattleSides(vec![
                CombatLogBattleSideRow {
                    battle: 42,
                    side: 0,
                    manpower: -7,
                    air_factor_raw: i64::MIN,
                    leader: u64::MAX,
                    pool_flags: 63,
                },
                CombatLogBattleSideRow {
                    battle: 42,
                    side: 1,
                    ..Default::default()
                },
            ]));
        *f.tables[TableId::CombatLogBattleCountries.index()] =
            TableView::Complete(TableRows::CombatLogBattleCountries(vec![
                CombatLogBattleCountryRow {
                    battle: 42,
                    country: 11,
                    ..Default::default()
                },
            ]));
        *f.tables[TableId::LandCombatResults.index()] =
            TableView::Complete(TableRows::LandCombatResults(vec![LandCombatResultsRow {
                id: 1,
                combat: 71,
                observers: vec![
                    LandTerminalObserver {
                        attacker: true,
                        kind: CombatLogObserverKind::State as u8,
                        values: [123_456, 987_654, 3, 0, 0],
                        ..Default::default()
                    },
                    LandTerminalObserver {
                        attacker: true,
                        kind: CombatLogObserverKind::Country as u8,
                        values: [0, 11, 0, 0, 0],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }]));
        f
    }
    fn load(f: DecodedFrame) -> CombatLogReply {
        load_filtered(
            f,
            LandBattleCountryFilter {
                tags: Some(vec!["GER".into()]),
                membership: None,
                unobserved: vec![],
            },
        )
    }
    fn load_filtered(f: DecodedFrame, countries: LandBattleCountryFilter) -> CombatLogReply {
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "combat-log".into(),
                revision: "1".into(),
            },
            "combat-log.battleplan".into(),
            std::sync::Arc::new(dashboard_base::test_fixtures::replay(vec![f])),
            &AtomicBool::new(false),
        )
        .unwrap();
        CombatLog::load(
            &source,
            &CombatLogParams {
                branch_id: "1".into(),
                hour: 60_000_001,
                countries,
            },
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    #[test]
    fn combat_log_product_preserves_empty_months_signed_counters_and_terminal_sides() {
        let r = load(fixture());
        assert_eq!(r.equipment.len(), 1);
        assert!(r.equipment[0].amount_raw.is_none());
        assert!(r.equipment[0].allow_zero);
        assert_eq!(r.manpower[0].losses, [i32::MIN, -7, i32::MAX]);
        assert!(r.active[0].terminal);
        assert_eq!(r.active[0].damage_raw, "123456");
        let battle = serde_json::to_string(&r.battles[0]).unwrap();
        for expected in [
            "18446744073709551615",
            "-92233720368547.75808",
            "GER",
            "Pool flags",
            "Countries",
        ] {
            assert!(battle.contains(expected), "{expected}");
        }
        assert_eq!(r.battles[0].flags, [1, 2, 3, 4]);
        let contract = include_str!("../../../../ui/src/generated/dashboard.ts");
        for declaration in CombatLog::types() {
            assert!(
                contract.contains(&format!("export {declaration}")),
                "{declaration}"
            );
        }
    }
    #[test]
    fn combat_log_terminal_and_active_gaps_do_not_hide_complete_retained_history() {
        for reason in [
            "attachment interval",
            "load interval",
            "lost event interval",
        ] {
            let mut f = fixture();
            *f.tables[TableId::LandCombatResults.index()] = TableView::Unavailable {
                reason: reason.into(),
            };
            *f.tables[TableId::CombatLogObservers.index()] = TableView::Unavailable {
                reason: "observer census gap".into(),
            };
            let r = load(f);
            assert_eq!(r.manpower.len(), 1);
            assert_eq!(r.equipment.len(), 1);
            assert_eq!(r.battles.len(), 1);
            assert!(r.availability.manpower.is_none());
            assert!(r.availability.battles.is_none());
            assert!(
                r.availability
                    .terminal
                    .unwrap()
                    .contains(&format!("LandCombatResults: {reason}"))
            );
            assert!(
                r.availability
                    .active
                    .unwrap()
                    .contains("CombatLogObservers: observer census gap")
            );
        }
    }
    #[test]
    fn combat_log_observer_details_consume_every_operand_kind_and_exact_loss_values() {
        use CombatLogObserverKind as K;
        let facts = vec![
            (K::State as u8, 0, [123_456, 987_654, 7, 0, 0]),
            (K::Side as u8, 0, [0, 112_345, u64::MAX, 0, 0]),
            (K::Pool as u8, 1, [1, 1, 0, 0, 0]),
            (
                K::Equipment as u8,
                1,
                [0, 7 << 32 | 70, i64::MIN as u64, 0, 0],
            ),
            (K::Modifier as u8, 0, [0, 8, (-7i32 as u32) as u64, 0, 0]),
            (K::Country as u8, 0, [0, 11, 0, 0, 0]),
            (
                K::Counters as u8,
                25,
                [(i32::MIN as u32) as u64, 2, 3, 4, 5],
            ),
            (
                K::Leader as u8,
                0,
                [u64::MAX, (-9i32 as u32) as u64, 0, 0, 0],
            ),
            (K::Damage as u8, 0, [11, 12, 13, 12_345, 0]),
            (K::Activity as u8, 0, [7 << 32 | 53, 11, 12, 0, 0]),
            (K::ActivityTemplate as u8, 0, [0, 8 << 32 | 54, 0, 0, 0]),
            (K::ActivityDamage as u8, 0, [0, 9 << 32 | 51, 23_456, 0, 0]),
            (K::ActivityEquipment as u8, 0, [u64::MAX, 1, 1, 0, 0]),
            (
                K::ActivityEquipment as u8,
                0,
                [0, 7 << 32 | 70, 34_567, 0, 0],
            ),
        ];
        let losses = [LandEquipmentLoss {
            variant: 7 << 32 | 70,
            amount_raw: i64::MIN,
        }];
        let r = observer(
            71,
            true,
            false,
            facts,
            Ok((-17, &losses)),
            &mut ObserverLabels {
                country: &mut |id| format!("country {id}"),
                equipment: &mut |id| format!("equipment {id}"),
                template: &mut |id| format!("template {id}"),
            },
        )
        .unwrap();
        assert_eq!(r.details.len(), 16);
        assert_eq!(r.activity_groups, 1);
        assert_eq!(r.leaders, 1);
        let json = serde_json::to_string(&r).unwrap();
        for expected in [
            "Leader hours",
            "18446744073709551615",
            "-2147483648",
            "-92233720368547.75808",
            "Activity templates",
            "Activity division damage",
            "Activity damaged equipment",
            "Damage attribution",
            "country 13",
            "-17",
        ] {
            assert!(json.contains(expected), "{expected}");
        }
    }
    #[test]
    fn combat_log_country_filter_preserves_multiple_manual_and_dated_group_members() {
        let mut f = LandBattleCountryFilter {
            tags: Some(vec!["GER".into(), "SOV".into()]),
            membership: None,
            unobserved: vec![],
        };
        assert!(matches_country(&f, "GER", 7));
        assert!(matches_country(&f, "SOV", 7));
        assert!(!matches_country(&f, "USA", 7));
        f.tags = None;
        assert!(matches_country(&f, "USA", 7));
        f.membership = Some(vec![
            dashboard_base::products::country_groups::MembershipRun {
                tag: "SOV".into(),
                first_hour: 6,
                last_hour: 8,
            },
        ]);
        assert!(matches_country(&f, "SOV", 7));
        assert!(!matches_country(&f, "GER", 7));
        assert!(!matches_country(&f, "SOV", 9));
    }
    #[test]
    fn combat_log_names_equipment_from_the_supplying_read_on_a_carried_hour() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n GER_mod_rifle:0 \"Supplying design\"\n FRA_later_rifle:0 \"Later operands\"\n",
        )]);
        let variant = 7 << 32 | 70;
        let mut read = fixture();
        read.hour = 60_000_022;
        for table in equipment::VariantCatalogue::TABLES {
            if !matches!(table, TableId::Countries | TableId::CountryLinks) {
                *read.tables[table.index()] = TableView::Complete(TableRows::empty(table));
            }
        }
        // The creator's appearance: a country that is not dynamic is named by its own tag.
        *read.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(vec![
                replay_data::rows::CountryAppearanceRow {
                    country: 9001,
                    ..Default::default()
                },
            ]));
        *read.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: variant,
                creator_link: 11,
                equipment_token: 5,
                ..Default::default()
            }]));
        *read.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 5,
                literal: b"mod_rifle".to_vec(),
            }]));
        *read.tables[TableId::CombatLogEnemyEquipment.index()] =
            TableView::Complete(TableRows::CombatLogEnemyEquipment(vec![
                CombatLogEnemyEquipmentRow {
                    country: 11,
                    group: 7,
                    variant,
                    amount_raw: 123_456,
                    ..Default::default()
                },
            ]));
        *read.tables[TableId::CombatLogBattleEquipment.index()] =
            TableView::Complete(TableRows::CombatLogBattleEquipment(vec![
                CombatLogBattleEquipmentRow {
                    battle: 42,
                    variant,
                    amount_raw: 123_456,
                    ..Default::default()
                },
            ]));
        let mut carried = read.clone();
        carried.hour += 1;
        // Naming reads retain their own lexical and country operands, even if
        // these companion tables have changed at the selected hour.
        let TableView::Complete(TableRows::Countries(countries)) =
            &mut *carried.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        countries[0].tag = b"FRA".to_vec();
        *carried.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 5,
                literal: b"later_rifle".to_vec(),
            }]));
        let (source, context) =
            dashboard_base::test_fixtures::source_with_content(vec![read, carried], &folder.0);
        let flag = AtomicBool::new(false);
        for hour in [60_000_022, 60_000_023] {
            let selected = progressive::frame_at(
                &source,
                1,
                hour,
                &equipment::VariantCatalogue::TABLES,
                &flag,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                dashboard_text::metadata::slot_read(
                    source.replay_ref().format,
                    &selected.metrics,
                    TableId::VariantDefinitions,
                    hour,
                ),
                hour == 60_000_022,
            );
            let reply = CombatLog::load(
                &source,
                &CombatLogParams {
                    branch_id: "1".into(),
                    hour,
                    countries: LandBattleCountryFilter {
                        tags: None,
                        membership: None,
                        unobserved: vec![],
                    },
                },
                &context,
                &flag,
            )
            .unwrap();
            assert_eq!(
                reply.equipment[0].name,
                Some(format!("Supplying design ({variant})"))
            );
            let battle = serde_json::to_string(&reply.battles[0]).unwrap();
            assert!(battle.contains(&format!("Supplying design ({variant})")));
            assert!(!battle.contains("Name unavailable"));
            assert!(
                !reply
                    .warnings
                    .iter()
                    .any(|w| w.contains("no resolved label"))
            );
        }
    }
    #[test]
    fn combat_log_observer_census_carries_quiet_rows_and_encodes_only_changed_operands() {
        use replay_frame::frame::{FrameEncoder, TableUpdate};
        let table = TableId::CombatLogObservers;
        let mut first = fixture();
        *first.tables[table.index()] = TableView::Complete(TableRows::CombatLogObservers(
            (0..64)
                .map(|ordinal| CombatLogObserverRow {
                    combat: 71,
                    attacker: true,
                    ordinal,
                    kind: CombatLogObserverKind::Leader as u8,
                    item: ordinal,
                    identity: 1000 + ordinal,
                    a: 12,
                    ..Default::default()
                })
                .collect(),
        ));
        let mut encoder = FrameEncoder::new();
        encoder.encode(first.clone(), true).unwrap();
        let mut quiet = first;
        quiet.hour += 1;
        let frame = encoder.encode(quiet.clone(), false).unwrap();
        assert!(matches!(
            frame.tables[table.index()],
            TableUpdate::Unchanged { .. }
        ));
        let mut changed = quiet;
        changed.hour += 1;
        let TableView::Complete(TableRows::CombatLogObservers(rows)) =
            &mut *changed.tables[table.index()]
        else {
            unreachable!()
        };
        rows[17].a += 1;
        let frame = encoder.encode(changed, false).unwrap();
        let TableUpdate::Delta {
            changed, deleted, ..
        } = &frame.tables[table.index()]
        else {
            panic!("one leader-hour change must encode as a delta")
        };
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].key, (71u128 << 64) | (17 << 1) | 1);
        assert_eq!(
            changed[0].mask,
            1 << 6,
            "only the leader-hour operand changes"
        );
        assert!(deleted.is_empty());
    }
    #[test]
    fn combat_log_keeps_unknown_active_and_terminal_participants_for_every_filter() {
        let mut f = fixture();
        let mut active = Vec::new();
        let mut terminal = Vec::new();
        // Empty, unresolved, known, and mixed known/unresolved participant lists.
        for (at, countries) in [vec![], vec![1234], vec![11], vec![11, 1234]]
            .into_iter()
            .enumerate()
        {
            let mut operands = vec![LandTerminalObserver {
                attacker: true,
                kind: CombatLogObserverKind::State as u8,
                values: [123_456, 987_654, 3, 0, 0],
                ..Default::default()
            }];
            operands.extend(countries.into_iter().enumerate().map(|(ordinal, id)| {
                LandTerminalObserver {
                    attacker: true,
                    kind: CombatLogObserverKind::Country as u8,
                    item: ordinal as u64,
                    values: [ordinal as u64, id, 0, 0, 0],
                }
            }));
            let combat = 71 + at as u64;
            active.extend(operands.iter().enumerate().map(|(ordinal, r)| {
                let mut row =
                    CombatLogObserverRow::from_fact(combat, r.attacker, r.kind, r.item, r.values)
                        .unwrap();
                row.ordinal = ordinal as u64;
                row
            }));
            terminal.push(LandCombatResultsRow {
                id: at as u64 + 1,
                combat,
                observers: operands,
                ..Default::default()
            });
        }
        *f.tables[TableId::CombatLogObservers.index()] =
            TableView::Complete(TableRows::CombatLogObservers(active));
        *f.tables[TableId::LandCombatResults.index()] =
            TableView::Complete(TableRows::LandCombatResults(terminal));
        let mut selections: Vec<_> = [None, Some(vec!["GER".into()]), Some(vec!["FRA".into()])]
            .into_iter()
            .map(|tags| LandBattleCountryFilter {
                tags,
                membership: None,
                unobserved: vec![],
            })
            .collect();
        selections.push(LandBattleCountryFilter {
            tags: None,
            membership: Some(vec![
                dashboard_base::products::country_groups::MembershipRun {
                    tag: "FRA".into(),
                    first_hour: f.hour,
                    last_hour: f.hour,
                },
            ]),
            unobserved: vec![],
        });
        for (at, countries) in selections.into_iter().enumerate() {
            let expected = if at < 2 {
                vec!["71", "72", "73", "74"]
            } else {
                vec!["71", "72", "74"]
            };
            let reply = load_filtered(f.clone(), countries);
            assert!(reply.availability.active.is_none());
            assert!(reply.availability.terminal.is_none());
            for terminal in [false, true] {
                let sides: Vec<_> = reply
                    .active
                    .iter()
                    .filter(|r| r.terminal == terminal)
                    .collect();
                assert_eq!(
                    sides.iter().map(|r| r.combat.as_str()).collect::<Vec<_>>(),
                    expected
                );
                assert!(
                    sides.iter().all(|r| r.attacker),
                    "absent defender sides stay absent"
                );
                let empty = serde_json::to_string(&sides[0]).unwrap();
                assert!(empty.contains("Unknown participants"));
                let unresolved = serde_json::to_string(&sides[1]).unwrap();
                assert!(unresolved.contains("Country 1234 (1234)"));
            }
            assert!(reply.warnings.iter().any(|w| w.contains("1234")));
        }
    }
    #[test]
    fn combat_log_stable_labels_reconstruct_the_flattened_dashboard_view() {
        use CombatLogObserverKind as K;
        let facts = [
            (K::State, 0, [123_456, 987_654, 3, 0, 0]),
            (K::Counters, 0, [0, 1, u64::MAX, 3, 4]),
            (K::Side, 0, [0, 12_500, u64::MAX, 0, 0]),
            (K::Pool, 1, [1, 1, 0, 0, 0]),
            (K::Equipment, 1, [0, 71, 0, 0, 0]),
            (K::Equipment, 1, [1, 71, u64::MAX, 0, 0]),
            (K::Modifier, 0, [0, 11, 12, 0, 0]),
            (K::Country, 0, [0, 11, 0, 0, 0]),
            (K::Country, 0, [1, 11, 0, 0, 0]),
            (K::Leader, 0, [u64::MAX, u64::MAX, 0, 0, 0]),
            (K::Leader, 1, [u64::MAX, 4, 0, 0, 0]),
            (K::Damage, 0, [11, 11, 11, u64::MAX, 0]),
            (K::Activity, 0, [72, 11, 11, 0, 0]),
            (K::ActivityTemplate, 0, [0, 73, 0, 0, 0]),
            (K::ActivityTemplate, 0, [1, 73, 0, 0, 0]),
            (K::ActivityDamage, 0, [0, 74, u64::MAX, 0, 0]),
            (K::ActivityEquipment, 0, [u64::MAX, 1, 1, 0, 0]),
            (K::ActivityEquipment, 0, [0, 71, u64::MAX, 0, 0]),
            (K::Activity, 1, [75, 11, 11, 0, 0]),
            (K::ActivityDamage, 1, [0, 76, 10, 0, 0]),
        ];
        let mut f = fixture();
        let rows = facts
            .iter()
            .enumerate()
            .map(|(ordinal, &(kind, item, values))| {
                let mut row =
                    CombatLogObserverRow::from_fact(71, true, kind as u8, item, values).unwrap();
                row.ordinal = ordinal as u64;
                row
            })
            .collect();
        *f.tables[TableId::CombatLogObservers.index()] =
            TableView::Complete(TableRows::CombatLogObservers(rows));
        let flattened = load(f.clone());
        let TableView::Complete(TableRows::CombatLogObservers(rows)) =
            &mut *f.tables[TableId::CombatLogObservers.index()]
        else {
            unreachable!()
        };
        for row in rows.iter_mut() {
            row.ordinal = 1_000_000 - row.ordinal * 13;
        }
        rows.sort_unstable_by_key(CombatLogObserverRow::key);
        let stable = load(f);
        assert_eq!(
            serde_json::to_value(&stable.active).unwrap(),
            serde_json::to_value(&flattened.active).unwrap()
        );
        let active = stable.active.iter().find(|side| !side.terminal).unwrap();
        assert_eq!(active.activity_groups, 2);
        assert_eq!(active.leaders, 2);
        let templates = active
            .details
            .iter()
            .find(|detail| detail.title == "Activity templates")
            .unwrap();
        assert_eq!(templates.rows.len(), 2, "duplicate templates stay ordered");
    }
}
