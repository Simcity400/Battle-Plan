use super::*;

impl Destination<'_> {
    fn forget_shares(&mut self) {
        self.commanders = engine_native_land::land_combat_details::Commanders::default();
    }

    pub(super) fn visit_reference(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: cland_combat::Members<'_>,
    ) {
        self.commanders = engine_native_land::land_combat_details::Commanders::default();
        let _ = members;
        let members = cland_combat::Members::none();
        self.forget_shares();
        let bound =
            (source.attackers.count as usize).saturating_add(source.defenders.count as usize);
        grow_vec(self.divisions, self.progress[1].required + bound);
        grow_vec(self.sides, self.progress[2].required.saturating_add(2));
        grow_vec(
            self.unit_scratch,
            (source.attackers.count.max(source.defenders.count)) as usize,
        );
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
        let mut images = std::mem::take(self.images);
        images.clear();
        self.forget_shares();
        {
            let _reads = crate::read_accounting::table(TABLES[3]);
            let started = qpc();
            self.combat_sides(memory, base, source);
            self.ticks[3] += elapsed(started);
        }
        {
            let started = qpc();
            let before = self.ticks[1] + self.ticks[2];
            let mut owned = std::mem::take(self.unit_scratch);
            let result = cland_combat::visit_units(
                memory,
                base,
                source,
                &members,
                &mut owned,
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
                            let exile_start = self.exile_progress.len;
                            self.sources.clear();
                            let countries =
                                engine_native::ccountry::Reader::new(memory, base, state);
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
                                    exile_tags: engine_native_memory::walk::Buffer::GrowingFrom(
                                        self.exile_tags,
                                        exile_start,
                                    ),
                                },
                                role,
                                pointer,
                                handle,
                            );
                            match result {
                                Ok((stats, exile, context, reserve_speed)) => self.admit(
                                    Placement {
                                        pointer,
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
                                        reserve_speed,
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
            *self.unit_scratch = owned;
            self.ticks[0] +=
                elapsed(started).saturating_sub(self.ticks[1] + self.ticks[2] - before);
            let result = result.and_then(|visited| match visited {
                cland_combat::Visited::Complete => Ok(()),
                cland_combat::Visited::Scratch { .. } => {
                    Err("land role subsets exceed their full participants")
                }
            });
            if let Err(reason) = result {
                for index in [1, 2] {
                    {
                        self.progress[index].failure = Some(reason);
                    }
                }
            }
        }
        *self.images = images;
    }
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
        self.forget_shares();
        let at = self.detail_used;
        let roster = self.arenas[0].len;
        let mut arenas = engine_native_land::land_combat_details::DetailArenas::with_buffers(
            engine_native_memory::walk::Buffer::GrowingFrom(self.rosters, roster),
            engine_native_memory::walk::Buffer::GrowingFrom(self.air_counts, at.air),
            engine_native_memory::walk::Buffer::GrowingFrom(self.detail_text, at.text),
            engine_native_memory::walk::Buffer::GrowingFrom(self.commander_traits, at.traits),
            engine_native_memory::walk::Buffer::GrowingFrom(self.timed_traits, at.timed),
            engine_native_memory::walk::Buffer::GrowingFrom(self.learning_traits, at.learning),
        );
        let result = engine_native_land::land_combat_details::read_details(
            memory,
            base,
            source,
            members,
            &mut self.commanders,
            &mut arenas,
        );
        let (complete, used, required) = (arenas.complete(), arenas.used, arenas.required);
        self.arenas[0].required += required.participants;
        for (total, value) in [
            (&mut self.detail_required.air, required.air),
            (&mut self.detail_required.text, required.text),
            (&mut self.detail_required.traits, required.traits),
            (&mut self.detail_required.timed, required.timed),
            (&mut self.detail_required.learning, required.learning),
        ] {
            *total += value;
        }
        match result {
            Ok(mut row) => {
                self.forget_shares();
                let context = land_tactic_inputs::capture_context(
                    memory,
                    base,
                    state,
                    source,
                    members,
                    &mut self.commanders,
                );
                if let Outcome::Unavailable { reason } = context.outcome {
                    self.progress[0].failure = Some(reason);
                    return;
                }
                if !grow_vec(self.details, self.progress[0].len + 1) || !complete {
                    return;
                }
                let text = at.text;
                for side in &mut row.sides {
                    side.committed_start += roster;
                    side.reserve_start += roster;
                    side.traits_start += at.traits;
                    side.timed_traits_start += at.timed;
                    side.air_counts.start += at.air;
                    side.learning_traits.start += at.learning;
                    for token in [
                        &mut side.tactic_token,
                        &mut side.commander_name_token,
                        &mut side.commander_description_token,
                    ] {
                        token.start += text;
                    }
                }
                row.stored_terrain_name.start += text;
                if let Some(border) = &mut row.border_war {
                    for side in &mut border.sides {
                        side.win_event.start += text;
                        side.lose_event.start += text;
                    }
                }
                for stored in &mut self.commander_traits[at.traits..at.traits + used.traits] {
                    stored.token_start += text;
                }
                for stored in &mut self.learning_traits[at.learning..at.learning + used.learning] {
                    stored.token_start += text;
                }
                self.arenas[0].len += used.participants;
                self.detail_used.air += used.air;
                self.detail_used.text += used.text;
                self.detail_used.traits += used.traits;
                self.detail_used.timed += used.timed;
                self.detail_used.learning += used.learning;
                self.details[self.progress[0].len] = Detail {
                    row,
                    tactic_context: context.row,
                };
                self.progress[0].len += 1;
            }
            Err(reason) => self.progress[0].failure = Some(reason),
        }
    }
}
