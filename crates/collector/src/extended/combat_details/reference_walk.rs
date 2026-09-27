//! Tests only: the land combat walk as main's readers read, sharing nothing
//! between readers (no census members, commander selections, pass shares or
//! division images) nor between divisions (the division sources are cleared
//! before each). These are copies of `Destination::visit` and
//! `Destination::detail` with every share dropped; the family tests compare
//! the walk with them, table by table and failure by failure.
use super::*;

impl Destination<'_> {
    /// Before each combat reader: forget every share.
    fn forget_shares(&mut self) {
        self.commanders = cland_combat::Commanders::default();
        self.combat.clear();
    }

    pub(super) fn visit_reference(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        source: cland_combat::Source,
        members: cland_combat::Members<'_>,
    ) {
        self.current_window = None;
        // Selections are the combat's own.
        self.commanders = cland_combat::Commanders::default();
        // The census authenticated every participant moments ago in this pass.
        let _ = members;
        let members = cland_combat::Members::none();
        self.forget_shares();
        {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let started = qpc();
            // The census has already retained every participant in this combat.
            // Its original counts bound both destinations without another read.
            let bound =
                (source.attackers.count as usize).saturating_add(source.defenders.count as usize);
            self.window_progress.required += bound;
            let start = self.window_progress.len;
            grow_vec(self.window, start + bound);
            grow_vec(self.divisions, self.progress[1].required + bound);
            match cland_combat::read_window_operands(
                memory,
                base,
                state,
                source,
                &members,
                &mut self.commanders,
                self.combat,
                &mut self.window[start..],
            ) {
                Ok(operands) => {
                    let len = operands.divisions_len;
                    self.window_progress.len += len;
                    self.current_window = Some((source.id, start, len, operands.hq_unit));
                }
                Err(reason) => {
                    // Fail the consuming tables closed: a missing HQ unit
                    // would invent a hidden badge, missing operands icons.
                    if self.progress[0].failure.is_none() {
                        self.progress[0].failure = Some(reason);
                    }
                    if self.progress[1].failure.is_none() {
                        self.progress[1].failure = Some(reason);
                    }
                }
            }
            self.ticks[1] += elapsed(started);
        }
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
                                flags: context.side_flags[side],
                                indicator: context.indicator,
                                border_values: context.border_values[side],
                                start: self.arenas[1].len,
                                len: 0,
                            };
                            *index = Some(next);
                            self.progress[2].len += 1;
                        }
                    }
                }
                Err(reason) => self.progress[2].failure = Some(reason),
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
                        // The window walk in visit() covers every full
                        // participant, so every visited handle joins. A miss
                        // fails the table closed rather than zero-filling
                        // operands the viewer would draw as absent icons.
                        let operand = match self.current_window {
                            Some((combat, start, len, _)) if combat == source.id => self.window
                                [start..start + len]
                                .get(row)
                                .filter(|row| row.handle == handle)
                                .copied(),
                            _ => None,
                        };
                        if operand.is_none() && self.progress[1].failure.is_none() {
                            self.progress[1].failure = Some("land window operands missing");
                        }
                        if self.progress[1].failure.is_none()
                            && let Some(operand) = operand
                        {
                            // Enemy transport terms go straight into the arena
                            // past its admitted length; the reader grows it on
                            // this worker, one successfully read term at a time.
                            let transport_start = self.reinforcement_progress[3].len;
                            self.sources.clear();
                            let result = read_division(
                                divisional,
                                base,
                                state,
                                self.sources,
                                self.scratch,
                                self.transport_terms,
                                transport_start,
                                pointer,
                                handle,
                            );
                            match result {
                                Ok((stats, context, reinforcement, manpower_len)) => {
                                    let commander_input =
                                        self.commander(memory, base, stats.commander);
                                    self.manpower_progress.required += manpower_len;
                                    let manpower_start = self.manpower_progress.len;
                                    self.width_progress.required += context.width_term_count;
                                    let start = self.width_progress.len;
                                    let count = context.width_term_count;
                                    let counts = [
                                        reinforcement.modifier_count,
                                        reinforcement.supply_term_count,
                                        reinforcement.acclimatization_count,
                                        reinforcement.transport_term_count,
                                    ];
                                    let starts = self.reinforcement_progress.map(|p| p.len);
                                    for (p, count) in
                                        self.reinforcement_progress.iter_mut().zip(counts)
                                    {
                                        p.required += count;
                                    }
                                    if stats.commander == 0 || commander_input.is_some() {
                                        // Every prefix was read successfully above. Grow from
                                        // those exact counts and retain it now, without retrying
                                        // native reads or dropping the containing division.
                                        grow_vec(self.width_terms, start + count);
                                        grow_vec(self.manpower, manpower_start + manpower_len);
                                        grow_vec(
                                            self.reinforcement_modifiers,
                                            starts[0] + counts[0],
                                        );
                                        grow_vec(self.supply_terms, starts[1] + counts[1]);
                                        grow_vec(self.acclimatization, starts[2] + counts[2]);
                                        let scratch = &*self.scratch;
                                        self.reinforcement_modifiers
                                            [starts[0]..starts[0] + counts[0]]
                                            .copy_from_slice(
                                                &scratch.reinforcement_modifiers[..counts[0]],
                                            );
                                        self.supply_terms[starts[1]..starts[1] + counts[1]]
                                            .copy_from_slice(&scratch.supply[..counts[1]]);
                                        self.acclimatization[starts[2]..starts[2] + counts[2]]
                                            .copy_from_slice(&scratch.acclimatization[..counts[2]]);
                                        // Transport terms were written in place at
                                        // starts[3]; admission only advances the length.
                                        for (p, count) in
                                            self.reinforcement_progress.iter_mut().zip(counts)
                                        {
                                            p.len += count;
                                        }
                                        self.width_terms[start..start + count]
                                            .copy_from_slice(&scratch.widths[..count]);
                                        self.width_progress.len += count;
                                        self.manpower
                                            [manpower_start..manpower_start + manpower_len]
                                            .copy_from_slice(&scratch.manpower[..manpower_len]);
                                        self.manpower_progress.len += manpower_len;
                                        self.divisions[self.progress[1].len] = Division {
                                            combat: source.id,
                                            side,
                                            ordinal,
                                            province,
                                            role,
                                            stats,
                                            direction: operand.direction,
                                            direction_group: operand.direction_group,
                                            template_frames: operand.template_frames,
                                            commander_input,
                                            context,
                                            width_start: start,
                                            manpower_start,
                                            manpower_len,
                                            reinforcement,
                                            reinforcement_starts: starts,
                                        };
                                        self.progress[1].len += 1;
                                    }
                                }
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
        self.forget_shares();
        match cland_combat::read_details(
            memory,
            base,
            source,
            members,
            &mut self.commanders,
            &mut self.scratch.rosters,
        ) {
            Ok((mut row, count)) => {
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
                        return;
                    }
                    Outcome::Truncated { .. } => return,
                    Outcome::Rows { .. } => {}
                }
                if self.progress[0].len == self.details.len()
                    || count > self.rosters.len().saturating_sub(self.arenas[0].len)
                {
                    return;
                }
                // The window walk ran before this detail in visit(); without
                // its HQ row the badge would be invented hidden. Fail closed.
                let Some((window_combat, window_start, window_len, hq_unit)) = self.current_window
                else {
                    self.progress[0].failure = Some("land window operands missing");
                    return;
                };
                if window_combat != source.id {
                    self.progress[0].failure = Some("land window operands mismatch");
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
                    hq_unit,
                    window_start,
                    window_len,
                };
                self.tactic_unlock_progress.len += unlock_count;
                self.progress[0].len += 1;
                self.arenas[0].len += count;
            }
            Err(reason) => self.progress[0].failure = Some(reason),
        }
    }
}
