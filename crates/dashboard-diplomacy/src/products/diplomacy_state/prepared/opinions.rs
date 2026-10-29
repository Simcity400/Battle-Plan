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
        self.number(Fixed::OpinionCoverage, hour, ready.then_some(1));
        if !ready {
            self.close_opinion_reads(hour);
            return Ok(());
        }

        let full = evidence::full_refresh(frame, format);
        let mut selected =
            evidence::current_owners(frame, format, TableId::RelationOpinions, joins);
        self.expire_opinion_reads(hour, |country| selected.contains(country));
        let rows = frame.complete::<tables::RelationOpinions>()?;
        self.refresh_opinion_orphans(frame, rows, joins);
        self.include_baseline_opinion_owners(frame, joins, full, &mut selected);
        for country in selected {
            check()?;
            self.observe_opinion_owner(frame, rows, joins, country, full, check)?;
        }
        Ok(())
    }
    pub(super) fn expire_opinion_reads(
        &mut self,
        hour: i32,
        selected: impl Fn(&DiplomacyCountry) -> bool,
    ) {
        if self.opinion_deadlines.is_none() {
            self.opinion_deadlines = Some(
                self.owners
                    .iter()
                    .filter_map(|(country, owner)| {
                        owner.read.map(|read| (deadline(read), country.clone()))
                    })
                    .collect(),
            );
        }
        let expired = due(
            self.opinion_deadlines.as_mut().expect("opinion deadlines"),
            hour,
        );
        for (at, country) in expired {
            if !selected(&country)
                && self
                    .owners
                    .get(&country)
                    .and_then(|owner| owner.read)
                    .is_some_and(|read| deadline(read) == at)
            {
                self.close_owner(&country, hour);
            }
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
            self.orphan_opinions = evidence::owner_runs(rows, |row| row.owner_country)
                .filter(|(owner, _)| !joins.raw.contains_key(owner))
                .map(|(_, rows)| rows)
                .sum();
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
        self.store_opinion_owner(frame.hour, country, read, &pairs, unresolved, None);
        Ok(())
    }
    fn project_opinion_owner<'j>(
        &mut self,
        rows: &[RelationOpinionRow],
        joins: &'j Joins,
        country: &DiplomacyCountry,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Vec<(&'j DiplomacyCountry, i32)>, usize)> {
        let selected = joins
            .aliases
            .get(country)
            .into_iter()
            .flatten()
            .flat_map(|&raw| {
                let first = rows.partition_point(|row| row.owner_country < raw);
                let last = rows.partition_point(|row| row.owner_country <= raw);
                rows[first..last].iter()
            })
            .inspect(|_| self.projected_rows += 1)
            .map(|row| Ok((row.target_country, row.opinion)));
        project(selected, move |raw| joins.raw.get(&raw), check)
    }
    /// Stores an owner's read of `pairs`, ascending by target, and the values they came from.
    pub(super) fn store_opinion_owner(
        &mut self,
        hour: i32,
        country: DiplomacyCountry,
        read: Option<i32>,
        pairs: &[(&DiplomacyCountry, i32)],
        unresolved: usize,
        projected: Option<Arc<columnar::Values>>,
    ) {
        let known = read.is_some() && unresolved == 0;
        self.remember_opinion_deadline(&country, read);
        let mut old = self.owners.remove(&country).unwrap_or_default();
        if let Some(total) = &mut self.owner_unresolved {
            *total = *total - old.unresolved + unresolved;
        }
        for (target, pair) in &old.pairs {
            if !known
                || pairs
                    .binary_search_by_key(&target, |&(held, _)| held)
                    .is_err()
            {
                self.unindex_pair(&country, target);
                self.pair_number(&country, target, pair.key, hour, None, false);
            }
        }
        let mut kept = BTreeMap::new();
        if known {
            for &(target, value) in pairs {
                let (target, held) = match old.pairs.remove_entry(target) {
                    Some((target, pair)) => (target, pair.key),
                    None => (target.clone(), None),
                };
                self.index_pair(&country, &target);
                let key = self.pair_number(
                    &country,
                    &target,
                    held,
                    hour,
                    Some(i128::from(value)),
                    !old.known,
                );
                kept.insert(
                    target,
                    Pair {
                        value,
                        key: Some(key),
                    },
                );
            }
        }
        self.country_number(
            Series::Owner,
            &country,
            hour,
            known.then(|| i128::from(read.expect("known supplying read"))),
            !old.known,
        );
        self.country_number(Series::Read, &country, hour, read.map(i128::from), false);
        self.owners.insert(
            country,
            Owner {
                read,
                known,
                unresolved,
                pairs: kept,
                projected,
            },
        );
    }
}

/// Pairs by resolved target, each the last row's value, and the rows refusing the owner's proof.
pub(super) fn project<T: Ord + Copy>(
    rows: impl Iterator<Item = io::Result<(u32, i32)>>,
    resolve: impl Fn(u32) -> Option<T>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Vec<(T, i32)>, usize)> {
    let mut resolved = Vec::new();
    let mut unresolved = 0;
    for row in rows {
        check()?;
        let (target, opinion) = row?;
        match resolve(target) {
            Some(target) => resolved.push((target, resolved.len(), opinion)),
            None => unresolved += 1,
        }
    }
    resolved.sort_unstable_by_key(|&(target, at, _)| (target, at));
    let mut pairs: Vec<(T, i32)> = Vec::with_capacity(resolved.len());
    for (target, _, opinion) in resolved {
        if let Some(last) = pairs.last_mut().filter(|last| last.0 == target) {
            last.1 = opinion;
            unresolved += 1;
        } else {
            unresolved += usize::from(opinion == 0);
            pairs.push((target, opinion));
        }
    }
    Ok((pairs, unresolved))
}
