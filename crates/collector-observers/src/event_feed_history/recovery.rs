use super::*;
use engine_native_memory::country_identity::Country;
use replay_data::event_feed_rows::decisions::{DecisionChange, Operation};

pub(crate) struct PreparedOwner {
    source: Store,
    country: Country,
    identity: Option<native::CountryIdentity>,
    work: [bool; 2],
    cuts: [Option<Cut>; 2],
    labels: [Vec<u32>; 2],
}

impl PreparedOwner {
    pub(crate) fn read(
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: Country,
        work: [bool; 2],
    ) -> Result<Self, &'static str> {
        allocation::advisory(|| {
            let mut source = Store::new(None)?;
            source.preparing_owner = true;
            source.generation = Some(generation);
            source.observing = true;
            source.registry_state = Some(state);
            source.prepare_decisions(1).map_err(|_| OWNER_STORAGE)?;
            source.prepare_delayed(1).map_err(|_| OWNER_STORAGE)?;
            source.seed_prepared_decisions(generation);
            source.seed_prepared_delayed(generation);
            let mut cuts = [None; 2];
            if work[OwnerRegistry::Decisions as usize] {
                cuts[OwnerRegistry::Decisions as usize] = Cut::take();
                source.recover_decisions(memory, base, generation, state, hour, country);
            }
            if work[OwnerRegistry::Delayed as usize] {
                cuts[OwnerRegistry::Delayed as usize] = Cut::take();
                source.recover_delayed(
                    memory,
                    base,
                    generation,
                    state,
                    hour,
                    Country {
                        canonical_slot: 0,
                        ..country
                    },
                );
            }
            if source
                .failure
                .is_some_and(|reason| OWNER_SHORTAGES.contains(&reason))
                || source
                    .owner_refusals
                    .iter()
                    .flatten()
                    .any(|refusal| refusal.reason == OWNER_STORAGE)
            {
                return Err(OWNER_STORAGE);
            }
            let identity = source
                .entries
                .first()
                .map(|entry| {
                    let owner = if Self::registry(entry) == OwnerRegistry::Decisions as usize {
                        entry.event.actor
                    } else {
                        entry.event.target
                    };
                    native::CountryIdentity {
                        reference: owner.reference,
                        id: owner.id,
                        instance: owner.instance,
                    }
                })
                .or_else(|| {
                    country
                        .pointer
                        .checked_add(engine_native::ccountry::SURRENDER_TARGET)
                        .and_then(|at| engine_native_memory::read::<u32>(memory, at))
                        .map(|instance| native::CountryIdentity {
                            reference: country.id,
                            id: country.id,
                            instance,
                        })
                });
            let labels = [
                allocation::filled(source.prepared_delayed_labels(), 0)
                    .map_err(|_| OWNER_STORAGE)?,
                allocation::filled(source.prepared_decision_labels(), 0)
                    .map_err(|_| OWNER_STORAGE)?,
            ];
            Ok(Self {
                source,
                country,
                identity,
                work,
                cuts,
                labels,
            })
        })
    }
    fn registry(entry: &Entry) -> usize {
        if matches!(
            entry.event.script.and_then(|script| script.change),
            Some(EventFeedScriptChange::Decision(_))
        ) {
            OwnerRegistry::Decisions as usize
        } else {
            OwnerRegistry::Delayed as usize
        }
    }
    fn completed(&self, registry: usize) -> bool {
        if registry == OwnerRegistry::Decisions as usize {
            self.source.decisions.certified(self.country.pointer)
        } else {
            self.source
                .delayed
                .countries
                .first()
                .is_some_and(|owner| owner.ready)
        }
    }
    fn control_tokens(&self) -> usize {
        self.work
            .iter()
            .enumerate()
            .filter(|(_, work)| **work)
            .map(|(registry, _)| {
                OWNER_STORAGE.len().max(OWNER_CROSSED.len()).max(
                    self.source.owner_refusals[registry].map_or(0, |refusal| refusal.reason.len()),
                )
            })
            .sum()
    }
}

impl Store {
    pub(crate) fn owner_work(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        state: usize,
        hour: i32,
        country: Country,
    ) -> [bool; 2] {
        if self.failure.is_some() || self.observation(generation).is_err() {
            return [false; 2];
        }
        self.decisions.census(country);
        let decisions = self.decisions.recovery(country.pointer);
        if decisions == decisions::Recovery::Certified {
            self.recover_decisions(memory, base, generation, state, hour, country);
        }
        let delayed = !self
            .delayed
            .countries
            .get(country.canonical_slot as usize)
            .is_some_and(|seen| {
                seen.pointer == country.pointer && seen.id == country.id && seen.ready
            });
        [delayed, decisions == decisions::Recovery::Baseline]
    }
    fn owner_control_demand(&self, country: Country) -> usize {
        collector_primitives::country_slots::census_rows(0)
            .max((country.canonical_slot as usize).saturating_add(1))
            .saturating_mul(2)
            .max(self.owner_controls)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "A local refusal keeps the selected registry, census identity and publication operands."
    )]
    fn refuse_prepared_owner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        country: Country,
        identity: native::CountryIdentity,
        registry: OwnerRegistry,
        refusal: OwnerRefusal,
    ) -> Result<(), &'static str> {
        let remaining = self.owner_controls.saturating_sub(1);
        let entries = self
            .entries
            .len()
            .saturating_add(1)
            .saturating_add(remaining);
        let tokens = self
            .tokens_used
            .saturating_add(refusal.reason.len())
            .saturating_add(remaining.saturating_mul(OWNER_CONTROL_TOKEN));
        self.entry_demand = self.entry_demand.max(entries);
        self.token_demand = self.token_demand.max(tokens);
        let fits = entries <= self.capacity() && tokens <= self.script_tokens.len();
        let repeated = if registry == OwnerRegistry::Decisions {
            self.prepared_decision_failure(country.pointer, identity)
        } else {
            self.prepared_delayed_failure(country)
        };
        if !fits && !repeated {
            let _ = self.advance();
            self.count_rejected(1);
            recovery_needed();
            return Ok(());
        }
        if registry == OwnerRegistry::Decisions {
            self.refuse_decision_owner(
                memory,
                base,
                generation,
                hour,
                country.pointer,
                identity,
                refusal.reason,
                true,
            )?;
        } else {
            self.refuse_delayed_owner(
                memory,
                base,
                generation,
                hour,
                identity,
                Some(country.canonical_slot as usize),
                refusal.reason,
                true,
            )?;
        }
        if refusal.retry {
            recovery_needed();
        }
        Ok(())
    }
    pub(crate) fn prepare_owner_demand(&mut self, plan: &PreparedOwner) -> [usize; 3] {
        self.prepare_owner_budget(plan, self.owner_control_demand(plan.country))
    }
    fn prepare_owner_budget(&mut self, plan: &PreparedOwner, controls: usize) -> [usize; 3] {
        let mut entries = 0_usize;
        let mut tokens = 0_usize;
        let mut scopes = 0_usize;
        for entry in &plan.source.entries {
            if plan.completed(PreparedOwner::registry(entry)) {
                entries = entries.saturating_add(1);
                let script = entry.event.script.unwrap();
                tokens = tokens.saturating_add(script.token_len as usize);
                scopes = scopes.saturating_add(script.scope_len as usize);
            }
        }
        let selected = plan.work.iter().filter(|work| **work).count();
        let control_tokens = controls
            .saturating_sub(selected)
            .saturating_mul(OWNER_CONTROL_TOKEN)
            .saturating_add(plan.control_tokens());
        let demand = [
            self.entries
                .len()
                .saturating_add(entries)
                .saturating_add(controls),
            self.tokens_used
                .saturating_add(tokens)
                .saturating_add(control_tokens),
            self.scopes_used.saturating_add(scopes),
        ];
        self.entry_demand = self.entry_demand.max(demand[0]);
        self.token_demand = self.token_demand.max(demand[1]);
        self.scope_demand = self.scope_demand.max(demand[2]);
        self.graph_demand = self.graph_demand.max(plan.source.graph_demand);
        if plan.work[OwnerRegistry::Decisions as usize] {
            self.decisions.demand = self.decisions.demand.max(self.prepared_decision_demand(
                &plan.source,
                plan.completed(OwnerRegistry::Decisions as usize),
            ));
        }
        if plan.work[OwnerRegistry::Delayed as usize] {
            self.delayed.demand = self.delayed.demand.max(self.prepared_delayed_demand(
                &plan.source,
                plan.country,
                plan.completed(OwnerRegistry::Delayed as usize),
            ));
        }
        demand
    }
    pub(crate) fn publish_owner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        plan: &mut PreparedOwner,
    ) -> Result<(), &'static str> {
        if self.failure.is_some() || self.observation(generation).is_err() {
            return Ok(());
        }
        let controls = self.owner_control_demand(plan.country);
        let demand = self.prepare_owner_budget(plan, controls);
        if demand[0] > self.capacity() {
            let _ = self.prepare_capacity(demand[0] - self.entries.len());
        }
        if demand[1] > self.script_tokens.len() || demand[2] > self.script_scopes.len() {
            let _ = self.prepare_scripts(
                demand[1].max(self.script_tokens.len()),
                demand[2].max(self.script_scopes.len()),
                self.scope_nodes.len(),
            );
        }
        let delayed = self.prepared_delayed_demand(
            &plan.source,
            plan.country,
            plan.completed(OwnerRegistry::Delayed as usize),
        );
        if delayed > self.delayed.capacity() {
            let _ = self.prepare_delayed(delayed);
        }
        let decisions = self.prepared_decision_demand(
            &plan.source,
            plan.completed(OwnerRegistry::Decisions as usize),
        );
        if decisions > self.decisions.capacity() {
            let _ = self.prepare_decisions(decisions);
        }
        let space = demand[0] <= self.capacity()
            && demand[1] <= self.script_tokens.len()
            && demand[2] <= self.script_scopes.len();
        if space {
            self.owner_controls = controls;
        }
        let fits = [
            space && delayed <= self.delayed.capacity(),
            space && decisions <= self.decisions.capacity(),
        ];
        let mut publish = [false; 2];
        for registry in 0..2 {
            if !plan.work[registry] {
                continue;
            }
            let crossed = plan.cuts[registry].is_none_or(|cut| !Self::sample_holds(cut));
            let refusal = if crossed {
                if plan.source.owner_refusals[registry]
                    .is_none_or(|refusal| refusal.reason != OWNER_CROSSED)
                {
                    crossed_sample(registry);
                }
                Some(OwnerRefusal {
                    reason: OWNER_CROSSED,
                    retry: true,
                    counted: false,
                })
            } else if !fits[registry] {
                Some(OwnerRefusal {
                    reason: OWNER_STORAGE,
                    retry: true,
                    counted: false,
                })
            } else {
                plan.source.owner_refusals[registry]
            };
            if let Some(refusal) = refusal {
                let Some(identity) = plan.identity else {
                    if refusal.retry {
                        recovery_needed();
                    } else {
                        self.reject(refusal.reason);
                    }
                    continue;
                };
                let registry = if registry == OwnerRegistry::Decisions as usize {
                    OwnerRegistry::Decisions
                } else {
                    OwnerRegistry::Delayed
                };
                self.refuse_prepared_owner(
                    memory,
                    base,
                    generation,
                    hour,
                    plan.country,
                    identity,
                    registry,
                    refusal,
                )?;
            } else if plan.completed(registry) {
                if registry == OwnerRegistry::Decisions as usize {
                    self.merge_decision_owner(
                        &plan.source,
                        plan.country.pointer,
                        &mut plan.labels[registry],
                    )?;
                } else {
                    self.merge_delayed_owner(
                        &plan.source,
                        plan.country,
                        &mut plan.labels[registry],
                    )?;
                }
                publish[registry] = true;
            }
        }
        for entry in &plan.source.entries {
            let registry = PreparedOwner::registry(entry);
            if !publish[registry] {
                continue;
            }
            let mut event = entry.event;
            let script = event.script.as_mut().unwrap();
            if let Some(id) = script.instance {
                script.instance = Some(
                    if registry == OwnerRegistry::Decisions as usize
                        && matches!(
                            script.change,
                            Some(EventFeedScriptChange::Decision(DecisionChange {
                                container: 0,
                                operation: Operation::Store,
                                ..
                            }))
                        )
                    {
                        self.fresh_decision_label()?
                    } else {
                        plan.labels[registry][id as usize]
                    },
                );
            }
            let tokens = script.token_start as usize;
            let scopes = script.scope_start as usize;
            let token_end = self.tokens_used + script.token_len as usize;
            let scope_end = self.scopes_used + script.scope_len as usize;
            self.script_tokens[self.tokens_used..token_end].copy_from_slice(
                &plan.source.script_tokens[tokens..tokens + script.token_len as usize],
            );
            self.script_scopes[self.scopes_used..scope_end].copy_from_slice(
                &plan.source.script_scopes[scopes..scopes + script.scope_len as usize],
            );
            script.token_start = u32::try_from(self.tokens_used)
                .map_err(|_| "event feed script arena ID overflow")?;
            script.scope_start = u32::try_from(self.scopes_used)
                .map_err(|_| "event feed script arena ID overflow")?;
            self.acquire(
                generation,
                |sequence| EventFeedEvent { sequence, ..event },
                None,
            )?;
            self.tokens_used = token_end;
            self.scopes_used = scope_end;
        }
        Ok(())
    }
    pub(crate) fn refuse_owner_preparation(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        country: Country,
        work: [bool; 2],
    ) {
        if self.failure.is_some() || self.observation(generation).is_err() {
            return;
        }
        let cut = Cut::take();
        let identity = native::provenance::World::new(memory, base)
            .and_then(|world| world.country_object(country.pointer));
        let reason = if cut.is_none_or(|cut| !Self::sample_holds(cut)) {
            OWNER_CROSSED
        } else {
            OWNER_STORAGE
        };
        if let Ok(identity) = identity {
            for registry in [OwnerRegistry::Decisions, OwnerRegistry::Delayed] {
                if work[registry as usize] {
                    let _ = self.refuse_prepared_owner(
                        memory,
                        base,
                        generation,
                        hour,
                        country,
                        identity,
                        registry,
                        OwnerRefusal {
                            reason,
                            retry: true,
                            counted: false,
                        },
                    );
                }
            }
        } else if reason != OWNER_CROSSED {
            self.reject(reason);
        }
        recovery_needed();
    }
}
