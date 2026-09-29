//! Tests only: the land combat walk as unshared readers read, sharing nothing
//! between readers (no census members, commander selections or division
//! images) nor between divisions (the division sources and the country
//! reader are rebuilt before each; the naming memo, which decides the name
//! arena's layout, is kept). These are copies of
//! `Destination::visit` and `Destination::detail` with every share dropped;
//! the family tests compare the walk with them, table by table and failure by
//! failure.
use super::*;

impl Destination<'_> {
    /// Before each combat reader: forget every share.
    fn forget_shares(&mut self) {
        self.commanders = cland_combat::Commanders::default();
    }

    pub(super) fn visit_reference(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: cland_combat::Members<'_>,
    ) {
        // Selections are the combat's own.
        self.commanders = cland_combat::Commanders::default();
        // The census authenticated every participant moments ago in this pass.
        let _ = members;
        let members = cland_combat::Members::none();
        self.forget_shares();
        // The census has already retained every participant in this combat;
        // its counts bound the division destination without another read.
        let bound =
            (source.attackers.count as usize).saturating_add(source.defenders.count as usize);
        grow_vec(self.divisions, self.progress[1].required + bound);
        {
            let started = qpc();
            self.detail_reference(memory, base, state, source, &members);
            self.ticks[0] += elapsed(started);
        }
        let mut side_indices = [None; 2];
        {
            let _reads = crate::read_accounting::table(TABLES[2]);
            let started = qpc();
            self.progress[2].required += 2;
            match cland_combat::read_modifiers_context(memory, base, source) {
                Ok(context) => {
                    for (side, index) in side_indices.iter_mut().enumerate() {
                        if self.progress[2].len < self.sides.len() {
                            let next = self.progress[2].len;
                            self.sides[next] = ModifierSide {
                                combat: source.id,
                                side: side as u8,
                                border_values: context.border_values[side],
                                start: self.arenas[1].len,
                                len: 0,
                            };
                            *index = Some(next);
                            self.progress[2].len += 1;
                        }
                    }
                }
                Err(reason) => {
                    self.progress[2].failure = Some(reason);
                }
            }
            self.ticks[2] += elapsed(started);
        }
        // Every division's army and stat block fields, two gathers for the
        // combat, for its four readers and its modifier member. Lent while
        // the visitor borrows `self`.
        let mut images = std::mem::take(self.images);
        images.clear();
        self.forget_shares();
        // The unit visit's storage, lent while the visitor borrows `self`.
        let mut shares = std::mem::take(self.combat);
        {
            let _reads = crate::read_accounting::table(TABLES[3]);
            let started = qpc();
            self.combat_sides(memory, base, source);
            self.ticks[3] += elapsed(started);
        }
        {
            let started = qpc();
            let before = self.ticks[1] + self.ticks[2];
            let result = cland_combat::visit_units(
                memory,
                base,
                source,
                &members,
                &mut shares,
                |side, ordinal, pointer, handle, province, role| {
                    let row = ordinal as usize
                        + if side == 0 {
                            0
                        } else {
                            source.attackers.count as usize
                        };
                    let divisional = &images.division(memory, row, pointer);
                    {
                        let _reads = crate::read_accounting::table(TABLES[1]);
                        let started = qpc();
                        self.progress[1].required += 1;
                        if self.progress[1].failure.is_none() {
                            // Enemy transport terms go straight into the arena
                            // past its admitted length; the reader grows it on
                            // this worker, one successfully read term at a time.
                            let transport_start = self.reinforcement_progress[3].len;
                            // Exile tags go straight into the preallocated arena
                            // past its admitted length; an overflow is counted
                            // for the spool's growth, never allocated here.
                            let exile_start = self.exile_progress.len;
                            // Nothing carries over from another division: its
                            // sources or the country reader. The naming memo is
                            // storage, not a read share: it keeps each shared
                            // group key once in the arena, as the walk does.
                            self.sources.clear();
                            let countries = engine::ccountry::Reader::new(memory, base, state);
                            let result = read_division(
                                divisional,
                                base,
                                state,
                                &countries,
                                self.sources,
                                self.scratch,
                                DivisionOutputs {
                                    name_groups: self.name_groups,
                                    names: &mut self.names,
                                    exile_tags: &mut self.exile_tags[exile_start..],
                                    transport_terms: self.transport_terms,
                                    transport_start,
                                },
                                role,
                                pointer,
                                handle,
                            );
                            match result {
                                Ok((stats, exile, context, reinforcement)) => self.admit(
                                    memory,
                                    base,
                                    Placement {
                                        combat: source.id,
                                        side,
                                        ordinal,
                                        province,
                                        role,
                                        exile_start,
                                    },
                                    Acquired {
                                        stats,
                                        exile,
                                        context,
                                        reinforcement,
                                    },
                                ),
                                Err(reason) => self.progress[1].failure = Some(reason),
                            }
                        }
                        self.ticks[1] += elapsed(started);
                    }
                    {
                        let _reads = crate::read_accounting::table(TABLES[2]);
                        let started = qpc();
                        self.modifier_member(
                            divisional,
                            base,
                            pointer,
                            handle,
                            ordinal,
                            side_indices[side as usize],
                        );
                        self.ticks[2] += elapsed(started);
                    }
                },
            );
            self.ticks[0] +=
                elapsed(started).saturating_sub(self.ticks[1] + self.ticks[2] - before);
            if let Err(reason) = result {
                for index in [1, 2] {
                    {
                        self.progress[index].failure = Some(reason);
                    }
                }
            }
        }
        *self.combat = shares;
        *self.images = images;
    }
    /// The country sets with the marker's indicator and flag words, which
    /// `read_country_sets` reads itself.
    fn detail_reference(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: &cland_combat::Members<'_>,
    ) {
        self.progress[0].required += 1;
        if self.progress[0].failure.is_some() {
            return;
        }
        if self.tactic_registry.is_none() {
            self.tactic_registry = Some(land_tactic_inputs::capture_registry(
                memory,
                base,
                self.tactic_candidates,
                self.tactic_weights,
            ));
        }
        match self.tactic_registry.expect("registry supplied").outcome {
            Outcome::Unavailable { reason } => {
                self.progress[0].failure = Some(reason);
                return;
            }
            Outcome::Truncated { .. } => return,
            Outcome::Rows { .. } => {}
        }
        // Selected commanders' texts share the commander text arena; they stay
        // committed only if this combat's row is kept.
        let texts = self.commander_names.len;
        self.forget_shares();
        match cland_combat::read_details(
            memory,
            base,
            source,
            members,
            &mut self.commanders,
            &mut self.scratch.rosters,
            &mut self.commander_names,
        ) {
            Ok(cland_combat::Details {
                mut row,
                participants: count,
                texts_fit,
            }) => {
                self.arenas[0].required += count;
                self.forget_shares();
                let mut context = land_tactic_inputs::capture_context(
                    memory,
                    base,
                    state,
                    source,
                    members,
                    &mut self.commanders,
                    &mut self.tactic_unlocks[self.tactic_unlock_progress.len..],
                );
                self.tactic_unlock_progress.required += context.row.required_unlocks;
                match context.outcome {
                    Outcome::Unavailable { reason } => {
                        self.progress[0].failure = Some(reason);
                        self.commander_names.len = texts;
                        return;
                    }
                    Outcome::Truncated { .. } => {
                        self.commander_names.len = texts;
                        return;
                    }
                    Outcome::Rows { .. } => {}
                }
                // A text that did not fit was counted for growth: the whole
                // combat is omitted this hour, as for a full roster arena.
                if !texts_fit
                    || self.progress[0].len == self.details.len()
                    || count > self.rosters.len().saturating_sub(self.arenas[0].len)
                {
                    self.commander_names.len = texts;
                    return;
                }
                let start = self.arenas[0].len;
                self.rosters[start..start + count].copy_from_slice(&self.scratch.rosters[..count]);
                for side in &mut row.sides {
                    side.committed_start += start;
                    side.reserve_start += start;
                }
                let unlock_count = context.row.unlock_count.iter().sum::<usize>();
                for start in &mut context.row.unlock_start {
                    *start += self.tactic_unlock_progress.len;
                }
                self.details[self.progress[0].len] = Detail {
                    row,
                    tactic_context: context.row,
                };
                self.tactic_unlock_progress.len += unlock_count;
                self.progress[0].len += 1;
                self.arenas[0].len += count;
            }
            Err(reason) => self.progress[0].failure = Some(reason),
        }
    }
}
