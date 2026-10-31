//! The family against the per-hour whole observation it replaces: equal bytes and replies.

use super::Field;
use super::*;
use crate::products::{
    keyed::{Keyed, Metadata, Observed, Tags},
    test_support::*,
};
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    series::{
        self,
        tests::{Fixture, assert_payloads_equal},
    },
};
use replay_data::rows::*;
use replay_frame::frame::{FrameDecoder, FrameEncoder, TableView};
use std::sync::Arc;

/// The per-hour observation the family made before it observed by changes.
#[derive(Default)]
struct Reference(Prepared);

impl Reference {
    fn observe_changes(&mut self, frame: &DecodedFrame, changes: &FrameChanges) {
        let state = &mut self.0;
        let gap = state.keyed.step(frame.hour).unwrap();
        // The whole-table naming observer.
        state.variants.observe(FormatVersion::CURRENT, frame);
        let changed = |tables: &[TableId]| {
            gap || [TableId::Countries, TableId::CountryLinks]
                .iter()
                .chain(tables)
                .any(|table| changes.rows_changed[table.index()])
        };
        let domains = [
            changed(&LICENCES),
            changed(&[
                ORGANISATIONS[0],
                ORGANISATIONS[1],
                ORGANISATIONS[2],
                TableId::LexicalTokens,
            ]),
            changed(&[TableId::VariantDefinitions, TableId::VariantChanges])
                || changes.slot_read[TableId::VariantDefinitions.index()],
        ];
        if domains.iter().any(|&domain| domain) {
            self.observe(frame, domains);
        }
    }

    fn keyed(&mut self) -> &mut Keyed<Field, Held> {
        &mut self.0.keyed
    }

    fn text(&mut self, text: String) -> i128 {
        let texts = &mut self.0.keyed.meta.extra.texts;
        let index = texts
            .iter()
            .position(|held| *held == text)
            .unwrap_or_else(|| {
                texts.push(text);
                texts.len() - 1
            });
        index as i128
    }

    fn observe(&mut self, frame: &DecodedFrame, domains: [bool; 3]) {
        let hour = frame.hour;
        let mut observed = Observed::new();
        let tags = match Tags::new(frame) {
            Ok(tags) => tags,
            Err(error) => {
                self.keyed()
                    .refuse(format!("country identities unavailable: {error}"));
                self.keyed().commit(hour, &observed, |field| {
                    domains[usize::from(field.domain())]
                });
                return;
            }
        };
        if domains[0] {
            self.licences(frame, &tags, &mut observed);
        }
        if domains[1] {
            self.organisations(frame, &tags, &mut observed);
        }
        if domains[2] {
            self.designs(frame, &tags, &mut observed);
        }
        self.keyed().commit(hour, &observed, |field| {
            domains[usize::from(field.domain())]
        });
    }

    fn licences(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) {
        let hour = frame.hour;
        let rows = match frame.complete::<tables::ProductionLicenseRecords>() {
            Ok(rows) => rows,
            Err(error) => {
                self.keyed()
                    .refuse(format!("licences unavailable: {error}"));
                return;
            }
        };
        let keyed = self.keyed();
        let mut whole = true;
        for row in rows {
            let Some(owner) = tags.slot(row.owner) else {
                keyed.refuse("a licence's licensee has no identity");
                whole = false;
                continue;
            };
            keyed.tag(tags, owner, hour);
            let at = row.ordinal;
            let licensor = match tags.slot(row.giver) {
                Some(giver) => {
                    keyed.tag(tags, giver, hour);
                    (Field::LicenceGiver(owner, at), i128::from(giver))
                }
                None => {
                    keyed.refuse("a licence's licensor has no identity");
                    (Field::LicenceUnidentified(owner, at), 1)
                }
            };
            for (field, value) in [
                licensor,
                (Field::LicenceLent(owner, at), i128::from(row.lended_cic)),
                (
                    Field::LicenceRequired(owner, at),
                    i128::from(row.required_cic),
                ),
                (Field::LicenceStart(owner, at), i128::from(row.start_date)),
            ] {
                observed.insert(keyed.key(field), value);
            }
            for (index, &variant) in row.equipment.iter().enumerate() {
                observed.insert(
                    keyed.key(Field::LicenceVariant(owner, at, index as u32)),
                    i128::from(variant),
                );
            }
        }
        if whole {
            observed.insert(keyed.key(Field::Known(0)), 1);
        }
    }

    fn organisations(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) {
        let hour = frame.hour;
        let (roster, countries, links) = match (
            frame.complete::<tables::IndustrialOrganisations>(),
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) {
            (Ok(roster), Ok(countries), Ok(links)) => (roster, countries, links),
            (Err(error), ..) | (_, Err(error), _) | (.., Err(error)) => {
                self.keyed()
                    .refuse(format!("organisations unavailable: {error}"));
                return;
            }
        };
        let lexical = match frame.complete::<tables::LexicalTokens>() {
            Ok(lexical) => Some(lexical),
            Err(error) => {
                self.keyed()
                    .refuse(format!("organisation names unavailable: {error}"));
                None
            }
        };
        let naming = equipment::Tags::new(
            countries,
            links,
            frame.complete::<tables::CountryAppearance>().ok(),
        );
        let key = self.keyed().key(Field::Known(1));
        observed.insert(key, 1);
        let details = frame.complete::<tables::IndustrialOrganisationDetails>();
        if let Err(error) = &details {
            self.keyed()
                .refuse(format!("organisation state unavailable: {error}"));
        }
        let policies = frame.complete::<tables::IndustrialOrganisationPolicies>();
        if let Err(error) = &policies {
            self.keyed()
                .refuse(format!("organisation policies unavailable: {error}"));
        }
        for row in roster {
            let Some(holder) = tags.slot(row.country) else {
                self.keyed()
                    .refuse("an organisation's country has no identity");
                continue;
            };
            self.keyed().tag(tags, holder, hour);
            let id = row.organisation;
            let key = self.keyed().key(Field::MioHolder(id));
            observed.insert(key, i128::from(holder));
            let raw = match lexical {
                Some(lexical) => production_lines::raw_organisation(row, hour, &naming, lexical),
                None => RawOrganisation {
                    first_hour: hour,
                    owner_unknown: true,
                    ..Default::default()
                },
            };
            let versions = self.keyed().meta.extra.organisations.entry(id).or_default();
            if versions.last().is_none_or(|last| {
                (
                    &last.token,
                    &last.name_key,
                    &last.icon,
                    &last.owner_tag,
                    last.owner_unknown,
                ) != (
                    &raw.token,
                    &raw.name_key,
                    &raw.icon,
                    &raw.owner_tag,
                    raw.owner_unknown,
                )
            }) {
                versions.push(raw);
            }
            let Some(detail) = details.as_ref().ok().and_then(|details| {
                details
                    .binary_search_by_key(&id, |detail| detail.organisation)
                    .ok()
                    .map(|index| &details[index])
                    .filter(|detail| detail.country == row.country)
            }) else {
                continue;
            };
            let state = production_lines::organisation_state(
                detail,
                None,
                lexical.map_or(&[][..], |rows| rows),
            );
            for (field, value) in [
                (Field::MioSize(id), Some(i128::from(detail.state.size))),
                (
                    Field::MioFunds(id),
                    Some(i128::from(detail.state.funds_raw)),
                ),
                (Field::MioPoints(id), Some(i128::from(detail.state.points))),
                (Field::MioUnlocked(id), Some(detail.unlocked.len() as i128)),
                (
                    Field::MioCooldown(id),
                    detail.state.cooldown_hour.map(i128::from),
                ),
            ] {
                if let Some(value) = value {
                    let key = self.keyed().key(field);
                    observed.insert(key, value);
                }
            }
            if state.policy_token.as_deref() == Some("") {
                self.keyed()
                    .refuse("a selected policy's token is not spelled");
                let key = self.keyed().key(Field::MioPolicyUnresolved(id));
                observed.insert(key, 1);
            }
            if let Some(token) = state
                .policy_token
                .as_ref()
                .filter(|token| !token.is_empty())
            {
                let index = self.text(token.clone());
                let key = self.keyed().key(Field::MioPolicy(id));
                observed.insert(key, index);
            }
            if state.policy_token.is_some() {
                let name = policies.as_ref().ok().and_then(|rows| {
                    rows.binary_search_by_key(&detail.country, |row| row.country)
                        .ok()
                        .and_then(|index| {
                            rows[index]
                                .policies
                                .iter()
                                .find(|policy| policy.token == detail.state.policy)
                        })
                        .map(|policy| String::from_utf8_lossy(&policy.name).into_owned())
                });
                if let Some(name) = name.filter(|name| !name.is_empty()) {
                    let index = self.text(name);
                    let key = self.keyed().key(Field::MioPolicyName(id));
                    observed.insert(key, index);
                }
            }
        }
    }

    fn designs(&mut self, frame: &DecodedFrame, tags: &Tags<'_>, observed: &mut Observed) {
        let hour = frame.hour;
        let rows = match frame.complete::<tables::VariantDefinitions>() {
            Ok(rows) => rows,
            Err(error) => {
                self.keyed().refuse(format!("designs unavailable: {error}"));
                return;
            }
        };
        let keyed = self.keyed();
        keyed.meta.extra.designs_from.get_or_insert(hour);
        observed.insert(keyed.key(Field::Known(2)), 1);
        for row in rows {
            let Some(creator) = tags.slot(row.creator_link) else {
                keyed.refuse("a design's creator has no identity");
                continue;
            };
            keyed.tag(tags, creator, hour);
            observed.insert(keyed.key(Field::DesignCreator(row.id)), i128::from(creator));
            observed.insert(
                keyed.key(Field::DesignObsolete(row.id)),
                i128::from(row.obsolete != 0),
            );
        }
    }

    /// The flush it made, from a copy of the designs' naming reads.
    fn flush(&mut self, writer: &mut store::Writer) {
        let state = &mut self.0;
        let named: BTreeSet<u64> = state
            .keyed
            .meta
            .fields
            .values()
            .filter_map(|field| match *field {
                Field::DesignCreator(id) => Some(id),
                _ => None,
            })
            .collect();
        let held = &state.keyed.meta.extra;
        let meta = Metadata {
            fields: state.keyed.meta.fields.clone(),
            tags: state.keyed.meta.tags.clone(),
            refusals: state.keyed.meta.refusals.clone(),
            extra: Extra {
                texts: held.texts.clone(),
                organisations: held.organisations.clone(),
                variants: named
                    .into_iter()
                    .filter_map(|id| Some((id, state.variants.versions.get(&id)?.clone())))
                    .collect(),
                designs_from: held.designs_from,
            },
        };
        state
            .keyed
            .flush_values(writer, FAMILY, 1, &AtomicBool::new(false))
            .unwrap();
        series::write_catalogue(writer, FAMILY, 1, state.keyed.keys(), &meta).unwrap();
    }
}

/// A language for the replies' names.
struct Language(BTreeMap<String, String>);

impl Texts for Language {
    fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
    fn text(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

fn languages() -> Vec<Option<Language>> {
    let language = |names: &[(&str, &str)]| {
        Some(Language(
            names
                .iter()
                .map(|&(key, text)| (key.to_owned(), text.to_owned()))
                .collect(),
        ))
    };
    vec![
        None,
        language(&[
            ("mio_a", "Alpha Works"),
            ("mio_b", "Beta Works"),
            ("mio_b2", "Beta Industries"),
            ("policy_x", "Policy X"),
        ]),
        language(&[
            ("mio_a", "Alpha Werke"),
            ("mio_c", "Gamma Werke"),
            ("named_mio", "Benannt"),
        ]),
    ]
}

/// Every view's reply, by every tag and none, in every language.
fn replies(fixture: &Fixture, through: i32, tags: &[&str]) -> Vec<String> {
    let flag = AtomicBool::new(false);
    let mut replies = Vec::new();
    for language in languages() {
        let naming = Naming {
            texts: language.as_ref().map(|language| language as &dyn Texts),
            units: None,
            icon: &|raw: &RawOrganisation| {
                (!raw.token.is_empty()).then(|| {
                    let key = format!("GFX_{}", raw.token);
                    let image = if raw.icon.is_empty() {
                        Err("no stored sprite".to_owned())
                    } else {
                        Ok(raw.icon.clone())
                    };
                    (key, image)
                })
            },
        };
        for view in [
            CatalogueView::Licences,
            CatalogueView::Organisations,
            CatalogueView::Designs,
        ] {
            for chosen in std::iter::once(&[][..]).chain(tags.iter().map(std::slice::from_ref)) {
                let reply = query(
                    &fixture.store,
                    &fixture.key,
                    &ProductionCatalogueParams {
                        branch_id: "1".into(),
                        view,
                        tags: chosen.iter().map(|tag| tag.to_string()).collect(),
                    },
                    Some(through),
                    &naming,
                    Vec::new(),
                    &flag,
                )
                .unwrap();
                replies.push(serde_json::to_string(&reply).unwrap());
            }
        }
    }
    replies
}

struct Recording {
    blocks: Vec<Vec<i32>>,
    frame: Box<dyn Fn(i32) -> DecodedFrame>,
}

/// Prepare both ways, comparing after every block; the rows observed at each later hour.
fn compare(recording: &Recording, tags: &[&str], label: &str) -> Vec<(i32, usize)> {
    let format = FormatVersion::CURRENT;
    let mut family = Prepared::default();
    let mut reference = Reference::default();
    let (published, expected) = (Fixture::new(), Fixture::new());
    let mut later = Vec::new();
    for (block, hours) in recording.blocks.iter().enumerate() {
        // Every block is read alone: a keyframe, then each hour's delta.
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut tracker = FrameChangeTracker::new();
        for (index, &hour) in hours.iter().enumerate() {
            let wire = encoder.encode((recording.frame)(hour), index == 0).unwrap();
            let decoded = decoder.advance(&wire).unwrap().clone();
            let changes = tracker.observe_updates(&decoded, Arc::new(wire), format);
            let before = family.open.rows;
            family.observe_changes(format, &decoded, &changes).unwrap();
            if index > 0 {
                later.push((hour, family.open.rows - before));
            }
            reference.observe_changes(&decoded, &changes);
        }
        let mut writer = published.writer();
        family
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        let mut writer = expected.writer();
        reference.flush(&mut writer);
        writer.commit().unwrap();
        assert_payloads_equal(
            &published.payloads(),
            &expected.payloads(),
            &format!("{label}: block {block}"),
        );
        let through = *hours.last().unwrap();
        assert_eq!(
            replies(&published, through, tags),
            replies(&expected, through, tags),
            "{label}: replies after block {block}"
        );
        // Continued from its saved state, as the next block's preparation is.
        let saved = serde_json::to_vec(&family).unwrap();
        family = serde_json::from_slice(&saved).unwrap();
    }
    later
}

/// Whether the family's inputs are equal at two frames, balances apart.
fn same_inputs(before: &DecodedFrame, after: &DecodedFrame) -> bool {
    let identities = |frame: &DecodedFrame| {
        frame.complete::<tables::Countries>().ok().map(|rows| {
            rows.iter()
                .map(|row| (row.id, row.canonical_slot, row.tag.clone()))
                .collect::<Vec<_>>()
        })
    };
    identities(before) == identities(after)
        && [
            TableId::CountryLinks,
            TableId::ProductionLicenseRecords,
            TableId::IndustrialOrganisations,
            TableId::IndustrialOrganisationDetails,
            TableId::IndustrialOrganisationPolicies,
            TableId::LexicalTokens,
            TableId::VariantDefinitions,
        ]
        .iter()
        .all(|table| *before.tables[table.index()] == *after.tables[table.index()])
}

fn balances(at: &mut DecodedFrame, mut change: impl FnMut(&mut CountryRow)) {
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *at.tables[TableId::Countries.index()]
    {
        rows.iter_mut().for_each(&mut change);
    }
}

/// Countries 1..=6 at slots 10..=60, 7 from hour 52, raw 9 aliasing slot 10, raw 8 unknown.
fn churn(hour: i32) -> DecodedFrame {
    let mut at = frame(hour);
    let third = if hour >= 30 { "NEW" } else { "OTH" };
    let mut identities = vec![
        (1, 10, "LIC"),
        (2, 20, "GIV"),
        (3, 30, third),
        (4, 40, "FOU"),
        (5, 50, "FIV"),
        (6, 60, ""),
    ];
    if hour >= 52 {
        identities.push((7, 70, "SEV"));
    }
    countries(&mut at, &identities);
    // Every country's balances move every hour, as the game's do.
    balances(&mut at, |row| {
        row.political_power_raw = Some(i64::from(hour) * 1000 + i64::from(row.id));
        row.stability_raw = i64::from(hour % 5);
    });
    let mut links: Vec<CountryLinkRow> = identities
        .iter()
        .map(|&(id, canonical_slot, _)| CountryLinkRow { id, canonical_slot })
        .collect();
    links.push(CountryLinkRow {
        id: 9,
        canonical_slot: 10,
    });
    links.sort_by_key(|link| link.id);
    set(&mut at, TableRows::CountryLinks(links));
    if hour == 62 {
        unavailable(&mut at, TableId::Countries);
    }

    // Licences: granted, revoked, aliased licensees, an unknown licensor and licensee.
    let mut licences = vec![ProductionLicenseRecordRow {
        owner: 1,
        ordinal: 0,
        giver: 2,
        lended_cic: (hour / 24) % 3,
        required_cic: 2,
        start_date: 900,
        equipment: if hour < 40 {
            vec![0xc1, 0xc2]
        } else {
            vec![0xc1]
        },
        ..Default::default()
    }];
    if (10..30).contains(&hour) {
        licences.push(ProductionLicenseRecordRow {
            owner: 3,
            ordinal: 1,
            giver: 8,
            ..Default::default()
        });
    }
    if (15..48).contains(&hour) {
        licences.push(ProductionLicenseRecordRow {
            owner: 4,
            ordinal: 0,
            giver: 1,
            lended_cic: 1,
            equipment: vec![0xd1, 0xd2, 0xd3],
            ..Default::default()
        });
    }
    if (36..44).contains(&hour) {
        licences.push(ProductionLicenseRecordRow {
            owner: 8,
            ordinal: 2,
            giver: 2,
            lended_cic: 1,
            ..Default::default()
        });
    }
    if (20..50).contains(&hour) {
        licences.push(ProductionLicenseRecordRow {
            owner: 9,
            ordinal: 0,
            giver: 3,
            lended_cic: 4,
            equipment: vec![0xe1],
            ..Default::default()
        });
    }
    licences.sort_by_key(|row| (row.owner, row.ordinal));
    set(&mut at, TableRows::ProductionLicenseRecords(licences));
    if hour == 33 {
        unavailable(&mut at, TableId::ProductionLicenseRecords);
    }

    // Organisations: roster churn, unknown and mismatched countries, policy churn.
    let mut roster = vec![
        IndustrialOrganisationRow {
            organisation: 100,
            country: 1,
            owner: 1,
            token: 500,
            ..Default::default()
        },
        IndustrialOrganisationRow {
            organisation: 101,
            country: 2,
            owner: 2,
            token: 501,
            icon: b"GFX_beta".to_vec(),
            ..Default::default()
        },
        IndustrialOrganisationRow {
            organisation: 103,
            country: 8,
            owner: 8,
            token: 502,
            ..Default::default()
        },
    ];
    if hour >= 12 {
        roster.push(IndustrialOrganisationRow {
            organisation: 102,
            country: 3,
            owner: 3,
            token: 502,
            name: b"named_mio".to_vec(),
            ..Default::default()
        });
    }
    if hour < 60 {
        roster.push(IndustrialOrganisationRow {
            organisation: 104,
            country: 4,
            owner: 4,
            token: 500,
            ..Default::default()
        });
    }
    roster.sort_by_key(|row| row.organisation);
    set(&mut at, TableRows::IndustrialOrganisations(roster));
    let detail = |organisation: u64, country: u32, policy: u32| IndustrialOrganisationDetailsRow {
        organisation,
        country,
        state: IndustrialOrganisationState {
            size: hour / 24,
            // Funds move at the daily read.
            funds_raw: i64::from(hour / 24) * 10 - i64::from(organisation as i32),
            points: (hour / 24) % 4,
            policy,
            cooldown_hour: (hour / 24 % 2 == 1).then_some(hour / 24 * 24 + 72),
            ..Default::default()
        },
        unlocked: (0..(hour / 24) as u32).collect(),
        ..Default::default()
    };
    let mut details = vec![
        detail(100, 1, 0x4c17),
        detail(101, 2, if hour < 26 { 0x4c17 } else { 600 }),
        detail(104, 4, 600),
    ];
    if hour >= 12 {
        // Unspelled from 30; another country's detail from 70.
        details.push(detail(
            102,
            if hour >= 70 { 5 } else { 3 },
            if hour >= 30 { 601 } else { 600 },
        ));
    }
    details.sort_by_key(|row| row.organisation);
    set(&mut at, TableRows::IndustrialOrganisationDetails(details));
    if hour == 40 {
        unavailable(&mut at, TableId::IndustrialOrganisationDetails);
    }
    let policy_name = if hour < 50 { "Policy A" } else { "Policy B" };
    set(
        &mut at,
        TableRows::IndustrialOrganisationPolicies(vec![
            IndustrialOrganisationPoliciesRow {
                country: 2,
                policies: vec![IndustrialOrganisationPolicy {
                    token: 600,
                    name: policy_name.as_bytes().to_vec(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            IndustrialOrganisationPoliciesRow {
                country: 4,
                policies: vec![IndustrialOrganisationPolicy {
                    token: 600,
                    name: b"Policy Four".to_vec(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ]),
    );
    if hour == 55 {
        unavailable(&mut at, TableId::IndustrialOrganisationPolicies);
    }
    // Tokens: renamed at 36, unreadable at 44.
    let mut lexical = vec![
        LexicalTokenRow {
            token: 500,
            literal: b"mio_a".to_vec(),
        },
        LexicalTokenRow {
            token: 501,
            literal: if hour < 36 {
                b"mio_b".to_vec()
            } else {
                b"mio_b2".to_vec()
            },
        },
        LexicalTokenRow {
            token: 502,
            literal: b"mio_c".to_vec(),
        },
        LexicalTokenRow {
            token: 600,
            literal: b"policy_x".to_vec(),
        },
    ];
    if hour >= 58 {
        lexical.push(LexicalTokenRow {
            token: 503,
            literal: b"mio_d".to_vec(),
        });
    }
    lexical.sort_by_key(|row| row.token);
    set(&mut at, TableRows::LexicalTokens(lexical));
    if hour == 44 {
        unavailable(&mut at, TableId::LexicalTokens);
    }

    // Designs: created, made obsolete, deleted; one with an unknown creator.
    let mut designs: Vec<VariantDefinitionRow> = (1..=8_u64)
        .filter(|&id| {
            let created = id as i32 * 5;
            hour >= created && (id % 2 == 1 || hour < created + 38)
        })
        .map(|id| VariantDefinitionRow {
            id,
            creator_link: if id == 7 { 8 } else { (id % 6) as u32 + 1 },
            obsolete: u8::from(hour > id as i32 * 5 + 10),
            ..Default::default()
        })
        .collect();
    designs.sort_by_key(|row| row.id);
    set(&mut at, TableRows::VariantDefinitions(designs));
    if hour == 66 {
        unavailable(&mut at, TableId::VariantDefinitions);
    }
    at
}

#[test]
fn observing_by_changes_publishes_what_observing_every_row_does() {
    // 24-hour blocks; the one at 48 starts mid-churn, the last misses two hours.
    let mut blocks: Vec<Vec<i32>> = (0..4)
        .map(|block| (block * 24..block * 24 + 24).collect())
        .collect();
    blocks[3].retain(|&hour| !(80..82).contains(&hour));
    let recording = Recording {
        blocks,
        frame: Box::new(churn),
    };
    let later = compare(&recording, &["LIC", "GIV", "OTH", "NEW", "SEV"], "churn");
    let recorded: BTreeSet<i32> = recording.blocks.iter().flatten().copied().collect();
    // An hour whose inputs equal the hour before's, balances apart, observes no row.
    let quiet: Vec<i32> = later
        .iter()
        .filter(|(hour, _)| recorded.contains(&(hour - 1)))
        .filter(|(hour, _)| same_inputs(&churn(hour - 1), &churn(*hour)))
        .map(|&(hour, observed)| {
            assert_eq!(observed, 0, "hour {hour}");
            hour
        })
        .collect();
    assert!(quiet.len() >= 30, "{quiet:?}");
}

/// 1,500 countries with a licence, an organisation and two designs each.
fn crowd(hour: i32) -> DecodedFrame {
    const COUNTRIES: u32 = 1_500;
    let mut at = frame(hour);
    many_countries(&mut at, COUNTRIES);
    balances(&mut at, |row| {
        row.political_power_raw = Some(i64::from(hour) * 7 + i64::from(row.id))
    });
    let raw = |slot: u32| slot + 1;
    set(
        &mut at,
        TableRows::ProductionLicenseRecords(
            (0..COUNTRIES)
                .map(|slot| ProductionLicenseRecordRow {
                    owner: raw(slot),
                    ordinal: 0,
                    giver: raw((slot + 1) % COUNTRIES),
                    lended_cic: 1,
                    equipment: vec![u64::from(slot) * 2 + 1],
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut at,
        TableRows::IndustrialOrganisations(
            (0..COUNTRIES)
                .map(|slot| IndustrialOrganisationRow {
                    organisation: u64::from(slot) + 1,
                    country: raw(slot),
                    owner: raw(slot),
                    token: 500,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut at,
        TableRows::IndustrialOrganisationDetails(
            (0..COUNTRIES)
                .map(|slot| IndustrialOrganisationDetailsRow {
                    organisation: u64::from(slot) + 1,
                    country: raw(slot),
                    state: IndustrialOrganisationState {
                        size: 1,
                        policy: 0x4c17,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut at,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 500,
            literal: b"mio_a".to_vec(),
        }]),
    );
    let changing = (hour as u64 % 10) + 1;
    set(
        &mut at,
        TableRows::VariantDefinitions(
            (1..=u64::from(COUNTRIES) * 2)
                .map(|id| VariantDefinitionRow {
                    id,
                    creator_link: raw(((id - 1) / 2) as u32),
                    obsolete: u8::from(id == changing),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    at
}

#[test]
fn fifteen_hundred_countries_observe_only_their_changed_rows() {
    let blocks = vec![(0..6).collect(), (6..12).collect()];
    let recording = Recording {
        blocks,
        frame: Box::new(crowd),
    };
    let later = compare(&recording, &["T0001", "T1499"], "crowd");
    // Two designs change each later hour, of 3,000; nothing else does.
    assert!(
        later.iter().all(|&(_, observed)| observed <= 2),
        "{later:?}"
    );
}

#[test]
fn a_restored_key_catalogue_numbers_new_fields_as_their_names_would() {
    let mut keyed = Keyed::<Field, Held>::default();
    let first = [
        Field::Known(2),
        Field::DesignCreator(5),
        Field::LicenceVariant(1, 0, 3),
    ];
    let keys: Vec<u64> = first.iter().map(|&field| keyed.key(field)).collect();
    assert_eq!(keys, vec![0, 1, 2]);
    let mut restored: Keyed<Field, Held> =
        serde_json::from_slice(&serde_json::to_vec(&keyed).unwrap()).unwrap();
    assert_eq!(restored.lookup(&Field::DesignCreator(5)), Some(1));
    assert_eq!(restored.key(Field::LicenceVariant(1, 0, 3)), 2);
    assert_eq!(restored.key(Field::MioFunds(9)), 3);
    assert_eq!(restored.lookup(&Field::MioSize(9)), None);
    let names: Vec<(String, u64)> = restored
        .keys()
        .iter()
        .map(|(name, &key)| (name.clone(), key))
        .collect();
    let mut expected: Vec<(String, u64)> = first
        .iter()
        .chain([Field::MioFunds(9)].iter())
        .enumerate()
        .map(|(key, field)| (serde_json::to_string(field).unwrap(), key as u64))
        .collect();
    expected.sort();
    assert_eq!(names, expected);
}
