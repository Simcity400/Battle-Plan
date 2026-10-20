use super::*;
use replay_data::rows::LexicalTokenRow;

struct TupleRead<'a> {
    hour: i32,
    read_hour: i32,
    joins: &'a Joins,
    tokens: Option<&'a [LexicalTokenRow]>,
}
struct PactIdentity {
    signature: String,
    raw_countries: Vec<u32>,
    roles: Vec<Option<DiplomacyCountry>>,
    kind: String,
    unresolved_kind: Option<u32>,
}

impl Prepared {
    pub(super) fn tuples(
        &mut self,
        frame: &DecodedFrame,
        format: FormatVersion,
        joins: &Joins,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let hour = frame.hour;
        let ready = evidence::available(frame, TableId::RelationTuples)
            && (evidence::full_refresh(frame, format)
                || evidence::available(frame, TableId::SliceReads));
        self.number("tuples:coverage".into(), hour, ready.then_some(1), false);
        if !ready {
            self.end_pacts(None, true);
            self.close_tuple_reads(hour);
            return Ok(());
        }

        let mut selected = evidence::current_owners(frame, format, TableId::RelationTuples, joins);
        self.expire_tuple_reads(hour, joins, &selected);
        let rows = frame.complete::<tables::RelationTuples>()?;
        let tokens = frame.complete::<tables::LexicalTokens>().ok();
        let full = evidence::full_refresh(frame, format);
        self.include_baseline_tuple_owners(frame, joins, full, &mut selected);
        let mut owners = self.remember_tuple_reads(frame, joins, full, selected);
        self.include_orphan_tuple_owners(frame, rows, joins, full, &mut owners);
        self.expire_raw_tuple_reads(hour, &owners);
        for (raw, read) in owners {
            self.observe_tuple_owner(
                rows,
                raw,
                TupleRead {
                    hour,
                    read_hour: read,
                    joins,
                    tokens,
                },
                check,
            )?;
        }
        Ok(())
    }
    fn expire_tuple_reads(
        &mut self,
        hour: i32,
        joins: &Joins,
        selected: &BTreeSet<DiplomacyCountry>,
    ) {
        let expired: Vec<_> = self
            .tuple_reads
            .iter()
            .filter(|(owner, read)| {
                !selected.contains(*owner)
                    && i64::from(hour) - i64::from(**read)
                        >= i64::from(replay_data::cadence::STAGGER_PERIOD)
            })
            .map(|(owner, _)| owner.clone())
            .collect();
        for owner in expired {
            for raw in joins.aliases.get(&owner).into_iter().flatten() {
                let keys: Vec<_> = self
                    .pacts_by_owner
                    .get(raw)
                    .into_iter()
                    .flatten()
                    .copied()
                    .collect();
                for key in keys {
                    self.end_pact(key, true);
                }
            }
            self.tuple_reads.remove(&owner);
            self.number(country_key("t", &owner), hour, None, false);
        }
    }
    fn include_baseline_tuple_owners(
        &self,
        frame: &DecodedFrame,
        joins: &Joins,
        full: bool,
        selected: &mut BTreeSet<DiplomacyCountry>,
    ) {
        let hour = frame.hour;
        if self.segment_first == Some(hour) {
            for country in joins.countries.values() {
                if evidence::owner_read_proven(
                    frame,
                    TableId::RelationTuples,
                    country.country_id,
                    full,
                )
                .is_some()
                {
                    selected.insert(country.clone());
                }
            }
        }
    }
    fn remember_tuple_reads(
        &mut self,
        frame: &DecodedFrame,
        joins: &Joins,
        full: bool,
        selected: BTreeSet<DiplomacyCountry>,
    ) -> BTreeMap<u32, i32> {
        let hour = frame.hour;
        let mut owners = BTreeMap::<u32, i32>::new();
        for owner in selected {
            let read = std::iter::once(owner.country_id)
                .chain(joins.aliases.get(&owner).into_iter().flatten().copied())
                .filter_map(|raw| {
                    evidence::owner_read_proven(frame, TableId::RelationTuples, raw, full)
                })
                .max();
            if let Some(read) = read {
                for &raw in joins.aliases.get(&owner).into_iter().flatten() {
                    owners.insert(raw, read);
                }
                self.number(
                    country_key("t", &owner),
                    hour,
                    Some(i128::from(read)),
                    false,
                );
                self.tuple_reads.insert(owner, read);
            }
        }
        owners
    }
    fn include_orphan_tuple_owners(
        &mut self,
        frame: &DecodedFrame,
        rows: &[RelationTupleRow],
        joins: &Joins,
        full: bool,
        owners: &mut BTreeMap<u32, i32>,
    ) {
        let hour = frame.hour;
        // An unresolved storage owner must not silently remove an occurrence.
        // Explicit supplying stamps (or a full refresh) retain its raw identity.
        if self
            .tuple_view
            .as_ref()
            .is_none_or(|view| !view.is(&frame.tables[TableId::RelationTuples.index()]))
        {
            self.orphan_tuple_owners = rows
                .iter()
                .filter(|row| !joins.raw.contains_key(&row.owner_country))
                .map(|row| row.owner_country)
                .collect();
            self.tuple_view = Some(frame.tables[TableId::RelationTuples.index()].identity());
        }
        for &raw in &self.orphan_tuple_owners {
            if let Some(read) =
                evidence::owner_read_proven(frame, TableId::RelationTuples, raw, full)
                    .filter(|read| *read == hour || self.segment_first == Some(hour))
            {
                owners.insert(raw, read);
            }
        }
    }
    fn expire_raw_tuple_reads(&mut self, hour: i32, owners: &BTreeMap<u32, i32>) {
        let expired_raw: Vec<_> = self
            .raw_tuple_reads
            .iter()
            .filter(|(raw, read)| {
                !owners.contains_key(raw)
                    && i64::from(hour) - i64::from(**read)
                        >= i64::from(replay_data::cadence::STAGGER_PERIOD)
            })
            .map(|(&raw, _)| raw)
            .collect();
        for raw in expired_raw {
            let keys: Vec<_> = self
                .pacts_by_owner
                .get(&raw)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            for key in keys {
                self.end_pact(key, true);
            }
            self.raw_tuple_reads.remove(&raw);
        }
    }
    fn observe_tuple_owner(
        &mut self,
        rows: &[RelationTupleRow],
        raw: u32,
        read: TupleRead<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.raw_tuple_reads.insert(raw, read.read_hour);
        check()?;
        let first = rows.partition_point(|r| r.owner_country < raw);
        let last = rows.partition_point(|r| r.owner_country <= raw);
        let mut found = BTreeSet::new();
        for row in &rows[first..last] {
            check()?;
            found.insert(self.observe_pact(row, &read)?);
        }
        self.end_missing_pacts(raw, &found);
        Ok(())
    }
    fn observe_pact(
        &mut self,
        row: &RelationTupleRow,
        observed: &TupleRead<'_>,
    ) -> io::Result<u64> {
        let joins = observed.joins;
        let tokens = observed.tokens;
        let read = observed.read_hour;
        let raw_countries = vec![
            row.owner_country,
            row.target_country,
            row.first_country,
            row.second_country,
        ];
        let roles = raw_countries
            .iter()
            .map(|raw| self.role_at_read(joins, *raw, read))
            .collect::<Vec<_>>();
        let spelling = tokens
            .and_then(|tokens| {
                tokens
                    .binary_search_by_key(&row.type_id, |r| r.token)
                    .ok()
                    .map(|i| &tokens[i])
            })
            .and_then(|row| std::str::from_utf8(&row.literal).ok())
            .map(str::to_owned);
        let unresolved_kind = spelling.is_none().then_some(row.type_id);
        let kind = spelling.unwrap_or_else(|| format!("unresolved-token-{}", row.type_id));
        self.catalogue.kinds.insert(kind.clone(), row.type_id);
        let signature = tuple_signature(row, &roles, &kind)?;

        let key = self.pact_key(
            row,
            observed,
            PactIdentity {
                signature,
                raw_countries,
                roles,
                kind,
                unresolved_kind,
            },
        );
        let pact = &mut self.pacts.get_mut(&key).expect("open pact").reply;
        pact.last_seen = read;
        pact.read_hour = read;
        pact.end_hour = date(row.end_hour);
        pact.cancelled = boolean(row.cancel);
        pact.retired = boolean(row.retired);
        pact.cancel_raw = row.cancel;
        pact.retired_raw = row.retired;
        self.dirty.insert(key);
        Ok(key)
    }
    fn pact_key(
        &mut self,
        row: &RelationTupleRow,
        observed: &TupleRead<'_>,
        identity: PactIdentity,
    ) -> u64 {
        let PactIdentity {
            signature,
            raw_countries,
            roles,
            kind,
            unresolved_kind,
        } = identity;
        let hour = observed.hour;
        let read = observed.read_hour;
        let raw = row.owner_country;
        if let Some(&key) = self.pact_keys.get(&signature) {
            key
        } else {
            let id = format!("pact:{}:{}", hour, self.keys.len());
            let key = self.key(id.clone());
            self.catalogue.entries.push(Entry {
                key,
                queue: false,
                roles: roles.clone(),
                raw_roles: raw_countries.clone(),
                unresolved_kind,
            });
            self.pacts.insert(
                key,
                PactLive {
                    signature: signature.clone(),
                    raw_owner: raw,
                    reply: DiplomacyPact {
                        id,
                        kind,
                        owner: tag(&roles[0]),
                        target: tag(&roles[1]),
                        first: tag(&roles[2]),
                        second: tag(&roles[3]),
                        identities: roles,
                        raw_countries,
                        ordinal: row.ordinal,
                        start_hour: date(row.start_hour),
                        end_hour: date(row.end_hour),
                        cancelled: boolean(row.cancel),
                        retired: boolean(row.retired),
                        cancel_raw: row.cancel,
                        retired_raw: row.retired,
                        first_seen: read,
                        last_seen: read,
                        present_at_end: true,
                        ended_by: false,
                        read_hour: read,
                    },
                },
            );
            self.pact_keys.insert(signature, key);
            self.pacts_by_owner.entry(raw).or_default().insert(key);
            key
        }
    }
    fn end_missing_pacts(&mut self, raw: u32, found: &BTreeSet<u64>) {
        let missing: Vec<_> = self
            .pacts_by_owner
            .get(&raw)
            .into_iter()
            .flatten()
            .filter(|key| !found.contains(key))
            .copied()
            .collect();
        for key in missing {
            self.end_pact(key, false);
        }
    }
}
