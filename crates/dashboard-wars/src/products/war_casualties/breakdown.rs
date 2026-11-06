//! Exact known losses, grouped by the same viewpoint episodes as Wars.
//! Casualty totals combine land, sea and air losses. Their unknown fields
//! have no enemy or domain; neither is invented here.

use super::*;
#[cfg(any(test, feature = "test-support"))]
use crate::products::wars::Wars;
use crate::products::wars::WarsParams;
use crate::products::wars::{WarName, WarRelationKey, WarsReply};

pub mod prepared;

pub struct CasualtyBreakdown;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasualtyBreakdownParams {
    pub branch_id: String,
    pub tag: String,
    pub window: Option<usize>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub read_hour: i32,
    pub hourly: bool,
    /// The actual relation observation which supplied the native grouping.
    pub grouping_hour: i32,
    pub grouping_hourly: bool,
    /// Current enemy facts when present, else its last observed holder's facts.
    pub enemy_hour: i32,
    pub enemy_hourly: bool,
    pub value: String,
    pub war_id: Option<String>,
    pub reason: Option<String>,
    pub incomplete: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownSeries {
    pub id: String,
    pub country_identity: String,
    pub enemy_tag: String,
    pub enemy_identity: String,
    pub runs: Vec<CasualtyBreakdownRun>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownCountryRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub identity: String,
    pub incomplete: bool,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyWarNameRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: WarName,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownWar {
    pub id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub started_by: bool,
    pub ended_by: bool,
    pub names: Vec<CasualtyWarNameRun>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CasualtyBreakdownReply {
    pub branch_id: String,
    pub tag: String,
    pub available: bool,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    pub observed: Vec<HourSpan>,
    pub countries: Vec<CasualtyBreakdownCountryRun>,
    pub series: Vec<CasualtyBreakdownSeries>,
    pub wars: Vec<CasualtyBreakdownWar>,
    pub has_gap: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
struct GroupRun {
    first: i32,
    last: i32,
    war: Option<usize>,
    ambiguous: bool,
}
fn group_runs(memberships: &[(i32, i32, usize)]) -> Vec<GroupRun> {
    let mut changes: BTreeMap<i64, Vec<(usize, i32)>> = BTreeMap::new();
    for &(first, last, war) in memberships {
        changes.entry(i64::from(first)).or_default().push((war, 1));
        changes
            .entry(i64::from(last) + 1)
            .or_default()
            .push((war, -1));
    }
    let mut active = BTreeMap::<usize, i32>::new();
    let mut changes = changes.into_iter().peekable();
    let mut runs = Vec::new();
    while let Some((first, events)) = changes.next() {
        for (war, delta) in events {
            let count = active.entry(war).or_default();
            *count += delta;
            if *count == 0 {
                active.remove(&war);
            }
        }
        let Some(&(next, _)) = changes.peek() else {
            continue;
        };
        runs.push(GroupRun {
            first: first as i32,
            last: (next - 1) as i32,
            war: (active.len() == 1).then(|| *active.keys().next().expect("one war")),
            ambiguous: active.len() > 1,
        });
    }
    runs
}
fn group_at(groups: &[GroupRun], hour: i32) -> Option<&GroupRun> {
    let index = groups.partition_point(|run| run.first <= hour);
    index
        .checked_sub(1)
        .and_then(|i| groups.get(i))
        .filter(|run| hour <= run.last)
}
fn clipped(span: &HourSpan, first: i32, last: i32) -> Option<HourSpan> {
    let first_hour = span.first_hour.max(first);
    let last_hour = span.last_hour.min(last);
    (first_hour <= last_hour).then_some(HourSpan {
        first_hour,
        last_hour,
    })
}
fn identity((slot, id): Identity) -> String {
    format!("{slot}:{id}")
}

/// A breakdown run as cached; `reply` writes it out for the wars it was grouped by.
struct Run {
    first_hour: i32,
    last_hour: i32,
    read_hour: i32,
    hourly: bool,
    grouping_hour: i32,
    grouping_hourly: bool,
    enemy_hour: i32,
    enemy_hourly: bool,
    value: i64,
    /// The war's index in the grouping reply.
    war: Option<usize>,
    /// Without a war: whether the native view places the relation in several.
    ambiguous: bool,
    incomplete: bool,
}
impl Run {
    fn reply(&self, hours: HourSpan, wars: &WarsReply) -> CasualtyBreakdownRun {
        CasualtyBreakdownRun {
            first_hour: hours.first_hour,
            last_hour: hours.last_hour,
            read_hour: self.read_hour,
            hourly: self.hourly,
            grouping_hour: self.grouping_hour,
            grouping_hourly: self.grouping_hourly,
            enemy_hour: self.enemy_hour,
            enemy_hourly: self.enemy_hourly,
            value: self.value.to_string(),
            war_id: self.war.map(|index| wars.wars[index].id.clone()),
            reason: self.war.is_none().then(|| {
                if self.ambiguous {
                    "The native view places this relation in multiple war episodes.".into()
                } else {
                    "The native war grouping was not recorded for this relation observation.".into()
                }
            }),
            incomplete: self.incomplete,
        }
    }
}
/// A series as cached: its wire fields without runs, and its runs.
struct Series {
    head: CasualtyBreakdownSeries,
    runs: Vec<Run>,
}
struct Projection {
    countries: Vec<CasualtyBreakdownCountryRun>,
    series: Vec<Series>,
}
/// The country, wars reply and casualties a projection was made from.
#[cfg(any(test, feature = "test-support"))]
struct ProjectionKey {
    tag: String,
    wars: Arc<WarsReply>,
    values: Arc<ScannedCasualties>,
}
#[cfg(any(test, feature = "test-support"))]
impl PartialEq for ProjectionKey {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag
            && Arc::ptr_eq(&self.wars, &other.wars)
            && Arc::ptr_eq(&self.values, &other.values)
    }
}
/// Each branch's latest projection; an earlier one lives only while a request holds it.
#[cfg(any(test, feature = "test-support"))]
type Cache = dashboard_base::latest::BranchLatest<ProjectionKey, Projection>;
/// One holder continuity walk, shared with branch-wide prepared validation.
fn country_runs(
    changes: BTreeMap<i64, Vec<(Identity, bool, i32)>>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<CasualtyBreakdownCountryRun>> {
    let mut countries: Vec<CasualtyBreakdownCountryRun> = Vec::new();
    let mut active = BTreeMap::<Identity, (i32, i32)>::new();
    let mut changes = changes.into_iter().peekable();
    while let Some((first, events)) = changes.next() {
        check_cancel(cancelled)?;
        for (country, incomplete, delta) in events {
            let counts = active.entry(country).or_default();
            counts.0 += delta;
            counts.1 += if incomplete { delta } else { 0 };
            if counts.0 == 0 {
                active.remove(&country);
            }
        }
        let Some(&(next, _)) = changes.peek() else {
            continue;
        };
        if active.is_empty() {
            continue;
        }
        if active.len() != 1 {
            return Err(io::Error::other("Overlapping casualty country holders"));
        }
        let (&country, &(_, missing)) = active.iter().next().expect("one holder");
        let first = first as i32;
        let last = (next - 1) as i32;
        let name = identity(country);
        if let Some(previous) = countries.last_mut()
            && previous.last_hour.checked_add(1) == Some(first)
            && previous.identity == name
            && previous.incomplete == (missing != 0)
        {
            previous.last_hour = last;
        } else {
            countries.push(CasualtyBreakdownCountryRun {
                first_hour: first,
                last_hour: last,
                identity: name,
                incomplete: missing != 0,
            });
        }
    }
    Ok(countries)
}

fn project<'a>(
    records: impl Iterator<Item = (&'a pairs::Key, &'a [pairs::PairRead])> + Clone,
    wars: &WarsReply,
    selected_identities: &HashSet<Identity>,
    cancelled: &AtomicBool,
    countries: Option<Vec<CasualtyBreakdownCountryRun>>,
) -> io::Result<Projection> {
    let countries = match countries {
        Some(countries) => countries,
        None => {
            // Holder spans use hourly native lifetime reads, including a country's
            // first relation and losses against an enemy that no longer exists.
            let mut changes = BTreeMap::<i64, Vec<(Identity, bool, i32)>>::new();
            for ((_, _, first, second, _), reads) in records.clone() {
                check_cancel(cancelled)?;
                let (country, take_first) = if selected_identities.contains(first) {
                    (*first, true)
                } else if selected_identities.contains(second) {
                    (*second, false)
                } else {
                    continue;
                };
                for read in reads {
                    if !(if take_first {
                        read.first_live
                    } else {
                        read.second_live
                    }) {
                        continue;
                    }
                    changes
                        .entry(i64::from(read.first_hour))
                        .or_default()
                        .push((country, read.incomplete, 1));
                    changes
                        .entry(i64::from(read.last_hour) + 1)
                        .or_default()
                        .push((country, read.incomplete, -1));
                }
            }
            country_runs(changes, cancelled)?
        }
    };
    let mut reply = Projection {
        countries,
        series: Vec::new(),
    };
    let mut holders = HashMap::<Identity, Vec<HourSpan>>::new();
    for ((_, _, first, second, _), _) in records.clone() {
        let country = if selected_identities.contains(first) {
            *first
        } else if selected_identities.contains(second) {
            *second
        } else {
            continue;
        };
        holders.entry(country).or_insert_with(|| {
            let name = identity(country);
            reply
                .countries
                .iter()
                .filter(|run| run.identity == name)
                .map(|run| HourSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                })
                .collect()
        });
    }
    let selected: HashSet<_> = records
        .clone()
        .map(|(key, _)| key)
        .filter(|(_, _, first, second, _)| {
            selected_identities.contains(first) || selected_identities.contains(second)
        })
        .map(|(_, _, first, second, start)| WarRelationKey::new(*first, *second, *start))
        .collect();
    let mut memberships = HashMap::<WarRelationKey, Vec<(i32, i32, usize)>>::new();
    for (index, war) in wars.wars.iter().enumerate() {
        check_cancel(cancelled)?;
        for run in &war.relations {
            if selected.contains(&run.key) {
                memberships.entry(run.key.clone()).or_default().push((
                    run.first_hour,
                    run.last_hour,
                    index,
                ));
            }
        }
    }
    for ((a, b, first, second, start), reads) in records {
        check_cancel(cancelled)?;
        let (country, enemy, take_first) = if selected_identities.contains(first) {
            (*first, b, true)
        } else if selected_identities.contains(second) {
            (*second, a, false)
        } else {
            continue;
        };
        let Some(holders) = holders.get(&country) else {
            continue;
        };
        let key = WarRelationKey::new(*first, *second, *start);
        let groups = group_runs(memberships.get(&key).map_or(&[], Vec::as_slice));
        let mut runs = Vec::new();
        for read in reads {
            let index = holders.partition_point(|run| run.last_hour < read.first_hour);
            for holder in holders[index..]
                .iter()
                .take_while(|run| run.first_hour <= read.last_hour)
            {
                let span =
                    clipped(holder, read.first_hour, read.last_hour).expect("overlapping holder");
                let mut first = span.first_hour;
                while first <= span.last_hour {
                    let grouping_hour = if read.relation_hourly {
                        first
                    } else {
                        read.relation_read_hour
                    };
                    let group = group_at(&groups, grouping_hour);
                    let last = if read.relation_hourly {
                        group.map_or_else(
                            || {
                                groups
                                    .iter()
                                    .find(|run| run.first > first)
                                    .map_or(span.last_hour, |run| span.last_hour.min(run.first - 1))
                            },
                            |run| span.last_hour.min(run.last),
                        )
                    } else {
                        span.last_hour
                    };
                    runs.push(Run {
                        first_hour: first,
                        last_hour: last,
                        read_hour: if read.hourly { first } else { read.read_hour },
                        hourly: read.hourly,
                        grouping_hour,
                        grouping_hourly: read.relation_hourly,
                        enemy_hour: if take_first {
                            read.second_tag_hour
                        } else {
                            read.first_tag_hour
                        },
                        enemy_hourly: if take_first {
                            read.second_live
                        } else {
                            read.first_live
                        },
                        value: if take_first {
                            read.first_taken
                        } else {
                            read.second_taken
                        },
                        war: group.and_then(|run| run.war),
                        ambiguous: group.is_some_and(|run| run.ambiguous),
                        incomplete: read.incomplete,
                    });
                    let Some(next) = last.checked_add(1) else {
                        break;
                    };
                    first = next;
                }
            }
        }
        if !runs.is_empty() {
            reply.series.push(Series {
                head: CasualtyBreakdownSeries {
                    id: serde_json::to_string(&(first, second, start, a.as_ref(), b.as_ref()))
                        .map_err(io::Error::other)?,
                    country_identity: identity(country),
                    enemy_tag: enemy.to_string(),
                    enemy_identity: identity(if take_first { *second } else { *first }),
                    runs: Vec::new(),
                },
                runs,
            });
        }
    }
    Ok(reply)
}

impl Product for CasualtyBreakdown {
    const KEY: &'static str = "casualty-breakdown";
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&super::served::PLAN);
    const CONTENT_DEPENDENT: bool = true;
    type Params = CasualtyBreakdownParams;
    type Reply = CasualtyBreakdownReply;
    fn types() -> Vec<String> {
        vec![
            CasualtyBreakdownParams::decl(),
            CasualtyBreakdownRun::decl(),
            CasualtyBreakdownSeries::decl(),
            CasualtyBreakdownCountryRun::decl(),
            CasualtyWarNameRun::decl(),
            CasualtyBreakdownWar::decl(),
            CasualtyBreakdownReply::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        p.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid casualty branch")?;
        validate_tag(&p.tag)
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(p).map_err(io::Error::other)?;
        let branch = p.branch_id.parse().expect("validated");
        let values = scanned_branch(source, branch, cancelled)?;
        let selected = values.selected_identities(std::slice::from_ref(&p.tag));
        let wars = Wars::shared(
            source,
            &WarsParams {
                branch_id: p.branch_id.clone(),
                viewpoint_tag: Some(p.tag.clone()),
            },
            Some(&selected),
            context,
            cancelled,
        )?;
        let windows = source.block_windows(branch).to_vec();
        if p.window.is_none() {
            return build_reply(
                p,
                windows,
                &values.observed,
                values.incomplete,
                &wars,
                None,
                cancelled,
            );
        }
        if p.window.is_some_and(|window| window >= windows.len()) {
            return Err(dashboard_core::invalid("Unknown casualty window"));
        }
        let key = ProjectionKey {
            tag: p.tag.clone(),
            wars: wars.clone(),
            values: values.clone(),
        };
        let state = source.state::<Cache>();
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        let projection = match state.get(branch, &key) {
            Some(projection) => projection,
            None => {
                let projection = Arc::new(project(
                    values.pairs.runs(),
                    &wars,
                    &selected,
                    cancelled,
                    None,
                )?);
                state.put(branch, key, &projection);
                projection
            }
        };
        drop(state);
        build_reply(
            p,
            windows,
            &values.observed,
            values.incomplete,
            &wars,
            Some(&projection),
            cancelled,
        )
    }
}

fn build_reply(
    p: &CasualtyBreakdownParams,
    windows: Vec<HourSpan>,
    observed: &[HourSpan],
    incomplete: bool,
    wars: &WarsReply,
    projection: Option<&Projection>,
    cancelled: &AtomicBool,
) -> io::Result<CasualtyBreakdownReply> {
    let mut reply = CasualtyBreakdownReply { branch_id: p.branch_id.clone(), tag: p.tag.clone(),
        available: wars.available && !observed.is_empty(), windows, window: p.window,
        observed: observed.to_vec(), countries: Vec::new(), series: Vec::new(), wars: Vec::new(), has_gap: incomplete,
        warnings: vec!["Known losses only: losses with no recorded attacker cannot be assigned to an enemy. Land, naval and air are combined; the recording has no domain split.".into()] };
    reply.warnings.extend(wars.warnings.iter().cloned());
    let Some(window) = p.window else {
        check_reply_bytes(json_bytes(&reply)?)?;
        return Ok(reply);
    };
    let span = reply
        .windows
        .get(window)
        .ok_or_else(|| io::Error::other("Unknown casualty window"))?;
    let projection =
        projection.ok_or_else(|| dashboard_core::invalid("Missing casualty projection"))?;
    reply.observed = observed
        .iter()
        .filter_map(|run| clipped(span, run.first_hour, run.last_hour))
        .collect();
    reply.countries = projection
        .countries
        .iter()
        .filter_map(|run| {
            clipped(span, run.first_hour, run.last_hour).map(|hours| CasualtyBreakdownCountryRun {
                first_hour: hours.first_hour,
                last_hour: hours.last_hour,
                ..run.clone()
            })
        })
        .collect();
    let mut used_wars = HashSet::new();
    for series in &projection.series {
        check_cancel(cancelled)?;
        let first = series
            .runs
            .partition_point(|run| run.last_hour < span.first_hour);
        let runs: Vec<_> = series.runs[first..]
            .iter()
            .take_while(|run| run.first_hour <= span.last_hour)
            .filter_map(|run| {
                clipped(span, run.first_hour, run.last_hour).map(|hours| {
                    if let Some(index) = run.war {
                        used_wars.insert(&wars.wars[index].id);
                    }
                    run.reply(hours, wars)
                })
            })
            .collect();
        if !runs.is_empty() {
            reply.series.push(CasualtyBreakdownSeries {
                runs,
                ..series.head.clone()
            });
        }
    }
    for war in wars.wars.iter().filter(|war| used_wars.contains(&war.id)) {
        // Ended relations retain the name supplied with their last grouping;
        // no future observation labels an earlier value.
        let mut names = Vec::new();
        for observed in war
            .observations
            .iter()
            .filter(|run| run.first_hour <= span.last_hour)
        {
            names.push(CasualtyWarNameRun {
                first_hour: observed.first_hour,
                last_hour: observed.last_hour.min(span.last_hour),
                name: observed.name.clone(),
            });
        }
        reply.wars.push(CasualtyBreakdownWar {
            id: war.id.clone(),
            first_hour: war.first_hour,
            last_hour: war.last_hour.min(span.last_hour),
            started_by: war.started_by,
            ended_by: war.ended_by || war.last_hour > span.last_hour,
            names,
        });
    }
    check_reply_bytes(json_bytes(&reply)?)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_overlaps_are_unattributed_and_later_membership_does_not_backfill() {
        let groups = group_runs(&[(100, 103, 0), (102, 104, 1), (106, 107, 2)]);
        assert_eq!(group_at(&groups, 100).unwrap().war, Some(0));
        assert!(group_at(&groups, 102).unwrap().ambiguous);
        assert_eq!(group_at(&groups, 104).unwrap().war, Some(1));
        assert_eq!(group_at(&groups, 105).unwrap().war, None);
        assert!(group_at(&groups, 99).is_none());
        assert_eq!(group_at(&groups, 106).unwrap().war, Some(2));
    }
    #[test]
    fn selection_follows_the_native_holder_through_a_rename_without_alias_replacement() {
        let mut scan = Scan::new();
        let a = scan.country((1, 10));
        let b = scan.country((2, 20));
        let mut histories = pairs::PairHistory::default();
        for (hour, tag, amount) in [(100, "initial", 12), (101, "renamed", 13)] {
            let live = BTreeMap::from([(tag.into(), a), ("enemy".into(), b)]);
            scan.begin_hour(hour, &live, true, true);
            scan.relation(a, b, 90, [amount, 0, 0, 0]);
            scan.end_hour(true);
            histories.observe(&scan);
        }
        let replacement = scan.country((1, 11));
        let live = BTreeMap::from([("renamed".into(), replacement), ("enemy".into(), b)]);
        scan.begin_hour(102, &live, true, true);
        scan.relation(replacement, b, 102, [100, 0, 0, 0]);
        scan.end_hour(true);
        histories.observe(&scan);
        let mut values = scan.finish(&AtomicBool::new(false)).unwrap();
        values.pairs = histories;
        let wars = WarsReply {
            branch_id: "1".into(),
            available: true,
            viewpoints: Vec::new(),
            country_names: Vec::new(),
            wars: Vec::new(),
            unobserved: Vec::new(),
            capitulations_unobserved: Vec::new(),
            warnings: Vec::new(),
        };
        let original = project(
            values.pairs.runs(),
            &wars,
            &values.selected_identities(&["initial".into()]),
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        assert_eq!(original.countries[0].first_hour, 100);
        assert_eq!(original.countries.last().unwrap().last_hour, 101);
        assert!(original.series.iter().any(|series| {
            series
                .runs
                .iter()
                .any(|run| run.first_hour == 101 && run.value == 13)
        }));
        let alias = project(
            values.pairs.runs(),
            &wars,
            &values.selected_identities(&["renamed".into()]),
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        assert!(alias.series.iter().any(|series| {
            series
                .runs
                .iter()
                .any(|run| run.first_hour == 100 && run.value == 12)
        }));
        assert_eq!(alias.countries.last().unwrap().identity, "1:11");
        assert_eq!(
            values.selected_identities(&["initial".into()]),
            HashSet::from([(1, 10)])
        );
    }
    #[test]
    fn a_peaceful_tag_selects_its_renamed_first_war_but_not_an_alias_replacement() {
        let mut scan = Scan::new();
        let holder = scan.country((1, 10));
        let enemy = scan.country((2, 20));
        let mut histories = pairs::PairHistory::default();
        for (hour, tag) in [(100, "peaceful"), (101, "renamed")] {
            let live = BTreeMap::from([(tag.into(), holder), ("enemy".into(), enemy)]);
            scan.begin_hour(hour, &live, true, true);
            if hour == 101 {
                scan.relation(holder, enemy, 101, [12, 0, 0, 0]);
            }
            scan.end_hour(true);
            histories.observe(&scan);
        }
        let replacement = scan.country((1, 11));
        let live = BTreeMap::from([("renamed".into(), replacement), ("enemy".into(), enemy)]);
        scan.begin_hour(102, &live, true, true);
        scan.relation(replacement, enemy, 102, [100, 0, 0, 0]);
        scan.end_hour(true);
        histories.observe(&scan);
        assert!(
            histories
                .runs()
                .all(|((a, b, _, _, _), _)| a.as_ref() != "peaceful" && b.as_ref() != "peaceful")
        );
        let mut values = scan.finish(&AtomicBool::new(false)).unwrap();
        values.pairs = histories;
        assert_eq!(
            values.selected_identities(&["peaceful".into()]),
            HashSet::from([(1, 10)])
        );
        assert_eq!(
            values.selected_identities(&["renamed".into()]),
            HashSet::from([(1, 10), (1, 11)])
        );
        let wars = WarsReply {
            branch_id: "1".into(),
            available: true,
            viewpoints: Vec::new(),
            country_names: Vec::new(),
            wars: Vec::new(),
            unobserved: Vec::new(),
            capitulations_unobserved: Vec::new(),
            warnings: Vec::new(),
        };
        let reply = project(
            values.pairs.runs(),
            &wars,
            &values.selected_identities(&["peaceful".into()]),
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        assert_eq!(reply.countries.len(), 1);
        assert_eq!(
            (reply.countries[0].first_hour, reply.countries[0].last_hour),
            (101, 101)
        );
        assert_eq!(reply.series.len(), 1);
        assert_eq!(reply.series[0].runs[0].value, 12);
        assert_eq!(reply.series[0].head.country_identity, "1:10");
    }
    #[test]
    fn projections_keep_each_branch_s_latest() {
        use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
        use replay_data::{
            rows::CountryLinkRow, rows::CountryRow, rows::TableRows, rows::WarRelationRow,
        };
        use replay_frame::frame::TableView;
        let mut frame = source(1, 100, None);
        frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
            CountryRow {
                id: 10,
                canonical_slot: 1,
                tag: b"selected".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 20,
                canonical_slot: 2,
                tag: b"enemy".to_vec(),
                ..Default::default()
            },
        ]))
        .into();
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 10,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 20,
                    canonical_slot: 2,
                },
            ]))
            .into();
        frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                first_country: 10,
                second_country: 20,
                first_casualties: 12,
                ..Default::default()
            }]))
            .into();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "breakdown".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap();
        let params = CasualtyBreakdownParams {
            branch_id: "1".into(),
            tag: "selected".into(),
            window: Some(0),
        };
        let context = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);
        CasualtyBreakdown::load(&source, &params, &context, &cancelled).unwrap();
        let state = source.state::<Cache>();
        assert_eq!(state.lock().unwrap().held(), 1);
        CasualtyBreakdown::load(&source, &params, &context, &cancelled).unwrap();
        assert_eq!(state.lock().unwrap().held(), 1);
    }
    #[test]
    fn a_first_relation_updates_breakdown_and_country_summary_at_the_same_hour() {
        let mut scan = Scan::new();
        let a = scan.country((1, 10));
        let b = scan.country((2, 20));
        let live = BTreeMap::from([("selected".into(), a), ("enemy".into(), b)]);
        let mut histories = pairs::PairHistory::default();
        scan.begin_hour(100, &live, true, true);
        scan.end_hour(true);
        histories.observe(&scan);
        scan.begin_hour(101, &live, false, true);
        scan.relation(a, b, 101, [4, 0, 0, 0]);
        scan.end_hour(true);
        histories.observe(&scan);
        assert_eq!(scan.tracks["selected"].at(101).unwrap().taken, 4);
        scan.begin_hour(102, &live, false, true);
        scan.end_hour(true);
        histories.observe(&scan);
        let mut values = scan.finish(&AtomicBool::new(false)).unwrap();
        values.pairs = histories;
        let wars = WarsReply {
            branch_id: "1".into(),
            available: true,
            viewpoints: Vec::new(),
            country_names: Vec::new(),
            wars: Vec::new(),
            unobserved: Vec::new(),
            capitulations_unobserved: Vec::new(),
            warnings: Vec::new(),
        };
        let reply = project(
            values.pairs.runs(),
            &wars,
            &values.selected_identities(&["selected".into()]),
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        assert_eq!(values.country_at("selected", 101).unwrap().read_hour, 101);
        assert_eq!(reply.countries[0].first_hour, 101);
        assert_eq!(reply.series[0].runs[0].first_hour, 101);
        assert_eq!(reply.series[0].runs[0].value, 4);
    }
}
