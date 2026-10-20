use super::*;
use replay_data::rows::RelationOpinionRow;

impl Prepared {
    pub(super) fn opinions(
        &mut self,
        frame: &DecodedFrame,
        format: FormatVersion,
        joins: &Joins,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let hour = frame.hour;
        let ready = evidence::available(frame, TableId::RelationOpinions)
            && (evidence::full_refresh(frame, format)
                || evidence::available(frame, TableId::SliceReads));
        self.number("opinions:coverage".into(), hour, ready.then_some(1), false);
        if !ready {
            let old: Vec<_> = self.owners.keys().cloned().collect();
            for country in old {
                self.close_owner(&country, hour);
            }
            return Ok(());
        }

        let full = evidence::full_refresh(frame, format);
        let mut selected =
            evidence::current_owners(frame, format, TableId::RelationOpinions, joins);
        self.expire_opinion_reads(hour, &selected);
        let rows = frame.complete::<tables::RelationOpinions>()?;
        self.refresh_opinion_orphans(frame, rows, joins);
        self.include_baseline_opinion_owners(frame, joins, full, &mut selected);
        for country in selected {
            check()?;
            self.observe_opinion_owner(frame, rows, joins, country, full, check)?;
        }
        Ok(())
    }
    fn expire_opinion_reads(&mut self, hour: i32, selected: &BTreeSet<DiplomacyCountry>) {
        let expired: Vec<_> = self
            .owners
            .iter()
            .filter(|(country, owner)| {
                !selected.contains(*country)
                    && owner.read.is_some_and(|read| {
                        i64::from(hour) - i64::from(read)
                            >= i64::from(replay_data::cadence::STAGGER_PERIOD)
                    })
            })
            .map(|(country, _)| country.clone())
            .collect();
        for country in expired {
            self.close_owner(&country, hour);
        }
    }
    fn refresh_opinion_orphans(
        &mut self,
        frame: &DecodedFrame,
        rows: &[RelationOpinionRow],
        joins: &Joins,
    ) {
        if self
            .opinion_view
            .as_ref()
            .is_none_or(|view| !view.is(&frame.tables[TableId::RelationOpinions.index()]))
        {
            self.orphan_opinions = rows
                .iter()
                .filter(|row| !joins.raw.contains_key(&row.owner_country))
                .count();
            self.opinion_view = Some(frame.tables[TableId::RelationOpinions.index()].identity());
        }
    }
    fn include_baseline_opinion_owners(
        &self,
        frame: &DecodedFrame,
        joins: &Joins,
        full: bool,
        selected: &mut BTreeSet<DiplomacyCountry>,
    ) {
        let hour = frame.hour;
        // A cold baseline can preserve explicit stamps; target identity proof at
        // that supplying hour is checked independently by the query.
        if self.segment_first == Some(hour) {
            for country in joins.countries.values() {
                if !self.owners.contains_key(country)
                    && evidence::owner_read_proven(
                        frame,
                        TableId::RelationOpinions,
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
    fn observe_opinion_owner(
        &mut self,
        frame: &DecodedFrame,
        rows: &[RelationOpinionRow],
        joins: &Joins,
        country: DiplomacyCountry,
        full: bool,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let read = std::iter::once(country.country_id)
            .chain(joins.aliases.get(&country).into_iter().flatten().copied())
            .filter_map(|raw| {
                evidence::owner_read_proven(frame, TableId::RelationOpinions, raw, full)
            })
            .max();

        let (pairs, unresolved) = self.project_opinion_owner(rows, joins, &country, check)?;
        self.store_opinion_owner(frame.hour, country, read, pairs, unresolved);
        Ok(())
    }
    fn project_opinion_owner(
        &mut self,
        rows: &[RelationOpinionRow],
        joins: &Joins,
        country: &DiplomacyCountry,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(BTreeMap<DiplomacyCountry, i32>, usize)> {
        let mut pairs = BTreeMap::new();
        let mut unresolved = 0;
        for &raw in joins.aliases.get(country).into_iter().flatten() {
            let first = rows.partition_point(|row| row.owner_country < raw);
            let last = rows.partition_point(|row| row.owner_country <= raw);
            for row in &rows[first..last] {
                check()?;
                self.projected_rows += 1;
                if let Some(target) = joins.raw.get(&row.target_country) {
                    if pairs.insert(target.clone(), row.opinion).is_some() || row.opinion == 0 {
                        unresolved += 1;
                    }
                } else {
                    unresolved += 1;
                }
            }
        }
        Ok((pairs, unresolved))
    }
    fn store_opinion_owner(
        &mut self,
        hour: i32,
        country: DiplomacyCountry,
        read: Option<i32>,
        pairs: BTreeMap<DiplomacyCountry, i32>,
        unresolved: usize,
    ) {
        let known = read.is_some() && unresolved == 0;
        let old = self.owners.remove(&country).unwrap_or_default();
        for target in old
            .pairs
            .keys()
            .filter(|target| !known || !pairs.contains_key(*target))
        {
            self.unindex_pair(&country, target);
            self.number(pair_key(&country, target), hour, None, false);
        }
        if known {
            for (target, value) in &pairs {
                self.pair_owners
                    .entry(target.clone())
                    .or_default()
                    .insert(country.clone());
                self.number(
                    pair_key(&country, target),
                    hour,
                    Some(i128::from(*value)),
                    !old.known,
                );
            }
        }
        self.number(
            country_key("o", &country),
            hour,
            known.then(|| i128::from(read.expect("known supplying read"))),
            !old.known,
        );
        self.number(
            country_key("r", &country),
            hour,
            read.map(i128::from),
            false,
        );
        self.owners.insert(
            country,
            Owner {
                read,
                known,
                unresolved,
                pairs: if known { pairs } else { BTreeMap::new() },
            },
        );
    }
}
