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
            self.number(Fixed::QueueCoverage, hour, None);
            return Ok(());
        }
        let fresh = slot_read(
            format,
            &frame.metrics,
            TableId::IncomingDiplomaticActions,
            hour,
        );
        if !fresh {
            self.number(Fixed::QueueCoverage, hour, self.queue_read.map(i128::from));
            return Ok(());
        }

        let rows = frame.complete::<tables::IncomingDiplomaticActions>()?;
        let projected = project(rows.iter(), joins)?;
        self.observe_queue(hour, &projected, check)
    }
    pub(super) fn observe_queue(
        &mut self,
        hour: i32,
        projected: &Queue,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        self.number(
            Fixed::QueueDuplicates,
            hour,
            (projected.duplicates > 0).then_some(projected.duplicates as i128),
        );
        if projected.duplicates > 0 {
            self.suspend_queue();
            self.number(Fixed::QueueCoverage, hour, None);
            return Ok(());
        }
        let mut found = BTreeSet::new();
        let mut numbered = BTreeMap::new();
        for (&wire_key, proposal) in &projected.rows {
            check()?;
            let key = match self.queue_numbered.get(&wire_key) {
                Some((held, key)) if Arc::ptr_eq(held, proposal) => *key,
                _ => self.begin_proposal(
                    hour,
                    &proposal.row,
                    proposal.signature.clone(),
                    proposal.roles.clone(),
                    proposal.raw_countries.clone(),
                ),
            };
            if !found.insert(key) {
                return Err(io::Error::other(
                    "Duplicate incoming diplomatic action handle in one owner queue",
                ));
            }
            self.update_proposal(key, hour, proposal.row.last_command_hour);
            numbered.insert(wire_key, (Arc::clone(proposal), key));
        }
        self.end_missing_proposals(hour, &found, &projected.handles);
        self.queue_numbered = numbered;
        self.queue_read = Some(hour);
        self.number(Fixed::QueueCoverage, hour, Some(i128::from(hour)));
        Ok(())
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
        handles: &BTreeMap<(u32, u64), usize>,
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
            let same_handle = handles.contains_key(&(
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

pub(super) struct Proposal {
    row: IncomingDiplomaticActionRow,
    signature: String,
    roles: Vec<Option<DiplomacyCountry>>,
    raw_countries: Vec<u32>,
}
#[derive(Clone, Default)]
pub(super) struct Queue {
    rows: BTreeMap<u128, Arc<Proposal>>,
    handles: BTreeMap<(u32, u64), usize>,
    duplicates: usize,
}
impl Queue {
    pub(super) fn set(&mut self, key: u128, proposal: Option<Arc<Proposal>>) {
        if let Some(old) = self.rows.remove(&key) {
            let handle = (old.row.country, old.row.handle);
            let count = self.handles.get_mut(&handle).expect("queued handle");
            self.duplicates -= usize::from(*count > 1);
            *count -= 1;
            if *count == 0 {
                self.handles.remove(&handle);
            }
        }
        if let Some(proposal) = proposal {
            let count = self
                .handles
                .entry((proposal.row.country, proposal.row.handle))
                .or_default();
            self.duplicates += usize::from(*count > 0);
            *count += 1;
            self.rows.insert(key, proposal);
        }
    }
}
pub(super) fn project_one(
    row: &IncomingDiplomaticActionRow,
    joins: &Joins,
) -> io::Result<Arc<Proposal>> {
    let raw_countries = vec![
        row.country,
        row.actor,
        row.recipient,
        row.original_actor,
        row.original_recipient,
    ];
    let roles: Vec<_> = raw_countries
        .iter()
        .map(|raw| joins.raw.get(raw).cloned())
        .collect();
    // Ordinal, wrapper status and action_type establish no episode identity.
    let signature = serde_json::to_string(&(row.handle, &raw_countries, &roles, row.hour))?;
    Ok(Arc::new(Proposal {
        row: row.clone(),
        signature,
        roles,
        raw_countries,
    }))
}
pub(super) fn project<'a>(
    rows: impl Iterator<Item = &'a IncomingDiplomaticActionRow>,
    joins: &Joins,
) -> io::Result<Queue> {
    let mut projected = Queue::default();
    for row in rows {
        projected.set(row.key(), Some(project_one(row, joins)?));
    }
    Ok(projected)
}
