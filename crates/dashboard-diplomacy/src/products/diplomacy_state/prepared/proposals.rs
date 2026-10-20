use super::*;
use replay_data::rows::IncomingDiplomaticActionRow;

impl Prepared {
    pub(super) fn proposals(
        &mut self,
        frame: &DecodedFrame,
        format: FormatVersion,
        joins: &Joins,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        let hour = frame.hour;
        if !evidence::available(frame, TableId::IncomingDiplomaticActions) {
            self.suspend_queue();
            self.number("queue:coverage".into(), hour, None, false);
            return Ok(());
        }
        let fresh = slot_read(
            format,
            &frame.metrics,
            TableId::IncomingDiplomaticActions,
            hour,
        );
        if !fresh {
            self.number(
                "queue:coverage".into(),
                hour,
                self.queue_read.map(i128::from),
                false,
            );
            return Ok(());
        }

        let rows = frame.complete::<tables::IncomingDiplomaticActions>()?;
        let mut found = BTreeSet::new();
        let Some(handles) = self.queue_handles(hour, rows) else {
            return Ok(());
        };
        for row in rows {
            check()?;
            let key = self.proposal_key(hour, row, joins)?;
            if !found.insert(key) {
                return Err(io::Error::other(
                    "Duplicate incoming diplomatic action handle in one owner queue",
                ));
            }
            self.update_proposal(key, hour, row.last_command_hour);
        }
        self.end_missing_proposals(hour, &found, &handles);
        self.queue_read = Some(hour);
        self.number("queue:coverage".into(), hour, Some(i128::from(hour)), false);
        Ok(())
    }
    fn queue_handles(
        &mut self,
        hour: i32,
        rows: &[IncomingDiplomaticActionRow],
    ) -> Option<BTreeSet<(u32, u64)>> {
        let mut handles = BTreeSet::new();
        let duplicates = rows
            .iter()
            .filter(|row| !handles.insert((row.country, row.handle)))
            .count();
        self.number(
            "queue:duplicates".into(),
            hour,
            (duplicates > 0).then_some(duplicates as i128),
            false,
        );
        if duplicates > 0 {
            self.suspend_queue();
            self.number("queue:coverage".into(), hour, None, false);
            return None;
        }
        Some(handles)
    }
    fn proposal_key(
        &mut self,
        hour: i32,
        row: &IncomingDiplomaticActionRow,
        joins: &Joins,
    ) -> io::Result<u64> {
        let raw_countries = vec![
            row.country,
            row.actor,
            row.recipient,
            row.original_actor,
            row.original_recipient,
        ];
        let roles = vec![
            joins.raw.get(&row.country).cloned(),
            joins.raw.get(&row.actor).cloned(),
            joins.raw.get(&row.recipient).cloned(),
            joins.raw.get(&row.original_actor).cloned(),
            joins.raw.get(&row.original_recipient).cloned(),
        ];
        // Ordinal, wrapper status and action_type establish no episode identity.
        let signature = serde_json::to_string(&(row.handle, &raw_countries, &roles, row.hour))?;
        Ok(self.begin_proposal(hour, row, signature, roles, raw_countries))
    }
    fn begin_proposal(
        &mut self,
        hour: i32,
        row: &IncomingDiplomaticActionRow,
        signature: String,
        roles: Vec<Option<DiplomacyCountry>>,
        raw_countries: Vec<u32>,
    ) -> u64 {
        if let Some(&key) = self.queue_keys.get(&signature) {
            key
        } else {
            let id = format!("proposal:{}:{}", hour, self.keys.len());
            let key = self.key(id.clone());
            self.catalogue.entries.push(Entry {
                key,
                queue: true,
                roles: roles.clone(),
                raw_roles: raw_countries.clone(),
                unresolved_kind: None,
            });
            self.queue.insert(
                key,
                QueueLive {
                    signature: signature.clone(),
                    reply: DiplomacyProposal {
                        id,
                        handle: row.handle.to_string(),
                        owner: tag(&roles[0]),
                        sender: tag(&roles[1]),
                        recipient: tag(&roles[2]),
                        original_sender: tag(&roles[3]),
                        original_recipient: tag(&roles[4]),
                        identities: roles,
                        raw_countries,
                        stored_hour: row.hour,
                        last_command_hour: row.last_command_hour,
                        first_seen: hour,
                        last_seen: hour,
                        gone_by: None,
                        gone_by_bound: false,
                        present_at_end: true,
                        read_hour: hour,
                    },
                },
            );
            self.queue_keys.insert(signature, key);
            key
        }
    }
    fn update_proposal(&mut self, key: u64, hour: i32, last_command_hour: i32) {
        let proposal = &mut self.queue.get_mut(&key).expect("open queue").reply;
        proposal.last_seen = hour;
        proposal.read_hour = hour;
        proposal.last_command_hour = last_command_hour;
        self.dirty.insert(key);
    }
    fn end_missing_proposals(
        &mut self,
        hour: i32,
        found: &BTreeSet<u64>,
        handles: &BTreeSet<(u32, u64)>,
    ) {
        let missing: Vec<_> = self
            .queue
            .iter()
            .filter(|(key, q)| !found.contains(key) && q.reply.gone_by.is_none())
            .map(|(&key, _)| key)
            .collect();
        for key in missing {
            let q = self.queue.get_mut(&key).expect("missing queue");
            // A reused handle seen after a gap is not an absent row. Leave that
            // old episode's disappearance unknown rather than inventing one.
            let same_handle = handles.contains(&(
                q.reply.raw_countries[0],
                q.reply.handle.parse::<u64>().expect("stored full handle"),
            ));
            if !same_handle || q.reply.present_at_end {
                q.reply.gone_by = Some(hour);
                q.reply.gone_by_bound |= !q.reply.present_at_end;
            }
            q.reply.present_at_end = false;
            if self.queue_keys.get(&q.signature) == Some(&key) {
                self.queue_keys.remove(&q.signature);
            }
            self.dirty.insert(key);
        }
    }
}
