use super::*;
use crate::products::country_history::RecordedHistories;

#[derive(Default)]
struct NativeCountry {
    aliases: Vec<MembershipRun>,
    presence: Vec<HourSpan>,
}

/// Admission follows the dated native holder, not a tag which another country
/// may later reuse. The same prepared roster drives totals and their member rows.
pub(super) struct NativeMembers {
    countries: Vec<NativeCountry>,
    used: Vec<bool>,
    identity_reads: Vec<HourSpan>,
    pub rosters: BTreeMap<String, Vec<EffectiveRoster>>,
    pub counted: BTreeMap<String, Vec<MembershipRun>>,
    pub reasons: BTreeMap<String, String>,
    pub unread_hours: usize,
    #[cfg(test)]
    pub identity_visits: usize,
}

impl NativeMembers {
    pub fn new(
        history: &History,
        identities: &RecordedHistories,
        membership: FactionMembership,
        cancelled: &AtomicBool,
    ) -> io::Result<Self> {
        let mut countries: Vec<NativeCountry> = Vec::new();
        let mut native = BTreeMap::new();
        let mut tags: BTreeMap<&str, Vec<(i32, i32, usize)>> = BTreeMap::new();
        for (tag, country) in &identities.countries {
            check_cancel(cancelled)?;
            for run in country.identities.iter() {
                let index = *native.entry((run.slot, run.country_id)).or_insert_with(|| {
                    countries.push(NativeCountry::default());
                    countries.len() - 1
                });
                tags.entry(tag)
                    .or_default()
                    .push((run.first_hour, run.last_hour, index));
                countries[index].aliases.push(MembershipRun {
                    tag: tag.clone(),
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                });
            }
        }
        for country in &mut countries {
            country
                .aliases
                .sort_by_key(|run| (run.first_hour, run.last_hour));
            for alias in &country.aliases {
                country_groups::mark_unobserved(
                    &mut country.presence,
                    alias.first_hour,
                    alias.last_hour,
                );
            }
        }
        let mut out = Self {
            used: vec![false; countries.len()],
            countries,
            identity_reads: identities.identity_reads.clone(),
            rosters: BTreeMap::new(),
            counted: BTreeMap::new(),
            reasons: BTreeMap::new(),
            unread_hours: 0,
            #[cfg(test)]
            identity_visits: 0,
        };
        for (id, rosters) in &history.factions {
            check_cancel(cancelled)?;
            let mut reason = roster_reason(history, rosters, membership);
            let mut fixed = BTreeSet::new();
            if reason.is_none() && membership != FactionMembership::Hour {
                let anchors: Vec<_> = if membership == FactionMembership::End {
                    rosters
                        .last()
                        .into_iter()
                        .map(|run| (run, run.last_hour, run.last_hour))
                        .collect()
                } else {
                    rosters
                        .iter()
                        .map(|run| (run, run.first_hour, run.last_hour))
                        .collect()
                };
                'anchors: for (roster, first, last) in anchors {
                    check_cancel(cancelled)?;
                    for tag in &roster.members {
                        check_cancel(cancelled)?;
                        let mut next = i64::from(first);
                        let runs = tags
                            .get(tag.as_str())
                            .map(Vec::as_slice)
                            .unwrap_or_default();
                        let from = runs.partition_point(|&(_, end, _)| end < first);
                        for &(start, end, index) in runs[from..]
                            .iter()
                            .take_while(|&&(start, _, _)| start <= last)
                        {
                            #[cfg(test)]
                            {
                                out.identity_visits += 1;
                            }
                            if i64::from(start) > next {
                                break;
                            }
                            fixed.insert(index);
                            next = next.max(i64::from(end) + 1);
                        }
                        if next <= i64::from(last) {
                            reason = Some("This faction's selected member set has an unrecorded native country identity".into());
                            break 'anchors;
                        }
                    }
                }
            }
            let fixed: Arc<[usize]> = fixed.into_iter().collect::<Vec<_>>().into();
            let mut effective = Vec::new();
            for roster in rosters {
                check_cancel(cancelled)?;
                if membership != FactionMembership::Hour {
                    push_roster(
                        &mut effective,
                        roster.first_hour,
                        roster.last_hour,
                        reason.is_none().then(|| Arc::clone(&fixed)),
                    );
                    continue;
                }
                if !roster.complete {
                    push_roster(&mut effective, roster.first_hour, roster.last_hour, None);
                    continue;
                }
                let mut events: BTreeMap<i64, Vec<(usize, bool)>> = BTreeMap::new();
                events.entry(i64::from(roster.first_hour)).or_default();
                events.entry(i64::from(roster.last_hour) + 1).or_default();
                for tag in &roster.members {
                    check_cancel(cancelled)?;
                    let runs = tags
                        .get(tag.as_str())
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let from = runs.partition_point(|&(_, end, _)| end < roster.first_hour);
                    for &(start, end, index) in runs[from..]
                        .iter()
                        .take_while(|&&(start, _, _)| start <= roster.last_hour)
                    {
                        #[cfg(test)]
                        {
                            out.identity_visits += 1;
                        }
                        let first = start.max(roster.first_hour);
                        let last = end.min(roster.last_hour);
                        events
                            .entry(i64::from(first))
                            .or_default()
                            .push((index, true));
                        events
                            .entry(i64::from(last) + 1)
                            .or_default()
                            .push((index, false));
                    }
                }
                let mut current: BTreeMap<usize, usize> = BTreeMap::new();
                let mut known = 0usize;
                let mut before = i64::from(roster.first_hour);
                for (hour, changes) in events {
                    check_cancel(cancelled)?;
                    if before < hour {
                        let members = (known == roster.members.len())
                            .then(|| Arc::from(current.keys().copied().collect::<Vec<_>>()));
                        if members.is_none() {
                            out.unread_hours =
                                out.unread_hours.saturating_add((hour - before) as usize);
                        }
                        push_roster(&mut effective, before as i32, (hour - 1) as i32, members);
                    }
                    for (index, start) in changes {
                        if start {
                            *current.entry(index).or_default() += 1;
                            known += 1;
                        } else {
                            known -= 1;
                            let count = current.get_mut(&index).expect("active identity");
                            *count -= 1;
                            if *count == 0 {
                                current.remove(&index);
                            }
                        }
                    }
                    before = hour;
                }
            }
            let mut counted: BTreeMap<String, Vec<HourSpan>> = BTreeMap::new();
            for (first, last, members) in &effective {
                for &index in members.iter().flat_map(|members| members.iter()) {
                    out.used[index] = true;
                    let aliases = &out.countries[index].aliases;
                    let from = aliases.partition_point(|alias| alias.last_hour < *first);
                    for alias in aliases[from..]
                        .iter()
                        .take_while(|alias| alias.first_hour <= *last)
                    {
                        country_groups::mark_unobserved(
                            counted.entry(alias.tag.clone()).or_default(),
                            (*first).max(alias.first_hour),
                            (*last).min(alias.last_hour),
                        );
                    }
                }
            }
            out.counted.insert(
                id.clone(),
                counted
                    .into_iter()
                    .flat_map(|(tag, spans)| {
                        spans.into_iter().map(move |span| MembershipRun {
                            tag: tag.clone(),
                            first_hour: span.first_hour,
                            last_hour: span.last_hour,
                        })
                    })
                    .collect(),
            );
            if let Some(reason) = reason {
                out.reasons.insert(id.clone(), reason);
            }
            out.rosters.insert(id.clone(), effective);
        }
        Ok(out)
    }

    pub fn tags(&self) -> Vec<String> {
        self.countries
            .iter()
            .enumerate()
            .filter(|(index, _)| self.used[*index])
            .flat_map(|(_, country)| &country.aliases)
            .map(|alias| alias.tag.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn measures(
        &self,
        tracks: &[CountryTrack],
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Vec<MeasureRun>>> {
        let by_tag: BTreeMap<_, _> = tracks
            .iter()
            .map(|track| (track.tag.as_str(), &track.runs))
            .collect();
        self.countries
            .iter()
            .enumerate()
            .map(|(index, country)| {
                check_cancel(cancelled)?;
                if !self.used[index] {
                    return Ok(Vec::new());
                }
                let mut runs = Vec::new();
                for (alias_index, alias) in country.aliases.iter().enumerate() {
                    let Some(values) = by_tag.get(alias.tag.as_str()) else {
                        continue;
                    };
                    let from = values.partition_point(|run| run.last_hour < alias.first_hour);
                    for run in values[from..]
                        .iter()
                        .take_while(|run| run.first_hour <= alias.last_hour)
                    {
                        let first = run.first_hour.max(alias.first_hour);
                        let renamed = first == alias.first_hour
                            && alias_index
                                .checked_sub(1)
                                .and_then(|index| country.aliases.get(index))
                                .is_some_and(|previous| {
                                    previous.tag != alias.tag
                                        && previous.last_hour.checked_add(1) == Some(first)
                                });
                        runs.push(MeasureRun {
                            first_hour: first,
                            last_hour: run.last_hour.min(alias.last_hour),
                            value: run.value,
                            starts_segment: !renamed
                                && (first == alias.first_hour
                                    || run.starts_segment && first == run.first_hour),
                        });
                    }
                }
                // A native country absent from a complete country read is proved
                // zero. An unread identity/roster hour cannot establish that absence.
                for_each_absent(
                    &self.identity_reads,
                    &country.presence,
                    cancelled,
                    |first_hour, last_hour| {
                        runs.push(MeasureRun {
                            first_hour,
                            last_hour,
                            value: Some(0),
                            starts_segment: true,
                        });
                        Ok(())
                    },
                )?;
                runs.sort_by_key(|run| run.first_hour);
                Ok(runs)
            })
            .collect()
    }
}

fn push_roster(
    runs: &mut Vec<EffectiveRoster>,
    first: i32,
    last: i32,
    members: Option<Arc<[usize]>>,
) {
    if let Some((_, before, previous)) = runs.last_mut()
        && before.checked_add(1) == Some(first)
        && *previous == members
    {
        *before = last;
    } else {
        runs.push((first, last, members));
    }
}
