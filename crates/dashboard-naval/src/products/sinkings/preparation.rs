//! Pending attribution operands and only the occurrences whose inputs changed.
use super::*;

#[derive(Serialize, Deserialize)]
pub(super) enum Pending {
    Ship(OpenShip, [bool; 2]),
    Convoy(OpenConvoy, [bool; 2]),
}
impl Pending {
    fn references(&self) -> [u32; 2] {
        match self {
            Self::Ship(ship, missing) => [
                if missing[0] { ship.victim } else { 0 },
                if missing[1] { ship.attacker } else { 0 },
            ],
            Self::Convoy(convoy, missing) => [
                if missing[0] { convoy.owner } else { 0 },
                if missing[1] { convoy.killer } else { 0 },
            ],
        }
    }
}

pub struct Change {
    pub key: (bool, u64),
    pub ship: Option<ShipEvent>,
    pub convoy: Option<ConvoyEvent>,
    pub possible: [BTreeSet<Identity>; 2],
    pub pending: bool,
}
impl Change {
    pub fn uncertainties(&self, lost: bool) -> BTreeMap<Identity, Vec<(i32, i32)>> {
        let (first, read) = if let Some(ship) = &self.ship {
            (ship.entry.sink_hour, ship.read_hour)
        } else {
            let convoy = self.convoy.as_ref().expect("one changed occurrence");
            (convoy.month, convoy.read_hour)
        };
        self.possible[usize::from(!lost)]
            .iter()
            .map(|&identity| (identity, vec![(first, read)]))
            .collect()
    }
}

impl ShipHistory {
    /// The legacy observe/finish methods keep their complete-scan contract.
    /// Preparation uses the same tracker and attribution functions, retaining
    /// only the operands still waiting for a country bridge.
    pub fn prepare(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> Vec<Change> {
        let initializing = self.prepared_references.is_none();
        if initializing {
            let mut references = BTreeMap::<u32, BTreeSet<(bool, u64)>>::new();
            for (&key, pending) in &self.prepared_pending {
                for raw in pending.references().into_iter().filter(|raw| *raw != 0) {
                    references.entry(raw).or_default().insert(key);
                }
            }
            self.prepared_references = Some(references);
        }
        self.scan.ships.clear();
        self.scan.convoys.clear();
        self.ship_indices.clear();
        #[cfg(test)]
        {
            self.preparing = true;
        }
        self.observe_with_changes(format, frame, Some(changes));
        #[cfg(test)]
        {
            self.preparing = false;
        }
        let mut dirty = BTreeSet::new();
        if initializing {
            dirty.extend(self.prepared_pending.keys().copied());
        } else {
            for raw in &self.scan.changed_references {
                dirty.extend(
                    self.prepared_references
                        .as_ref()
                        .expect("prepared references")
                        .get(raw)
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
        }
        let mut changes = Vec::new();
        for key in dirty {
            let pending = self
                .prepared_pending
                .remove(&key)
                .expect("pending dirty occurrence");
            self.remove_references(key, &pending);
            changes.push(match pending {
                Pending::Ship(ship, _) => Change {
                    key,
                    ship: Some(ship.event()),
                    convoy: None,
                    possible: Default::default(),
                    pending: false,
                },
                Pending::Convoy(convoy, _) => Change {
                    key,
                    ship: None,
                    convoy: Some(convoy.event()),
                    possible: Default::default(),
                    pending: false,
                },
            });
        }
        for (&index, ship) in self.ship_indices.iter().zip(&self.scan.ships) {
            changes.push(Change {
                key: (false, u64::from(index)),
                ship: Some(ship.clone()),
                convoy: None,
                possible: Default::default(),
                pending: false,
            });
        }
        for convoy in &self.scan.convoys {
            let key = (true, self.next_convoy_index);
            self.next_convoy_index = self
                .next_convoy_index
                .checked_add(1)
                .expect("convoy occurrence index overflow");
            changes.push(Change {
                key,
                ship: None,
                convoy: Some(convoy.clone()),
                possible: Default::default(),
                pending: false,
            });
        }
        // Materialize the supplying-read proof only for references of changed
        // occurrences; other holders retain CountryCache's deferred extension.
        let raws: BTreeSet<_> = changes
            .iter()
            .flat_map(|change| {
                if let Some(ship) = &change.ship {
                    [ship.entry.victim_country, ship.entry.attacker_country]
                } else {
                    let convoy = change.convoy.as_ref().expect("changed convoy");
                    [convoy.owner_raw, convoy.killer_raw]
                }
            })
            .collect();
        self.extend_references(&raws);
        for change in &mut changes {
            let resolved =
                self.resolve_change(change.key, change.ship.take(), change.convoy.take());
            *change = resolved;
            if change.pending {
                let pending = if let Some(ship) = &change.ship {
                    Pending::Ship(
                        OpenShip {
                            victim: ship.entry.victim_country,
                            attacker: ship.entry.attacker_country,
                            sink_hour: ship.entry.sink_hour,
                            read_hour: ship.read_hour,
                            primary: ship.entry.cause_record == 0 && ship.entry.assisted_kill == 0,
                        },
                        [ship.victim.is_none(), ship.attacker.is_none()],
                    )
                } else {
                    let convoy = change.convoy.as_ref().expect("changed convoy");
                    Pending::Convoy(
                        OpenConvoy {
                            month: convoy.month,
                            read_hour: convoy.read_hour,
                            count: convoy.count,
                            baseline: convoy.baseline,
                            previous_read: convoy.previous_read,
                            owner: convoy.owner_raw,
                            killer: convoy.killer_raw,
                        },
                        [convoy.owner.is_none(), convoy.killer.is_none()],
                    )
                };
                for raw in pending.references().into_iter().filter(|raw| *raw != 0) {
                    self.prepared_references
                        .as_mut()
                        .expect("prepared references")
                        .entry(raw)
                        .or_default()
                        .insert(change.key);
                    self.scan.unresolved.insert(raw);
                }
                self.prepared_pending.insert(change.key, pending);
            }
        }
        self.scan.ships.clear();
        self.scan.convoys.clear();
        self.ship_indices.clear();
        for change in &changes {
            if let Some(ship) = &change.ship {
                self.ship_indices
                    .push(u32::try_from(change.key.1).expect("native ship row index"));
                self.scan.ships.push(ship.clone());
            }
        }
        // Sinking log/title callers require the original global scan order.
        let mut ships: Vec<_> = std::mem::take(&mut self.ship_indices)
            .into_iter()
            .zip(std::mem::take(&mut self.scan.ships))
            .collect();
        ships.sort_by_key(|(index, _)| *index);
        for (index, ship) in ships {
            self.ship_indices.push(index);
            self.scan.ships.push(ship);
        }
        self.scan.warnings.clear();
        self.update_warnings();
        changes
    }
    fn remove_references(&mut self, key: (bool, u64), pending: &Pending) {
        for raw in pending.references() {
            let references = self
                .prepared_references
                .as_mut()
                .expect("prepared references");
            if let Some(keys) = references.get_mut(&raw) {
                keys.remove(&key);
                if keys.is_empty() {
                    references.remove(&raw);
                    self.scan.unresolved.remove(&raw);
                }
            }
        }
    }
    fn extend_references(&mut self, raws: &BTreeSet<u32>) {
        let Some(hour) = self.countries.extend_to else {
            return;
        };
        for raw in raws {
            let Some(tag) = self.countries.tags.get(raw) else {
                continue;
            };
            if let Some(run) = self
                .raw_history
                .get_mut(raw)
                .and_then(|runs| runs.last_mut())
            {
                run.last = hour;
            }
            if let Some(span) = self
                .scan
                .country_identities
                .get_mut(tag)
                .and_then(|spans| spans.last_mut())
            {
                span.last_hour = hour;
                self.identities
                    .get_mut(tag)
                    .expect("country identity")
                    .step(Some(span.identity), hour);
            }
            if let Some(span) = self
                .scan
                .segments
                .get_mut(tag)
                .and_then(|spans| spans.last_mut())
            {
                span.last_hour = hour;
            }
        }
    }
    pub fn prepared_scope_changed(&self) -> bool {
        self.countries.changed
    }
    pub fn prepared_holders(&self) -> &[(String, Identity)] {
        if self.countries.hour == Some(self.scan.last_hour) {
            &self.countries.holders
        } else {
            &[]
        }
    }
    pub fn finish_prepared(&mut self, last_hour: i32) {
        self.countries
            .extend(&mut self.scan, &mut self.raw_history, &mut self.identities);
        if let Some(next) = self.next
            && next <= i64::from(last_hour)
        {
            self.scan.missing(&mut self.track, next as i32, last_hour);
            self.next = Some(i64::from(last_hour) + 1);
        }
        self.scan.last_hour = last_hour;
        self.scan.ships.clear();
        self.scan.convoys.clear();
        self.ship_indices.clear();
        self.scan.ship_reads.clear();
        self.scan.convoy_reads.clear();
    }
    fn resolve_change(
        &self,
        key: (bool, u64),
        mut ship: Option<ShipEvent>,
        mut convoy: Option<ConvoyEvent>,
    ) -> Change {
        #[cfg(test)]
        work::WORK.with(|work| work.borrow_mut().events += 1);
        let possible;
        let pending;
        if let Some(ship) = &mut ship {
            possible = resolve_ship(ship, &self.raw_history, &self.scan.country_candidates);
            pending = ship.entry.victim_country != 0 && ship.victim.is_none()
                || ship.entry.attacker_country != 0 && ship.attacker.is_none();
        } else {
            let convoy = convoy.as_mut().expect("changed occurrence");
            possible = resolve_convoy(convoy, &self.raw_history, &self.scan.country_candidates);
            pending = convoy.owner_raw != 0 && convoy.owner.is_none()
                || convoy.killer_raw != 0 && convoy.killer.is_none();
        }
        Change {
            key,
            ship,
            convoy,
            possible,
            pending,
        }
    }
}
