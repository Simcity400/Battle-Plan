//! Observation-time group runs, corrected only over changed classification windows.
use super::*;

const MEASURES: [&str; 4] = ["stock", "balance", "need", "drawable"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockpileChoicePeak {
    pub raw: String,
    pub stock_drawable: bool,
    pub share_drawable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Kind {
    key: String,
    domain: Domain,
    requires_available: bool,
}

/// Fixed authored classification, shared by every holder in one content binding.
#[derive(Clone, Default)]
pub struct Content {
    database: Option<Arc<Database>>,
    kinds: Option<Arc<catalogue::KeyedResults<String, Kind>>>,
}
impl Content {
    pub(crate) fn load(
        source: &Source,
        options: Option<&dashboard_core::config::ContentOptions>,
    ) -> Self {
        let Some((catalogue, database)) = options
            .and_then(|options| catalogue::catalogue(source, options).ok())
            .and_then(|catalogue| {
                equipment::units(source, &catalogue)
                    .ok()
                    .map(|database| (catalogue, database))
            })
        else {
            return Self::default();
        };
        let kinds = catalogue
            .content
            .layers
            .derived::<catalogue::KeyedResults<String, Kind>>();
        Self {
            database: Some(database),
            kinds: Some(kinds),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Classification {
    token: String,
    kind: Option<Kind>,
}
impl Classification {
    fn groups(&self, id: Option<u64>) -> [String; 2] {
        let key = self.kind.as_ref().map_or_else(
            || {
                format!(
                    "{UNRESOLVED_MARK}{}",
                    if self.token.is_empty() {
                        id.map(|id| id.to_string()).unwrap_or_default()
                    } else {
                        self.token.clone()
                    }
                )
            },
            |kind| kind.key.clone(),
        );
        let domain = self
            .kind
            .as_ref()
            .map_or("unresolved", |kind| match kind.domain {
                Domain::Land => "land",
                Domain::Air => "air",
                Domain::Naval => "naval",
            });
        [
            series::field_key("type", &key),
            series::field_key("domain", domain),
        ]
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct ClassifiedSpan {
    first: i32,
    last: i32,
    classification: Classification,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Metadata {
    current: BTreeMap<u64, ClassifiedSpan>,
    first: Option<i32>,
    last: i32,
}

impl Metadata {
    pub(super) fn bounds(&self) -> (i32, i32) {
        (self.first.unwrap_or(self.last), self.last)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Choices {
    metadata: Metadata,
    intervals: Accumulator,
    values: [Accumulator; 4],
    active_groups: BTreeMap<String, BTreeSet<String>>,
    completeness: BTreeMap<String, [bool; 3]>,
    logistics: BTreeMap<String, Logistics>,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) work: dashboard_base::prepared::observer_tests::Work,
    #[serde(skip)]
    content: Content,
    /// Each classified variant's two groups in the current content binding.
    #[serde(skip)]
    variant_groups: BTreeMap<u64, [String; 2]>,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct LogisticsEvidence {
    own: usize,
    unresolved: usize,
    required: usize,
}
#[derive(Default, Serialize, Deserialize)]
struct Logistics {
    representatives: BTreeMap<String, (u32, Classification)>,
    witnesses: BTreeMap<String, BTreeMap<u32, Classification>>,
    groups: BTreeMap<String, LogisticsEvidence>,
    #[serde(skip)]
    by_id: BTreeMap<u32, String>,
    #[serde(skip)]
    ready: bool,
    #[serde(skip)]
    dirty: BTreeSet<String>,
}
impl Logistics {
    fn evidence(&mut self, classification: &Classification, add: bool) {
        for group in classification.groups(None) {
            self.dirty.insert(group.clone());
            let evidence = self.groups.entry(group).or_default();
            let update = |value: &mut usize, applies: bool| {
                if applies {
                    if add {
                        *value += 1;
                    } else {
                        *value -= 1;
                    }
                }
            };
            update(&mut evidence.own, true);
            update(&mut evidence.unresolved, classification.kind.is_none());
            update(
                &mut evidence.required,
                classification
                    .kind
                    .as_ref()
                    .is_none_or(|kind| kind.requires_available),
            );
        }
    }
    fn refresh_representative(&mut self, group: &str) {
        let next = self
            .witnesses
            .get(group)
            .and_then(|witnesses| witnesses.last_key_value())
            .map(|(&id, class)| (id, class.clone()));
        if self.representatives.get(group) == next.as_ref() {
            return;
        }
        if let Some((_, old)) = self.representatives.remove(group) {
            self.evidence(&old, false);
        }
        if let Some((id, class)) = next {
            self.evidence(&class, true);
            self.representatives.insert(group.to_owned(), (id, class));
        } else {
            self.witnesses.remove(group);
        }
    }
    fn observe(&mut self, id: u32, classification: Classification) {
        if !self.ready {
            self.by_id = self
                .witnesses
                .iter()
                .flat_map(|(group, witnesses)| witnesses.keys().map(move |&id| (id, group.clone())))
                .collect();
            self.ready = true;
        }
        let group = classification.groups(None)[0].clone();
        if let Some(old_group) = self.by_id.get(&id).cloned() {
            if old_group == group && self.witnesses[&group].get(&id) == Some(&classification) {
                return;
            }
            if old_group != group {
                self.witnesses
                    .get_mut(&old_group)
                    .expect("native logistics type")
                    .remove(&id);
                self.refresh_representative(&old_group);
            }
        }
        self.witnesses
            .entry(group.clone())
            .or_default()
            .insert(id, classification);
        self.by_id.insert(id, group.clone());
        self.refresh_representative(&group);
    }
}

#[derive(Clone, Copy)]
struct Operand {
    value: Option<i128>,
    present: bool,
}

fn operand<K: Ord + Copy>(rows: &ReadRuns<K>, key: K, hour: i32) -> Operand {
    let known = rows
        .spans
        .iter()
        .any(|span| span.first_hour <= hour && hour <= span.last_hour);
    let run = rows.runs.get(&key).and_then(|runs| {
        runs.iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
    });
    Operand {
        present: run.is_some(),
        value: if !known {
            None
        } else {
            run.map_or(Some(0), |run| {
                run.amount_raw
                    .as_deref()
                    .map(|raw| raw.parse().expect("ReadRuns integer"))
            })
        },
    }
}

fn group_key(group: &str, measure: &str) -> String {
    format!("group-{measure}:{group}")
}

/// Shared by observation and retrospective correction. Presence is separate
/// from zero: an unresolved zero logistics row still invalidates domain need.
fn evaluate(
    group: &str,
    variants: &BTreeMap<u64, Classification>,
    logistics: &BTreeMap<u32, Classification>,
    stock_known: bool,
    balance_known: bool,
    retained: Option<&Logistics>,
    mut read: impl FnMut(usize, u64) -> io::Result<Operand>,
) -> io::Result<[Option<i128>; 4]> {
    let (grouping, key): (String, String) = serde_json::from_str(group)?;
    let mut stock = stock_known.then_some(0_i128);
    for (&id, classification) in variants {
        let value = read(0, id)?;
        if classification.kind.is_none() && value.present && value.value != Some(0) {
            stock = None;
        }
        if classification
            .groups(Some(id))
            .iter()
            .any(|key| key == group)
        {
            stock = stock.and_then(|sum| value.value.map(|value| sum + value));
        }
    }
    // The UI key map uses the last logistics token for a duplicate archetype.
    let types: BTreeMap<_, _> = logistics
        .iter()
        .filter(|&(&id, classification)| {
            retained.is_none_or(|retained| {
                retained
                    .representatives
                    .get(&classification.groups(None)[0])
                    .is_some_and(|(representative, _)| *representative == id)
            })
        })
        .map(|(&id, classification)| (classification.groups(None)[0].clone(), (id, classification)))
        .collect();
    let own: Vec<_> = types
        .values()
        .copied()
        .filter(|(_, classification)| classification.groups(None).iter().any(|key| key == group))
        .collect();
    let evidence = retained
        .map(|retained| retained.groups.get(group).copied().unwrap_or_default())
        .unwrap_or_else(|| LogisticsEvidence {
            own: own.len(),
            unresolved: own.iter().filter(|(_, class)| class.kind.is_none()).count(),
            required: own
                .iter()
                .filter(|(_, class)| {
                    class
                        .kind
                        .as_ref()
                        .is_none_or(|kind| kind.requires_available)
                })
                .count(),
        });
    if evidence.unresolved > 0 {
        stock = None;
    }
    let mut balance = (balance_known && evidence.own > 0).then_some(0_i128);
    let mut need = balance;
    let mut recorded_required = 0;
    for (id, classification) in own {
        let b = read(5, u64::from(id))?.value;
        let s = read(6, u64::from(id))?.value;
        balance = balance.and_then(|sum| b.zip(s).map(|(value, _)| sum + value));
        let available = read(7, u64::from(id))?;
        if classification
            .kind
            .as_ref()
            .is_none_or(|kind| kind.requires_available)
            && available.present
            && available.value.is_some()
        {
            recorded_required += 1;
        }
        let derived = b.zip(s).and_then(|(b, s)| {
            let available = available
                .present
                .then_some(available.value)
                .flatten()
                .or_else(|| {
                    classification
                        .kind
                        .as_ref()
                        .filter(|kind| !kind.requires_available)
                        .map(|_| s.max(0))
                })?;
            let value = (available as i64).wrapping_sub(b as i64);
            (value.unsigned_abs() < NEED_BOUND_RAW as u64).then_some(i128::from(value))
        });
        need = need.and_then(|sum| derived.map(|value| sum + value));
    }
    if recorded_required < evidence.required {
        need = None;
    }
    if grouping == "domain" && key != "unresolved" {
        for (&id, _) in logistics
            .iter()
            .filter(|(_, classification)| classification.kind.is_none())
        {
            let b = read(5, u64::from(id))?;
            let s = read(6, u64::from(id))?;
            if (b.present && b.value != Some(0)) || (s.present && s.value != Some(0)) {
                balance = None;
            }
            if b.present || s.present {
                need = None;
            }
        }
    }
    if balance.is_none() {
        need = None;
    }
    Ok([
        stock,
        balance,
        need,
        (stock_known || balance_known).then_some(i128::from(
            balance.is_some() && need.is_some_and(|need| need > 0),
        )),
    ])
}

/// [`evaluate`] of every group at once, from one pass over the members: a
/// variant adds to its two groups and a logistics representative to its type
/// and domain, so the work is the members and the groups, not their product.
/// Each group's sums are taken in [`evaluate`]'s order.
fn evaluate_groups(
    groups: &BTreeSet<String>,
    variants: &[(u64, &Classification, &[String; 2])],
    logistics: &[(u32, &Classification, [String; 2])],
    stock_known: bool,
    balance_known: bool,
    retained: &Logistics,
    read: impl Fn(usize, u64) -> Operand,
) -> io::Result<Vec<[Option<i128>; 4]>> {
    struct Sums {
        stock: Option<i128>,
        balance: Option<i128>,
        need: Option<i128>,
        recorded_required: usize,
        evidence: LogisticsEvidence,
    }
    let mut sums: BTreeMap<&str, Sums> = groups
        .iter()
        .map(|group| {
            let evidence = retained.groups.get(group).copied().unwrap_or_default();
            let balance = (balance_known && evidence.own > 0).then_some(0_i128);
            (
                group.as_str(),
                Sums {
                    stock: stock_known.then_some(0_i128),
                    balance,
                    need: balance,
                    recorded_required: 0,
                    evidence,
                },
            )
        })
        .collect();
    // An unresolved variant holding stock leaves every group's stock unknown.
    let mut unresolved_stock = false;
    for &(id, classification, keys) in variants {
        let value = read(0, id);
        if classification.kind.is_none() && value.present && value.value != Some(0) {
            unresolved_stock = true;
        }
        for key in keys {
            if let Some(sums) = sums.get_mut(key.as_str()) {
                sums.stock = sums
                    .stock
                    .and_then(|sum| value.value.map(|value| sum + value));
            }
        }
    }
    // The UI key map uses the last logistics token for a duplicate archetype.
    let types: BTreeMap<&str, (u32, &Classification, &[String; 2])> = logistics
        .iter()
        .filter(|(id, _, keys)| {
            retained
                .representatives
                .get(&keys[0])
                .is_some_and(|(representative, _)| representative == id)
        })
        .map(|(id, classification, keys)| (keys[0].as_str(), (*id, *classification, keys)))
        .collect();
    for &(id, classification, keys) in types.values() {
        let b = read(5, u64::from(id)).value;
        let s = read(6, u64::from(id)).value;
        let available = read(7, u64::from(id));
        let required = classification
            .kind
            .as_ref()
            .is_none_or(|kind| kind.requires_available)
            && available.present
            && available.value.is_some();
        let derived = b.zip(s).and_then(|(b, s)| {
            let available = available
                .present
                .then_some(available.value)
                .flatten()
                .or_else(|| {
                    classification
                        .kind
                        .as_ref()
                        .filter(|kind| !kind.requires_available)
                        .map(|_| s.max(0))
                })?;
            let value = (available as i64).wrapping_sub(b as i64);
            (value.unsigned_abs() < NEED_BOUND_RAW as u64).then_some(i128::from(value))
        });
        for key in keys {
            if let Some(sums) = sums.get_mut(key.as_str()) {
                sums.balance = sums
                    .balance
                    .and_then(|sum| b.zip(s).map(|(value, _)| sum + value));
                sums.recorded_required += usize::from(required);
                sums.need = sums.need.and_then(|sum| derived.map(|value| sum + value));
            }
        }
    }
    // Unresolved logistics rows leave every resolved domain's totals unknown.
    let (mut unresolved_balance, mut unresolved_need) = (false, false);
    for &(id, classification, _) in logistics {
        if classification.kind.is_some() {
            continue;
        }
        let b = read(5, u64::from(id));
        let s = read(6, u64::from(id));
        if (b.present && b.value != Some(0)) || (s.present && s.value != Some(0)) {
            unresolved_balance = true;
        }
        if b.present || s.present {
            unresolved_need = true;
        }
    }
    groups
        .iter()
        .map(|group| {
            let (grouping, key): (String, String) = serde_json::from_str(group)?;
            let Sums {
                mut stock,
                mut balance,
                mut need,
                recorded_required,
                evidence,
            } = sums
                .remove(group.as_str())
                .expect("evaluated stockpile group");
            if unresolved_stock || evidence.unresolved > 0 {
                stock = None;
            }
            if recorded_required < evidence.required {
                need = None;
            }
            if grouping == "domain" && key != "unresolved" {
                if unresolved_balance {
                    balance = None;
                }
                if unresolved_need {
                    need = None;
                }
            }
            if balance.is_none() {
                need = None;
            }
            Ok([
                stock,
                balance,
                need,
                (stock_known || balance_known).then_some(i128::from(
                    balance.is_some() && need.is_some_and(|need| need > 0),
                )),
            ])
        })
        .collect()
}

impl Numbers {
    /// A holder's three completeness keys, allocated together where they are
    /// first used.
    fn complete_keys(&mut self, tag: &str) -> [u64; 3] {
        let Numbers { keys, resolved, .. } = self;
        *holder_keys(resolved, tag).complete.get_or_insert_with(|| {
            std::array::from_fn(|slot| {
                allocate(
                    keys,
                    tag,
                    &group_key(&series::field_key("complete", ""), MEASURES[slot]),
                )
            })
        })
    }
    /// A holder's four measure keys of `group`, allocated together where they
    /// are first used.
    fn group_keys(&mut self, tag: &str, group: &str) -> [u64; 4] {
        let Numbers { keys, resolved, .. } = self;
        let holder = holder_keys(resolved, tag);
        if let Some(&found) = holder.groups.get(group) {
            return found;
        }
        let found = std::array::from_fn(|measure| {
            allocate(keys, tag, &group_key(group, MEASURES[measure]))
        });
        holder.groups.insert(group.to_owned(), found);
        found
    }
}

impl Choices {
    pub(super) fn advance(&mut self, hour: i32) -> io::Result<()> {
        self.metadata.first.get_or_insert(hour);
        self.metadata.last = hour;
        self.intervals.advance(hour)?;
        for values in &mut self.values {
            values.advance(hour)?;
        }
        Ok(())
    }
    /// [`Self::advance`] at every hour `first..=last`, in constant time.
    pub(super) fn advance_span(&mut self, first: i32, last: i32) -> io::Result<()> {
        self.metadata.first.get_or_insert(first);
        self.metadata.last = last;
        self.intervals.advance_span(first, last)?;
        for values in &mut self.values {
            values.advance_span(first, last)?;
        }
        Ok(())
    }
    pub(super) fn bind_content(&mut self, content: &Content) -> bool {
        fn same<T>(old: &Option<Arc<T>>, new: &Option<Arc<T>>) -> bool {
            match (old, new) {
                (Some(old), Some(new)) => Arc::ptr_eq(old, new),
                (None, None) => true,
                _ => false,
            }
        }
        let changed = !same(&self.content.database, &content.database)
            || !same(&self.content.kinds, &content.kinds);
        if changed {
            self.variant_groups.clear();
        }
        self.content = content.clone();
        changed
    }
    fn classification(&self, token: &str) -> Classification {
        let kind = self
            .content
            .database
            .as_ref()
            .zip(self.content.kinds.as_ref())
            .and_then(|(database, kinds)| {
                kinds
                    .get_or_init(&token.to_owned(), |token| {
                        let (key, _, domain) =
                            archetype_of(database, token).map_err(io::Error::other)?;
                        Ok(Kind {
                            requires_available: need_note(database, &key)
                                .map_or(true, |note| note.is_some()),
                            key,
                            domain,
                        })
                    })
                    .ok()
                    .map(|kind| (*kind).clone())
            });
        Classification {
            token: token.to_owned(),
            kind,
        }
    }
    pub(super) fn classify(
        &mut self,
        names: &VariantCatalogue,
        ids: &BTreeSet<u64>,
        hour: i32,
    ) -> io::Result<()> {
        self.metadata.first.get_or_insert(hour);
        self.metadata.last = hour;
        for &id in ids {
            self.classify_one(id, names.token(id), hour)?;
        }
        self.intervals.through(hour);
        Ok(())
    }
    fn classify_one(&mut self, id: u64, token: &str, hour: i32) -> io::Result<()> {
        if let Some(current) = self.metadata.current.get_mut(&id).filter(|span| {
            span.classification.token == token && self.variant_groups.contains_key(&id)
        }) {
            current.last = hour;
            return Ok(());
        }
        let classification = self.classification(token);
        let first = self.metadata.first.expect("stockpile first hour");
        let current = self
            .metadata
            .current
            .entry(id)
            .or_insert_with(|| ClassifiedSpan {
                first,
                last: hour,
                classification: Classification {
                    token: String::new(),
                    kind: None,
                },
            });
        if current.classification != classification {
            if current.first < hour {
                current.last = hour - 1;
                self.intervals.event(id, hour, current)?;
            }
            *current = ClassifiedSpan {
                first: hour,
                last: hour,
                classification,
            };
        } else {
            current.last = hour;
        }
        self.variant_groups
            .insert(id, current.classification.groups(Some(id)));
        Ok(())
    }
    /// Whether the variant's classification, and so its groups, changed at `hour`.
    pub(super) fn reclassified(&self, id: u64, hour: i32) -> bool {
        self.metadata
            .current
            .get(&id)
            .is_some_and(|span| span.first == hour)
    }
    pub(super) fn observe(
        &mut self,
        tag: &str,
        holder: &mut Holder,
        scanned: &ScannedStockpile,
        names: &VariantCatalogue,
        hour: i32,
        numbers: &mut Numbers,
    ) -> io::Result<()> {
        for &id in scanned.stock.runs.keys().chain(scanned.daily.runs.keys()) {
            if !self.variant_groups.contains_key(&id) {
                let classification = self.classification(names.token(id));
                if self
                    .metadata
                    .current
                    .get(&id)
                    .is_none_or(|span| span.classification != classification)
                {
                    self.classify_one(id, names.token(id), hour)?;
                } else {
                    self.variant_groups
                        .insert(id, classification.groups(Some(id)));
                }
            }
        }
        let current_logistics = self.logistics(tag, holder, scanned, hour);
        let ids: BTreeSet<u64> = scanned
            .stock
            .runs
            .keys()
            .chain(scanned.daily.runs.keys())
            .filter(|id| {
                [&scanned.stock, &scanned.daily].iter().any(|rows| {
                    rows.runs
                        .get(*id)
                        .and_then(|runs| runs.last())
                        .is_some_and(|run| run.last_hour == hour)
                })
            })
            .copied()
            .collect();
        let variants: Vec<(u64, &Classification, &[String; 2])> = ids
            .iter()
            .map(|id| {
                (
                    *id,
                    &self.metadata.current[id].classification,
                    &self.variant_groups[id],
                )
            })
            .collect();
        let logistics: Vec<(u32, &Classification, [String; 2])> = current_logistics
            .iter()
            .map(|(&id, classification)| (id, classification, classification.groups(None)))
            .collect();
        let retained = &self.logistics[tag];
        let current_groups: BTreeSet<&str> = variants
            .iter()
            .flat_map(|(_, _, groups)| groups.iter())
            .chain(logistics.iter().flat_map(|(_, _, groups)| groups.iter()))
            .map(String::as_str)
            .collect();
        let mut groups: BTreeSet<String> = current_groups
            .iter()
            .map(|group| (*group).to_owned())
            .chain(self.active_groups.get(tag).into_iter().flatten().cloned())
            .chain(retained.dirty.iter().cloned())
            .collect();
        let stock_known = scanned
            .stock
            .spans
            .last()
            .is_some_and(|span| span.last_hour == hour);
        let balance_known = scanned
            .balance
            .spans
            .last()
            .is_some_and(|span| span.last_hour == hour);
        // Before a group first appears its stock is zero only with complete coverage.
        let complete = stock_known
            && variants.iter().all(|&(id, classification, _)| {
                let value = operand(&scanned.stock, id, hour);
                classification.kind.is_some() || !value.present || value.value == Some(0)
            });
        let complete_keys = numbers.complete_keys(tag);
        observe_numeric(
            &mut self.values[0],
            complete_keys[0],
            hour,
            complete.then_some(0),
            scanned
                .stock
                .spans
                .last()
                .is_some_and(|span| span.first_hour == hour),
            false,
        );
        let mut complete_balance = balance_known;
        let mut complete_need = balance_known;
        for &(id, classification, _) in &logistics {
            if classification.kind.is_some() {
                continue;
            }
            let b = operand(&scanned.balance, id, hour);
            let v = operand(&scanned.logistics_stock, id, hour);
            if (b.present && b.value != Some(0)) || (v.present && v.value != Some(0)) {
                complete_balance = false;
            }
            if b.present || v.present {
                complete_need = false;
            }
        }
        for (slot, complete) in [
            (1, complete_balance),
            (2, complete_balance && complete_need),
        ] {
            observe_numeric(
                &mut self.values[slot],
                complete_keys[slot],
                hour,
                complete.then_some(0),
                scanned
                    .balance
                    .spans
                    .last()
                    .is_some_and(|span| span.first_hour == hour),
                false,
            );
        }
        let completeness = [
            complete,
            complete_balance,
            complete_balance && complete_need,
        ];
        let completeness_changed = match self.completeness.get_mut(tag) {
            Some(previous) => std::mem::replace(previous, completeness) != completeness,
            None => {
                self.completeness.insert(tag.to_owned(), completeness);
                true
            }
        };
        if completeness_changed
            || [&scanned.stock.spans, &scanned.balance.spans]
                .iter()
                .any(|spans| spans.last().is_none_or(|span| span.first_hour == hour))
        {
            groups.extend(holder.groups.keys().cloned());
        }
        let read = |field: usize, id: u64| match field {
            0 => operand(&scanned.stock, id, hour),
            5 => operand(&scanned.balance, id as u32, hour),
            6 => operand(&scanned.logistics_stock, id as u32, hour),
            7 => operand(&scanned.available, id as u32, hour),
            _ => unreachable!(),
        };
        // Coverage can change a group's totals while completeness stays the same.
        // Every observed holder evaluates its groups, including nonsupplying hours.
        let measured = evaluate_groups(
            &groups,
            &variants,
            &logistics,
            stock_known,
            balance_known,
            retained,
            &read,
        )?;
        // The one-pass totals are each group's own evaluation.
        #[cfg(test)]
        {
            let all: BTreeMap<_, _> = variants
                .iter()
                .map(|&(id, classification, _)| (id, classification.clone()))
                .collect();
            for (group, values) in groups.iter().zip(&measured) {
                let expected = evaluate(
                    group,
                    &all,
                    &current_logistics,
                    stock_known,
                    balance_known,
                    Some(retained),
                    |field, id| Ok(read(field, id)),
                )?;
                assert_eq!(*values, expected, "{tag} {group} at {hour}");
            }
        }
        for (group, values) in groups.iter().zip(&measured) {
            if !holder.groups.contains_key(group) {
                holder.groups.insert(group.clone(), hour);
            }
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let keys = numbers.group_keys(tag, group);
            for (measure, value) in values.iter().enumerate() {
                let coverage = if measure == 0 {
                    &scanned.stock.spans
                } else {
                    &scanned.balance.spans
                };
                observe_numeric(
                    &mut self.values[measure],
                    keys[measure],
                    hour,
                    *value,
                    coverage.last().is_some_and(|span| span.first_hour == hour),
                    false,
                );
            }
        }
        if self.active_groups.get(tag).is_none_or(|active| {
            !active
                .iter()
                .map(String::as_str)
                .eq(current_groups.iter().copied())
        }) {
            let current = current_groups.into_iter().map(str::to_owned).collect();
            self.active_groups.insert(tag.to_owned(), current);
        }
        for values in &mut self.values {
            values.through(hour);
        }
        Ok(())
    }
    fn classify_logistics(&self, holder: &Holder, id: u32) -> Classification {
        self.classification(
            &holder
                .tokens
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("token {id}")),
        )
    }
    fn logistics(
        &mut self,
        tag: &str,
        holder: &Holder,
        scanned: &ScannedStockpile,
        hour: i32,
    ) -> BTreeMap<u32, Classification> {
        self.logistics
            .entry(tag.to_owned())
            .or_default()
            .dirty
            .clear();
        let ids: BTreeSet<_> = [
            &scanned.balance,
            &scanned.logistics_stock,
            &scanned.available,
        ]
        .into_iter()
        .flat_map(|rows| {
            rows.runs.iter().filter_map(|(&id, runs)| {
                runs.last()
                    .is_some_and(|run| run.last_hour == hour)
                    .then_some(id)
            })
        })
        .collect();
        let mut current = BTreeMap::new();
        for id in ids {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let classification = self.classify_logistics(holder, id);
            self.logistics
                .entry(tag.to_owned())
                .or_default()
                .observe(id, classification.clone());
            current.insert(id, classification);
        }
        self.logistics.entry(tag.to_owned()).or_default();
        current
    }
    pub(super) fn bytes(&self) -> usize {
        self.intervals.bytes() + self.values.iter().map(Accumulator::bytes).sum::<usize>()
    }
    pub(super) fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub(super) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.intervals.flush(
            writer,
            &format!("stockpile-{branch}-classification"),
            0,
            cancelled,
        )?;
        for (index, values) in self.values.iter_mut().enumerate() {
            values.flush(
                writer,
                &format!("stockpile-{branch}-group-{}", MEASURES[index]),
                0,
                cancelled,
            )?;
        }
        Ok(())
    }
}

fn affected(
    group: &str,
    history: &BTreeMap<u64, Vec<ClassifiedSpan>>,
    final_names: &BTreeMap<u64, Classification>,
) -> Vec<HourSpan> {
    let mut spans: Vec<_> = history
        .iter()
        .flat_map(|(&id, history)| {
            history.iter().filter_map(move |span| {
                let final_name = &final_names[&id];
                (span.classification != *final_name
                    && (span.classification.kind.is_none()
                        || final_name.kind.is_none()
                        || span
                            .classification
                            .groups(Some(id))
                            .iter()
                            .any(|key| key == group)
                        || final_name.groups(Some(id)).iter().any(|key| key == group)))
                .then_some(HourSpan {
                    first_hour: span.first,
                    last_hour: span.last,
                })
            })
        })
        .collect();
    spans.sort_by_key(|span| span.first_hour);
    let mut union: Vec<HourSpan> = Vec::new();
    for span in spans {
        if let Some(last) = union
            .last_mut()
            .filter(|last| i64::from(last.last_hour) + 1 >= i64::from(span.first_hour))
        {
            last.last_hour = last.last_hour.max(span.last_hour);
        } else {
            union.push(span);
        }
    }
    union
}

fn pieces(within: &[HourSpan], affected: &[HourSpan]) -> Vec<(i32, i32, bool)> {
    let mut result = Vec::new();
    for span in within {
        let mut first = i64::from(span.first_hour);
        for changed in affected.iter().filter(|changed| {
            changed.last_hour >= span.first_hour && changed.first_hour <= span.last_hour
        }) {
            let start = first.max(i64::from(changed.first_hour));
            let last = i64::from(span.last_hour.min(changed.last_hour));
            if first < start {
                result.push((first as i32, (start - 1) as i32, false));
            }
            result.push((start as i32, last as i32, true));
            first = last + 1;
        }
        if first <= i64::from(span.last_hour) {
            result.push((first as i32, span.last_hour, false));
        }
    }
    result
}

fn exact_max(
    reader: &mut Reader<'_>,
    key: u64,
    bucket: &dashboard_base::prepared_chart::ChartBucket,
) -> io::Result<i128> {
    const EXACT_LIMIT: f64 = 9_007_199_254_740_992.0;
    if bucket.min_value.abs() < EXACT_LIMIT && bucket.max_value.abs() < EXACT_LIMIT {
        return reader
            .value_at(key, bucket.max_hour)?
            .map(|run| run.value)
            .ok_or_else(|| dashboard_core::invalid("Missing stockpile maximum"));
    }
    reader
        .runs(key, bucket.first_hour, bucket.last_hour)?
        .iter()
        .map(|run| run.value)
        .max()
        .ok_or_else(|| dashboard_core::invalid("Missing stockpile maximum"))
}

pub(super) struct Query<'a> {
    keys: &'a BTreeMap<String, u64>,
    tag: &'a str,
    holder: &'a Holder,
    variants: BTreeMap<u64, Classification>,
    logistics: BTreeMap<u32, Classification>,
    history: BTreeMap<u64, Vec<ClassifiedSpan>>,
    aggregates: [Reader<'a>; 4],
    amounts: [Reader<'a>; 4],
    present: [Reader<'a>; 4],
    coverage: Reader<'a>,
    bounds: (i32, i32),
    included: &'a [String],
}
impl<'a> Query<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        source: &Source,
        request: &'a Request,
        context: &LoadContext,
        store: &'a store::Store,
        key: &'a store::SourceKey,
        keys: &'a BTreeMap<String, u64>,
        holder: &'a Holder,
        metadata: &Metadata,
        cancelled: &'a AtomicBool,
    ) -> io::Result<Self> {
        let branch = CountryParams {
            branch_id: request.branch_id.clone(),
            tag: request.tag.clone(),
        }
        .branch();
        let mut classifier = Choices::default();
        classifier.bind_content(&Content::load(source, context.content.as_ref()));
        let variants = holder
            .variants
            .iter()
            .filter_map(|id| {
                metadata
                    .current
                    .get(id)
                    .map(|span| (*id, span.classification.clone()))
            })
            .collect();
        let mut history = BTreeMap::new();
        let mut intervals = Reader::new(
            store,
            key,
            format!("stockpile-{branch}-classification"),
            cancelled,
        )?;
        for &id in &holder.variants {
            let mut spans = Vec::new();
            let mut cursor = 0;
            loop {
                let page = intervals.events(&[id], i32::MIN, i32::MAX, cursor, 256)?;
                for row in page.rows {
                    spans.push(serde_json::from_value::<ClassifiedSpan>(row.row)?);
                }
                let Some(next) = page.cursor else { break };
                cursor = next;
            }
            if let Some(current) = metadata.current.get(&id) {
                let mut current = current.clone();
                current.last = metadata.last;
                spans.push(current);
            }
            history.insert(id, spans);
        }
        let [a, b, c, d] = MEASURES.map(|field| {
            Reader::new(
                store,
                key,
                format!("stockpile-{branch}-group-{field}"),
                cancelled,
            )
        });
        let aggregates = [a?, b?, c?, d?];
        let [a, b, c, d] = ["stock", "balance", "logistics-stock", "available"]
            .map(|field| Reader::new(store, key, format!("stockpile-{branch}-{field}"), cancelled));
        let amounts = [a?, b?, c?, d?];
        let [a, b, c, d] = ["stock", "balance", "logistics-stock", "available"].map(|field| {
            Reader::new(
                store,
                key,
                format!("stockpile-{branch}-{field}-present"),
                cancelled,
            )
        });
        let present = [a?, b?, c?, d?];
        Ok(Self {
            keys,
            tag: &request.tag,
            holder,
            variants,
            logistics: holder
                .logistics
                .iter()
                .map(|&id| (id, classifier.classify_logistics(holder, id)))
                .collect(),
            history,
            aggregates,
            amounts,
            present,
            coverage: Reader::new(
                store,
                key,
                format!("stockpile-{branch}-coverage"),
                cancelled,
            )?,
            bounds: (metadata.first.unwrap_or(metadata.last), metadata.last),
            included: &request.included,
        })
    }
    fn actor(&self, field: &str) -> Option<u64> {
        self.keys.get(&series::field_key(self.tag, field)).copied()
    }
    pub(super) fn relevant(&self, field: usize, id: u64, groups: &[String], dense: bool) -> bool {
        if groups.is_empty() {
            return true;
        }
        if field < 5 {
            if groups.contains(&series::field_key("variant", &id.to_string())) {
                return true;
            }
            if dense {
                return false;
            }
            return self.variants.get(&id).is_some_and(|classification| {
                classification
                    .groups(Some(id))
                    .iter()
                    .any(|key| groups.contains(key))
                    || classification.kind.is_none()
                        && groups
                            .iter()
                            .any(|group| !group.starts_with("[\"variant\","))
            });
        }
        if dense {
            return false;
        }
        self.logistics
            .get(&(id as u32))
            .is_some_and(|classification| {
                classification
                    .groups(None)
                    .iter()
                    .any(|key| groups.contains(key))
                    || classification.kind.is_none()
                        && groups.iter().any(|group| {
                            group.starts_with("[\"domain\",")
                                && group != &series::field_key("domain", "unresolved")
                        })
                        && (self.included.is_empty()
                            || self
                                .included
                                .contains(&format!("{UNRESOLVED_MARK}{}", classification.token)))
            })
    }
    fn corrected(&mut self, group: &str, from: i32, to: i32) -> io::Result<[Vec<series::Run>; 4]> {
        if let Some(values) = self.correct_known(group, from, to)? {
            return Ok(values);
        }
        // An unknown observation-time total cannot be adjusted arithmetically.
        // Recover only its final members and the unresolved completeness operands.
        let variants: BTreeMap<_, _> = self
            .variants
            .iter()
            .filter(|(id, classification)| {
                classification.kind.is_none()
                    || classification
                        .groups(Some(**id))
                        .iter()
                        .any(|key| key == group)
            })
            .map(|(id, classification)| (*id, classification.clone()))
            .collect();
        let (kind, _): (String, String) = serde_json::from_str(group)?;
        let logistics: BTreeMap<_, _> = self
            .logistics
            .iter()
            .filter(|(_, classification)| {
                classification.groups(None).iter().any(|key| key == group)
                    || kind == "domain" && classification.kind.is_none()
            })
            .map(|(id, classification)| (*id, classification.clone()))
            .collect();
        let mut dates = BTreeSet::from([from]);
        let mut operands = BTreeMap::<(usize, u64), (Vec<series::Run>, Vec<series::Run>)>::new();
        for (field, slot) in [(0, 0), (5, 1), (6, 2), (7, 3)] {
            let ids: Vec<_> = if field == 0 {
                variants.keys().copied().collect()
            } else {
                logistics.keys().map(|id| u64::from(*id)).collect()
            };
            for id in ids {
                let (values, present) =
                    if let Some(actor) = self.actor(&format!("{}:{id}", FIELDS[field])) {
                        (
                            self.amounts[slot].runs(actor, from, to)?,
                            self.present[slot].runs(actor, from, to)?,
                        )
                    } else {
                        (Vec::new(), Vec::new())
                    };
                for run in values.iter().chain(&present) {
                    dates.insert(run.first_hour);
                    if let Some(end) = run.last_hour.checked_add(1).filter(|end| *end <= to) {
                        dates.insert(end);
                    }
                }
                operands.insert((field, id), (values, present));
            }
        }
        let mut coverage = [Vec::new(), Vec::new()];
        for (slot, field) in [(0, "stock-hours"), (1, "balance-hours")] {
            if let Some(actor) = self.actor(field) {
                coverage[slot] = self.coverage.runs(actor, from, to)?;
            }
            for run in &coverage[slot] {
                dates.insert(run.first_hour);
                if let Some(end) = run.last_hour.checked_add(1).filter(|end| *end <= to) {
                    dates.insert(end);
                }
            }
        }
        let dates: Vec<_> = dates.into_iter().collect();
        let mut result: [Vec<series::Run>; 4] = Default::default();
        for (index, &hour) in dates.iter().enumerate() {
            let last = dates.get(index + 1).map_or(to, |next| next - 1);
            let known = coverage.each_ref().map(|runs| {
                runs.iter()
                    .any(|run| run.first_hour <= hour && hour <= run.last_hour)
            });
            let values = evaluate(
                group,
                &variants,
                &logistics,
                known[0],
                known[1],
                None,
                |field, id| {
                    let (values, present) = &operands[&(field, id)];
                    Ok(Operand {
                        value: values
                            .iter()
                            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                            .map(|run| run.value)
                            .or_else(|| {
                                (!present.iter().any(|run| {
                                    run.first_hour <= hour && hour <= run.last_hour && run.value < 0
                                }) && known[usize::from(field != 0)])
                                .then_some(0)
                            }),
                        present: present.iter().any(|run| {
                            run.first_hour <= hour && hour <= run.last_hour && run.value != 0
                        }),
                    })
                },
            )?;
            for (slot, value) in values.into_iter().enumerate() {
                if let Some(value) = value {
                    let previous = result[slot].last();
                    let broken = previous
                        .is_none_or(|run| run.last_hour.checked_add(1) != Some(hour))
                        || coverage[usize::from(slot != 0)]
                            .iter()
                            .any(|run| run.first_hour == hour && run.break_before);
                    if let Some(previous) = result[slot]
                        .last_mut()
                        .filter(|run| !broken && run.value == value)
                    {
                        previous.last_hour = last;
                    } else {
                        result[slot].push(series::Run::new(hour, last, value, broken));
                    }
                }
            }
        }
        Ok(result)
    }
    /// Named membership changes adjust the recorded aggregate with signed
    /// operands. Refused totals/operands use the exact reconstruction above.
    fn correct_known(
        &mut self,
        group: &str,
        from: i32,
        to: i32,
    ) -> io::Result<Option<[Vec<series::Run>; 4]>> {
        let final_names = &self.variants;
        let moves: Vec<_> = self
            .history
            .iter()
            .flat_map(|(&id, history)| {
                history.iter().filter_map(move |span| {
                    let final_name = &final_names[&id];
                    (span.first <= to && span.last >= from && span.classification != *final_name)
                        .then_some((id, span.clone(), final_name.clone()))
                })
            })
            .collect();
        if moves.iter().any(|(_, span, final_name)| {
            span.classification.kind.is_none() || final_name.kind.is_none()
        }) {
            return Ok(None);
        }
        let Some(actor) = self.aggregate_key(group, 0) else {
            return Ok(None);
        };
        let base = self.aggregates[0].runs(actor, from, to)?;
        // A gap could be an unknown contributing operand which the new group
        // excludes. It must be reconstructed, never assigned an invented zero.
        if base.first().is_none_or(|run| run.first_hour != from)
            || base.last().is_none_or(|run| run.last_hour != to)
            || base
                .windows(2)
                .any(|pair| pair[0].last_hour.checked_add(1) != Some(pair[1].first_hour))
        {
            return Ok(None);
        }
        let mut dates = BTreeSet::from([from]);
        let mut operands = BTreeMap::new();
        for run in &base {
            dates.insert(run.first_hour);
        }
        for (id, span, final_name) in &moves {
            let old = span
                .classification
                .groups(Some(*id))
                .iter()
                .any(|key| key == group);
            let new = final_name.groups(Some(*id)).iter().any(|key| key == group);
            if old == new {
                continue;
            }
            dates.insert(span.first.max(from));
            if let Some(end) = span.last.checked_add(1).filter(|end| *end <= to) {
                dates.insert(end);
            }
            let values = if let Some(actor) = self.actor(&format!("stock:{id}")) {
                self.amounts[0].runs(actor, from, to)?
            } else {
                Vec::new()
            };
            for run in &values {
                dates.insert(run.first_hour);
                if let Some(end) = run.last_hour.checked_add(1).filter(|end| *end <= to) {
                    dates.insert(end);
                }
            }
            operands.insert(*id, values);
        }
        let dates: Vec<_> = dates.into_iter().collect();
        let mut result: [Vec<series::Run>; 4] = Default::default();
        for (index, &hour) in dates.iter().enumerate() {
            let supplied = base
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                .expect("covered group stock");
            let mut value = supplied.value;
            for (id, span, final_name) in moves
                .iter()
                .filter(|(_, span, _)| span.first <= hour && hour <= span.last)
            {
                let old = span
                    .classification
                    .groups(Some(*id))
                    .iter()
                    .any(|key| key == group);
                let new = final_name.groups(Some(*id)).iter().any(|key| key == group);
                if old == new {
                    continue;
                }
                let Some(amount) = operands[id]
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                    .map(|run| run.value)
                else {
                    return Ok(None);
                };
                value += if new { amount } else { -amount };
            }
            let last = dates.get(index + 1).map_or(to, |next| next - 1);
            let broken =
                result[0].last().is_none() || supplied.first_hour == hour && supplied.break_before;
            if let Some(previous) = result[0]
                .last_mut()
                .filter(|run| !broken && run.value == value)
            {
                previous.last_hour = last;
            } else {
                result[0].push(series::Run::new(hour, last, value, broken));
            }
        }
        // Variant classification cannot change the logistics record's grouping.
        for (slot, rows) in result.iter_mut().enumerate().skip(1) {
            if let Some(actor) = self.aggregate_key(group, slot) {
                *rows = self.aggregates[slot].runs(actor, from, to)?;
            }
        }
        Ok(Some(result))
    }
    fn aggregate_key(&self, group: &str, slot: usize) -> Option<u64> {
        self.actor(&group_key(group, MEASURES[slot]))
    }
    fn filtered(&self, group: &str) -> io::Result<bool> {
        let (kind, _): (String, String) = serde_json::from_str(group)?;
        Ok(kind == "domain" && !self.included.is_empty())
    }
    /// User subsets merge the selected types' change points, never their maxima.
    fn subset(&mut self, group: &str, within: &[HourSpan]) -> io::Result<[Vec<series::Run>; 4]> {
        let mut types = BTreeSet::new();
        let mut balanced = BTreeSet::new();
        for (&id, classification) in &self.variants {
            let groups = classification.groups(Some(id));
            let (_, key): (String, String) = serde_json::from_str(&groups[0])?;
            if groups[1] == group && self.included.contains(&key) {
                types.insert(groups[0].clone());
            }
        }
        for classification in self.logistics.values() {
            let groups = classification.groups(None);
            let (_, key): (String, String) = serde_json::from_str(&groups[0])?;
            if groups[1] == group && self.included.contains(&key) {
                types.insert(groups[0].clone());
                balanced.insert(groups[0].clone());
            }
        }
        let mut inputs = BTreeMap::new();
        let mut dates = BTreeSet::new();
        for span in within {
            dates.insert(span.first_hour);
            if let Some(end) = span.last_hour.checked_add(1) {
                dates.insert(end);
            }
        }
        for key in &types {
            let mut values: [Vec<series::Run>; 3] = Default::default();
            for (slot, rows) in values.iter_mut().enumerate() {
                if slot != 0 && !balanced.contains(key) {
                    continue;
                }
                *rows = self.runs(key, slot, within)?;
                for run in rows {
                    dates.insert(run.first_hour);
                    if let Some(end) = run.last_hour.checked_add(1) {
                        dates.insert(end);
                    }
                }
            }
            inputs.insert(key.clone(), values);
        }
        let (_, domain): (String, String) = serde_json::from_str(group)?;
        let unresolved: Vec<_> = self
            .logistics
            .iter()
            .filter_map(|(&id, classification)| {
                (classification.kind.is_none()
                    && self
                        .included
                        .contains(&format!("{UNRESOLVED_MARK}{}", classification.token)))
                .then_some(id)
            })
            .collect();
        let mut ambiguous = Vec::new();
        if domain != "unresolved" {
            for id in unresolved {
                for (field, slot) in [(5, 1), (6, 2)] {
                    if let Some(actor) = self.actor(&format!("{}:{id}", FIELDS[field])) {
                        for span in within {
                            let presence =
                                self.present[slot].runs(actor, span.first_hour, span.last_hour)?;
                            let values =
                                self.amounts[slot].runs(actor, span.first_hour, span.last_hour)?;
                            for run in presence.iter().chain(&values) {
                                dates.insert(run.first_hour);
                                if let Some(end) = run.last_hour.checked_add(1) {
                                    dates.insert(end);
                                }
                            }
                            ambiguous.push((presence, values));
                        }
                    }
                }
            }
        }
        let dates: Vec<_> = dates.into_iter().collect();
        let mut result: [Vec<series::Run>; 4] = Default::default();
        for pair in dates.windows(2) {
            let hour = pair[0];
            let last = pair[1] - 1;
            if !within
                .iter()
                .any(|span| span.first_hour <= hour && hour <= span.last_hour)
            {
                continue;
            }
            let sum = |slot: usize| {
                inputs
                    .iter()
                    .filter(|(key, _)| slot == 0 || balanced.contains(*key))
                    .try_fold(0_i128, |sum, (_, values)| {
                        values[slot]
                            .iter()
                            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                            .map(|run| sum + run.value)
                    })
            };
            let stock = (!types.is_empty()).then(|| sum(0)).flatten();
            let mut balance = (!balanced.is_empty()).then(|| sum(1)).flatten();
            let mut need = (!balanced.is_empty()).then(|| sum(2)).flatten();
            for (presence, values) in &ambiguous {
                if presence
                    .iter()
                    .any(|run| run.first_hour <= hour && hour <= run.last_hour && run.value != 0)
                {
                    need = None;
                    if values
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                        .is_none_or(|run| run.value != 0)
                    {
                        balance = None;
                    }
                }
            }
            if balance.is_none() {
                need = None;
            }
            for (slot, value) in [
                stock,
                balance,
                need,
                Some(i128::from(
                    balance.is_some() && need.is_some_and(|need| need > 0),
                )),
            ]
            .into_iter()
            .enumerate()
            {
                if let Some(value) = value {
                    let broken = within.iter().any(|span| span.first_hour == hour)
                        || result[slot]
                            .last()
                            .is_none_or(|run| run.last_hour.checked_add(1) != Some(hour))
                        || inputs.values().any(|values| {
                            values[slot.min(2)]
                                .iter()
                                .any(|run| run.first_hour == hour && run.break_before)
                        });
                    if let Some(previous) = result[slot]
                        .last_mut()
                        .filter(|run| !broken && run.value == value)
                    {
                        previous.last_hour = last;
                    } else {
                        result[slot].push(series::Run::new(hour, last, value, broken));
                    }
                }
            }
        }
        Ok(result)
    }
    fn zero_before_group(
        &mut self,
        group: &str,
        slot: usize,
        from: i32,
        to: i32,
    ) -> io::Result<Vec<series::Run>> {
        let last = if let Some(first) = self.holder.groups.get(group) {
            let Some(last) = first.checked_sub(1) else {
                return Ok(Vec::new());
            };
            to.min(last)
        } else {
            to
        };
        if from > last || slot > 2 {
            return Ok(Vec::new());
        }
        let own: Vec<_> = self
            .logistics
            .iter()
            .filter(|(_, classification)| {
                classification.groups(None).iter().any(|key| key == group)
            })
            .collect();
        if (slot == 0
            && own
                .iter()
                .any(|(_, classification)| classification.kind.is_none()))
            || (slot != 0 && own.is_empty())
            || (slot == 2
                && own.iter().any(|(_, classification)| {
                    classification
                        .kind
                        .as_ref()
                        .is_none_or(|kind| kind.requires_available)
                }))
        {
            return Ok(Vec::new());
        }
        let (grouping, key): (String, String) = serde_json::from_str(group)?;
        // Only resolved domains depend on other unresolved logistics types.
        let actor = if slot != 0 && (grouping != "domain" || key == "unresolved") {
            self.actor("balance-hours")
        } else {
            self.aggregate_key(&series::field_key("complete", ""), slot)
        };
        if let Some(actor) = actor {
            if slot != 0 && (grouping != "domain" || key == "unresolved") {
                return self.coverage.runs(actor, from, last);
            }
            return self.aggregates[slot].runs(actor, from, last);
        }
        Ok(Vec::new())
    }
    fn runs(
        &mut self,
        group: &str,
        slot: usize,
        within: &[HourSpan],
    ) -> io::Result<Vec<series::Run>> {
        if self.filtered(group)? {
            return Ok(self.subset(group, within)?[slot].clone());
        }
        let mut result = Vec::new();
        for (first, last, changed) in
            pieces(within, &affected(group, &self.history, &self.variants))
        {
            let mut runs = if changed {
                self.corrected(group, first, last)?[slot].clone()
            } else if let Some(actor) = self.aggregate_key(group, slot) {
                self.aggregates[slot].runs(actor, first, last)?
            } else {
                Vec::new()
            };
            if !changed && slot <= 2 {
                let mut earlier = self.zero_before_group(group, slot, first, last)?;
                earlier.append(&mut runs);
                runs = earlier;
            }
            result.extend(runs);
        }
        result.sort_by_key(|run| run.first_hour);
        Ok(result)
    }
    fn at(&mut self, group: &str, slot: usize, hour: i32) -> io::Result<Option<i128>> {
        if self.filtered(group)? {
            return Ok(self.subset(
                group,
                &[HourSpan {
                    first_hour: hour,
                    last_hour: hour,
                }],
            )?[slot]
                .first()
                .map(|run| run.value));
        }
        if affected(group, &self.history, &self.variants)
            .iter()
            .any(|span| span.first_hour <= hour && hour <= span.last_hour)
        {
            Ok(self.corrected(group, hour, hour)?[slot]
                .first()
                .map(|run| run.value))
        } else if let Some(actor) = self.aggregate_key(group, slot) {
            let value = self.aggregates[slot]
                .value_at(actor, hour)?
                .map(|run| run.value);
            if slot <= 2 && value.is_none() {
                return Ok(self
                    .zero_before_group(group, slot, hour, hour)?
                    .first()
                    .map(|run| run.value));
            }
            Ok(value)
        } else if slot <= 2 {
            Ok(self
                .zero_before_group(group, slot, hour, hour)?
                .first()
                .map(|run| run.value))
        } else {
            Ok(None)
        }
    }
    pub(super) fn summary(
        &mut self,
        group: &str,
        slot: usize,
        view: (f64, f64, u32),
        within: &[HourSpan],
    ) -> io::Result<StockpileOperandSummary> {
        use dashboard_base::prepared_chart::{ChartBucket, ChartRun, merge};
        let from = view.0.floor() as i32;
        let to = view.1.floor() as i32;
        let cover_key = self.actor(if slot == 0 {
            "stock-hours"
        } else {
            "balance-hours"
        });
        let template = if let Some(actor) = cover_key {
            self.coverage
                .summary_within(actor, (from, to), view, 0, Some(within))?
        } else {
            dashboard_base::prepared_chart::ChartSummary {
                approximate: Vec::new(),
                from: view.0,
                to: view.1,
                width: view.2,
                buckets: Vec::new(),
                runs: Some(Vec::new()),
            }
        };
        let changed = affected(group, &self.history, &self.variants);
        let mut values = template.clone();
        for (bucket, covered) in values.buckets.iter_mut().zip(&template.buckets) {
            *bucket = None;
            if let Some(covered) = covered {
                let clipped: Vec<_> = within
                    .iter()
                    .filter_map(|span| {
                        let first_hour = span.first_hour.max(covered.first_hour);
                        let last_hour = span.last_hour.min(covered.last_hour);
                        (first_hour <= last_hour).then_some(HourSpan {
                            first_hour,
                            last_hour,
                        })
                    })
                    .collect();
                for (first, last, corrected) in pieces(&clipped, &changed) {
                    if corrected || self.filtered(group)? {
                        let runs = if self.filtered(group)? {
                            self.subset(
                                group,
                                &[HourSpan {
                                    first_hour: first,
                                    last_hour: last,
                                }],
                            )?
                        } else {
                            self.corrected(group, first, last)?
                        };
                        for run in &runs[slot] {
                            let value = run.value as f64;
                            *bucket = merge(
                                *bucket,
                                Some(ChartBucket {
                                    absolute_total: run.value.unsigned_abs().saturating_mul(
                                        (i64::from(run.last_hour) - i64::from(run.first_hour) + 1)
                                            as u128,
                                    ),
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                    first_value: value,
                                    last_value: value,
                                    last_value_hour: run.first_hour,
                                    min_hour: run.first_hour,
                                    min_value: value,
                                    max_hour: run.first_hour,
                                    max_value: value,
                                    first_break: run.break_before,
                                    has_break: false,
                                }),
                            );
                        }
                    } else {
                        let mut prior = if slot <= 2 {
                            self.zero_before_group(group, slot, first, last)?
                        } else {
                            Vec::new()
                        };
                        for run in prior.drain(..) {
                            *bucket = merge(
                                *bucket,
                                Some(ChartBucket {
                                    absolute_total: 0,
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                    first_value: 0.0,
                                    last_value: 0.0,
                                    last_value_hour: run.first_hour,
                                    min_hour: run.first_hour,
                                    min_value: 0.0,
                                    max_hour: run.first_hour,
                                    max_value: 0.0,
                                    first_break: run.break_before,
                                    has_break: false,
                                }),
                            );
                        }
                        if let Some(actor) = self.aggregate_key(group, slot) {
                            let summary = self.aggregates[slot].summary(
                                actor,
                                self.bounds,
                                (f64::from(first), f64::from(last), 1),
                                0,
                            )?;
                            for next in summary.buckets.into_iter().flatten() {
                                *bucket = merge(*bucket, Some(next));
                            }
                        }
                    }
                }
            }
        }
        values.runs = if template.runs.is_some() {
            Some(
                self.runs(
                    group,
                    slot,
                    &within
                        .iter()
                        .filter_map(|span| {
                            let first_hour = span.first_hour.max(from);
                            let last_hour = span.last_hour.min(to);
                            (first_hour <= last_hour).then_some(HourSpan {
                                first_hour,
                                last_hour,
                            })
                        })
                        .collect::<Vec<_>>(),
                )?
                .into_iter()
                .filter(|run| run.last_hour >= from && run.first_hour <= to)
                .map(|run| ChartRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    value: run.value as f64,
                    break_before: run.break_before,
                })
                .collect(),
            )
        } else {
            None
        };
        let mut samples = Vec::new();
        let mut row_samples = Vec::new();
        for bucket in &values.buckets {
            let mut sample: [Option<String>; 4] = Default::default();
            let mut rows: [Option<i8>; 4] = Default::default();
            if let Some(bucket) = bucket {
                for (index, hour) in [
                    bucket.first_hour,
                    bucket.last_hour,
                    bucket.min_hour,
                    bucket.max_hour,
                ]
                .into_iter()
                .enumerate()
                {
                    sample[index] = self.at(group, slot, hour)?.map(|value| value.to_string());
                    rows[index] = sample[index].as_ref().map(|_| 1);
                }
            }
            samples.push(sample);
            row_samples.push(rows);
        }
        Ok(StockpileOperandSummary {
            field: MEASURES[slot].into(),
            id: group.into(),
            key: group.into(),
            presence: values.clone(),
            values,
            samples,
            row_samples,
        })
    }
    pub(super) fn peak(
        &mut self,
        group: &str,
        within: &[HourSpan],
    ) -> io::Result<StockpileChoicePeak> {
        let mut peak = 0_i128;
        let mut stock_drawable = false;
        let mut share_drawable = false;
        if self.filtered(group)? {
            let values = self.subset(group, within)?;
            for run in values[0].iter().chain(&values[2]) {
                peak = peak.max(run.value);
            }
            return Ok(StockpileChoicePeak {
                raw: peak.to_string(),
                stock_drawable: !values[0].is_empty(),
                share_drawable: values[3].iter().any(|run| run.value == 1),
            });
        }
        for (first, last, changed) in
            pieces(within, &affected(group, &self.history, &self.variants))
        {
            if changed {
                let values = self.corrected(group, first, last)?;
                for run in values[0].iter().chain(&values[2]) {
                    peak = peak.max(run.value);
                }
                stock_drawable |= !values[0].is_empty();
                share_drawable |= values[3].iter().any(|run| run.value == 1);
            } else {
                stock_drawable |= !self.zero_before_group(group, 0, first, last)?.is_empty();
                for slot in [0, 2, 3] {
                    if let Some(actor) = self.aggregate_key(group, slot) {
                        let summary = self.aggregates[slot].summary(
                            actor,
                            self.bounds,
                            (f64::from(first), f64::from(last), 1),
                            0,
                        )?;
                        for bucket in summary.buckets.into_iter().flatten() {
                            let value = exact_max(&mut self.aggregates[slot], actor, &bucket)?;
                            if slot == 0 {
                                stock_drawable = true;
                            }
                            if slot == 3 {
                                share_drawable |= value == 1;
                            } else {
                                peak = peak.max(value);
                            }
                        }
                    }
                }
            }
        }
        Ok(StockpileChoicePeak {
            raw: peak.to_string(),
            stock_drawable,
            share_drawable,
        })
    }
    pub(super) fn peaks(
        &mut self,
        within: &[HourSpan],
    ) -> io::Result<BTreeMap<String, StockpileChoicePeak>> {
        let groups: BTreeSet<_> = self
            .variants
            .iter()
            .flat_map(|(&id, classification)| classification.groups(Some(id)))
            .chain(
                self.logistics
                    .values()
                    .flat_map(|classification| classification.groups(None)),
            )
            .collect();
        let mut result = BTreeMap::new();
        for group in groups {
            result.insert(group.clone(), self.peak(&group, within)?);
        }
        let ids: Vec<_> = self.holder.variants.iter().copied().collect();
        for id in ids {
            let mut peak = StockpileChoicePeak {
                raw: "0".into(),
                ..Default::default()
            };
            let mut best = 0;
            if let Some(actor) = self.actor(&format!("stock:{id}")) {
                let first = self.present[0]
                    .next_change(actor, f64::from(i32::MIN) - 1.0, true)?
                    .unwrap_or(i32::MAX);
                for span in within {
                    if span.first_hour < first
                        && let Some(covered) = self.actor("stock-hours")
                    {
                        let last = span.last_hour.min(first.saturating_sub(1));
                        peak.stock_drawable |= self
                            .coverage
                            .summary(
                                covered,
                                self.bounds,
                                (f64::from(span.first_hour), f64::from(last), 1),
                                0,
                            )?
                            .buckets
                            .iter()
                            .any(Option::is_some);
                    }
                    let summary = self.amounts[0].summary(
                        actor,
                        self.bounds,
                        (f64::from(span.first_hour), f64::from(span.last_hour), 1),
                        0,
                    )?;
                    for bucket in summary.buckets.into_iter().flatten() {
                        best = best.max(exact_max(&mut self.amounts[0], actor, &bucket)?);
                        peak.stock_drawable = true;
                    }
                }
            }
            peak.raw = best.to_string();
            result.insert(series::field_key("variant", &id.to_string()), peak);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
