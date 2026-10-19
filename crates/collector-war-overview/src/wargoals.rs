//! Flat snapshots assembled from writer-certified owner inventories. The
//! existing overview visit lends authenticated countries and diplomacy roots.
use collector_observers::wargoal_changes as changes;
use engine_native::{Memory, ccountry, cwar_goal::inventory::Inputs, walk::Outcome};
use replay_data::rows::{self, tables};
use replay_frame::frame::TableView;
use std::sync::Mutex;

#[derive(Default)]
struct Owner {
    country_id: u32,
    country: usize,
    diplomacy: usize,
    certificate: Option<changes::Certificate>,
    inputs: Inputs,
}
#[derive(Default)]
struct Store {
    owners: Vec<Option<Owner>>,
    verify: Inputs,
    input_bytes: usize,
}
static STORE: Mutex<Store> = Mutex::new(Store {
    owners: Vec::new(),
    input_bytes: 0,
    verify: Inputs {
        goals: Vec::new(),
        justifications: Vec::new(),
        targets: Vec::new(),
    },
});

#[derive(Default)]
pub struct Captured {
    goals: Vec<rows::CountryWargoalRow>,
    targets: Vec<rows::WargoalTargetRow>,
    justifications: Vec<rows::WargoalJustificationRow>,
    reason: Option<&'static str>,
}
impl Captured {
    pub fn fail(&mut self, reason: &'static str) {
        self.reason = Some(reason);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.goals.capacity() * size_of::<rows::CountryWargoalRow>()
            + self.targets.capacity() * size_of::<rows::WargoalTargetRow>()
            + self.justifications.capacity() * size_of::<rows::WargoalJustificationRow>()
    }
    /// Scope the store to the complete existing census. No live pointer is
    /// followed after this acquisition; only the flat owned rows leave it.
    pub fn visit_owners<R>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        force_full: bool,
        walk: impl FnOnce(&mut dyn FnMut(ccountry::Country, usize)) -> R,
    ) -> R {
        self.visit_certified(
            memory,
            base,
            force_full,
            changes::certificate,
            changes::verify,
            walk,
        )
    }
    fn visit_certified<R>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        force_full: bool,
        certificate_for: impl Fn(u32) -> Option<changes::Certificate>,
        verify_for: impl Fn(u32) -> bool,
        walk: impl FnOnce(&mut dyn FnMut(ccountry::Country, usize)) -> R,
    ) -> R {
        self.goals.clear();
        self.targets.clear();
        self.justifications.clear();
        self.reason = None;
        let mut store = STORE.try_lock().ok();
        if store.is_none() {
            self.fail("war-goal owner store unavailable");
        }
        let result = walk(&mut |country, diplomacy| {
            if self.reason.is_some() {
                return;
            }
            let result = (|| {
                let store = store.as_mut().ok_or("war-goal owner store unavailable")?;
                let index = country.canonical_slot as usize;
                if index >= store.owners.len() {
                    engine_allocation::resize_default(&mut store.owners, index + 1)
                        .map_err(|_| "war-goal owner storage allocation failed")?;
                }
                let certificate = certificate_for(country.id);
                let Store {
                    owners,
                    verify,
                    input_bytes,
                } = &mut **store;
                let owner = owners[index].get_or_insert_with(Owner::default);
                let same_owner = owner.country_id == country.id
                    && owner.country == country.pointer
                    && owner.diplomacy == diplomacy;
                let parts = std::array::from_fn::<_, 2, _>(|part| {
                    !force_full
                        && same_owner
                        && certificate.is_some()
                        && owner.certificate.is_some()
                        && certificate.unwrap()[part] == owner.certificate.unwrap()[part]
                });
                let carry = parts.iter().all(|part| *part);
                let check = parts.iter().any(|part| *part) && verify_for(country.id);
                if !carry || check {
                    // Lost observation must not select an hourly full scan.
                    if certificate.is_none() && owner.country != 0 && !force_full {
                        return Err("war-goal writer certificate unavailable");
                    }
                    owner.certificate = None;
                    let before = owner.inputs.allocated_bytes() + verify.allocated_bytes();
                    let read = (|| {
                        // Keep owner capacities in place. Only the shared scratch
                        // grows for verification, and it reads carried domains only.
                        if check {
                            verify.goals.clear();
                            verify.justifications.clear();
                            verify.targets.clear();
                            verify.read_parts(memory, base, country, diplomacy, parts)?;
                        }
                        if !carry {
                            owner.inputs.read_parts(
                                memory,
                                base,
                                country,
                                diplomacy,
                                parts.map(|carried| !carried),
                            )?;
                        }
                        Ok::<_, &'static str>(())
                    })();
                    *input_bytes = input_bytes
                        .saturating_sub(before)
                        .saturating_add(owner.inputs.allocated_bytes() + verify.allocated_bytes());
                    read?;
                    if certificate_for(country.id) != certificate {
                        return Err("war-goal writers overlapped inventory acquisition");
                    }
                    if check {
                        let mismatch = !owner.inputs.same_parts(verify, parts);
                        changes::verified(mismatch);
                        if mismatch {
                            return Err("war-goal writer certificate mismatch");
                        }
                    }
                    owner.country = country.pointer;
                    owner.country_id = country.id;
                    owner.diplomacy = diplomacy;
                    owner.certificate = certificate;
                }
                changes::record(carry && !check);
                engine_allocation::reserve(&mut self.goals, owner.inputs.goals.len())
                    .map_err(|_| "war-goal row allocation failed")?;
                engine_allocation::reserve(&mut self.targets, owner.inputs.targets.len())
                    .map_err(|_| "war-goal target row allocation failed")?;
                engine_allocation::reserve(
                    &mut self.justifications,
                    owner.inputs.justifications.len(),
                )
                .map_err(|_| "war-goal justification row allocation failed")?;
                self.goals
                    .extend(owner.inputs.goals.iter().map(|g| rows::CountryWargoalRow {
                        country: country.id,
                        ordinal: g.ordinal,
                        handle: g.handle,
                        actor: g.actor,
                        recipient: g.recipient,
                        goal_type: g.goal_type,
                        expires: g.expires,
                        name_token: g.name_token,
                    }));
                self.targets
                    .extend(owner.inputs.targets.iter().map(|t| rows::WargoalTargetRow {
                        country: country.id,
                        kind: t.kind,
                        goal: t.goal,
                        ordinal: t.ordinal,
                        value: t.value,
                    }));
                self.justifications
                    .extend(owner.inputs.justifications.iter().map(|j| {
                        rows::WargoalJustificationRow {
                            country: country.id,
                            ordinal: j.ordinal,
                            actor: j.actor,
                            recipient: j.recipient,
                            goal_type: j.goal_type,
                            total_cost_raw: j.total_cost,
                            daily_cost_raw: j.daily_cost,
                            progress_raw: j.progress,
                            start: j.start,
                            end: j.end,
                            pay_upfront: j.pay_upfront,
                            threat_raw: j.threat,
                        }
                    }));
                Ok(())
            })();
            if let Err(reason) = result {
                self.fail(reason);
            }
        });
        if let Some(store) = store {
            changes::note_countries(store.owners.len());
            changes::note_inventory_bytes(
                store.input_bytes + store.owners.capacity() * size_of::<Option<Owner>>(),
            );
        }
        result
    }
    pub fn outcomes(&self) -> [Outcome; 3] {
        if let Some(reason) = self.reason {
            [Outcome::Unavailable { reason }; 3]
        } else {
            [
                Outcome::Rows {
                    len: self.goals.len(),
                },
                Outcome::Rows {
                    len: self.targets.len(),
                },
                Outcome::Rows {
                    len: self.justifications.len(),
                },
            ]
        }
    }
    pub fn tables(&self) -> [TableView; 3] {
        let outcomes = self.outcomes();
        [
            crate::table_conversion::table::<tables::CountryWargoals, _>(
                outcomes[0],
                &self.goals,
                |r| *r,
            ),
            crate::table_conversion::table::<tables::WargoalTargets, _>(
                outcomes[1],
                &self.targets,
                |r| *r,
            ),
            crate::table_conversion::table::<tables::WargoalJustifications, _>(
                outcomes[2],
                &self.justifications,
                |r| *r,
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{ccountry::synthetic::World, cpolitical_status};
    static TESTS: Mutex<()> = Mutex::new(());
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    #[test]
    fn wargoals_carry_reads_no_native_payload_and_changed_or_missing_certificate_cannot_be_empty() {
        let _serial = TESTS.lock().unwrap();
        let mut w = World::new(0, 1, 0x1600);
        let diplomacy = w.alloc(0x300);
        let politics = w.alloc(0x120);
        let owner = w.object(1);
        w.u64(owner + ccountry::POLITICAL_STATUS, politics as u64);
        w.u64(politics, cpolitical_status::VTABLE_RVA as u64);
        w.u64(politics + cpolitical_status::COUNTRY, owner as u64);
        let memory = w.memory();
        let reader = ccountry::Reader::new(&memory, 0, w.game_state()).unwrap();
        let country = reader.at(1).unwrap().unwrap();
        *STORE.lock().unwrap() = Store::default();
        let mut c = Captured::default();
        c.visit_certified(
            &memory,
            0,
            true,
            |_| Some([(1, 1), (1, 1)]),
            |_| false,
            |visit| visit(country, diplomacy),
        );
        assert!(
            c.outcomes()
                .iter()
                .all(|r| matches!(r, Outcome::Rows { len: 0 }))
        );
        c.visit_certified(
            &Unreadable,
            0,
            false,
            |_| Some([(1, 1), (1, 1)]),
            |_| false,
            |visit| visit(country, diplomacy),
        );
        assert!(
            c.outcomes()
                .iter()
                .all(|r| matches!(r, Outcome::Rows { len: 0 })),
            "a quiet owner must read no payload"
        );
        c.visit_certified(
            &Unreadable,
            0,
            false,
            |_| None,
            |_| false,
            |visit| visit(country, diplomacy),
        );
        assert!(c.outcomes().iter().all(|r| matches!(
            r,
            Outcome::Unavailable {
                reason: "war-goal writer certificate unavailable"
            }
        )));
        c.visit_certified(
            &Unreadable,
            0,
            false,
            |_| Some([(1, 2), (1, 2)]),
            |_| false,
            |visit| visit(country, diplomacy),
        );
        assert!(
            c.outcomes()
                .iter()
                .all(|r| matches!(r, Outcome::Unavailable { .. })),
            "a changed owner must read, and cannot turn failure into empty"
        );
        let reads = std::cell::Cell::new(0);
        c.visit_certified(
            &memory,
            0,
            true,
            |_| {
                let n = reads.get() + 1;
                reads.set(n);
                Some([(1, n), (1, n)])
            },
            |_| false,
            |visit| visit(country, diplomacy),
        );
        assert!(c.outcomes().iter().all(|r| matches!(
            r,
            Outcome::Unavailable {
                reason: "war-goal writers overlapped inventory acquisition"
            }
        )));
    }

    #[test]
    fn wargoals_verification_keeps_owner_storage_in_place() {
        let _serial = TESTS.lock().unwrap();
        let mut w = World::new(0, 1, 0x1600);
        let diplomacy = w.alloc(0x300);
        let politics = w.alloc(0x120);
        let owner = w.object(1);
        w.u64(owner + ccountry::POLITICAL_STATUS, politics as u64);
        w.u64(politics, cpolitical_status::VTABLE_RVA as u64);
        w.u64(politics + cpolitical_status::COUNTRY, owner as u64);
        let unused = w.alloc(8);
        w.array(
            diplomacy + engine_native::cdiplomacy_status::OWNED_WARGOALS,
            unused,
            1,
        );
        let memory = w.memory();
        let reader = ccountry::Reader::new(&memory, 0, w.game_state()).unwrap();
        let country = reader.at(1).unwrap().unwrap();
        *STORE.lock().unwrap() = Store::default();
        let mut captured = Captured::default();
        captured.visit_certified(
            &memory,
            0,
            true,
            |_| Some([(1, 1), (1, 1)]),
            |_| false,
            |visit| visit(country, diplomacy),
        );
        let original = STORE.lock().unwrap().owners[country.canonical_slot as usize]
            .as_ref()
            .unwrap()
            .inputs
            .targets
            .as_ptr() as usize;
        assert_eq!(captured.outcomes()[1], Outcome::Rows { len: 1 });
        for certificate in [[(1, 1), (1, 1)], [(1, 1), (1, 2)]] {
            captured.visit_certified(
                &memory,
                0,
                false,
                |_| Some(certificate),
                |_| true,
                |visit| visit(country, diplomacy),
            );
            assert_eq!(captured.outcomes()[1], Outcome::Rows { len: 1 });
            let store = STORE.lock().unwrap();
            let owner = store.owners[country.canonical_slot as usize]
                .as_ref()
                .unwrap();
            assert_eq!(
                owner.inputs.targets.as_ptr() as usize,
                original,
                "verification must not replace an owner's reusable target storage"
            );
        }
    }
}
