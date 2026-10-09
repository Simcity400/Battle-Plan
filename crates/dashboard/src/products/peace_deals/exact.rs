//! Exact finalizer observations override the derived card of the same identity.
use super::*;
use replay::{
    frame::TableView,
    rows::{PeaceSettlementRow, SettledPeaceAction},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceCapturedAction {
    pub derived: bool,
    pub country: u32,
    /// Event-hour tags; null when that raw reference cannot be resolved then.
    pub country_tag: Option<String>,
    pub turn: u32,
    pub ordinal: u32,
    pub token: u32,
    pub token_name: Option<String>,
    pub name_key: Option<String>,
    pub name: Option<String>,
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) cached_name: Option<Vec<u8>>,
    pub taker: u32,
    pub giver: u32,
    pub negotiator: u32,
    pub taker_tag: Option<String>,
    pub giver_tag: Option<String>,
    pub negotiator_tag: Option<String>,
    pub status: i32,
    pub cost: i32,
    pub form: u8,
    pub state: Option<i32>,
    pub flags: Option<[u8; 4]>,
    pub country_operand: Option<u32>,
    pub country_operand_tag: Option<String>,
    pub puppet_auxiliary: Option<String>,
    pub ship: Option<String>,
    pub ratio: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceCapturedTransfer {
    pub derived: bool,
    pub hour: i32,
    pub state: i32,
    pub taker: u32,
    pub to: Option<String>,
    /// A matching action's giver, absent for automatic/no-demand outcomes.
    pub giver: Option<u32>,
    pub from: Option<String>,
    pub from_country: Option<PeaceTransferCountry>,
    pub to_country: Option<PeaceTransferCountry>,
    pub automatic: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceCaptured {
    pub hour: i32,
    pub publication_hour: i32,
    /// Raw references and native order, independent of country-tag resolution.
    pub winners: Vec<u32>,
    pub losers: Vec<u32>,
    pub actions: Vec<PeaceCapturedAction>,
    pub transfers: Vec<PeaceCapturedTransfer>,
    pub subjects: Vec<PeaceCapturedSubject>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PeaceCapturedSubject {
    pub derived: bool,
    pub overlord: u32,
    pub subject: u32,
    pub overlord_tag: Option<String>,
    pub subject_tag: Option<String>,
}
struct Capture {
    row: PeaceSettlementRow,
    publication_hour: i32,
    tags: HashMap<u32, String>,
    countries: HashMap<u32, PeaceTransferCountry>,
    tokens: HashMap<u32, String>,
    vps: Option<HashMap<u32, i64>>,
    vps_hour: Option<i32>,
    state_action_key: Option<String>,
}
struct TagRead {
    first_hour: i32,
    last_hour: i32,
    tag: String,
    identity: (u32, u32),
}
#[derive(Default)]
pub(super) struct Observed {
    seen: HashSet<u64>,
    captures: Vec<Capture>,
    unobserved: Vec<HourSpan>,
    reasons: HashSet<String>,
    country_tags: HashMap<u32, Vec<TagRead>>,
}
impl Observed {
    fn missing(&mut self, hour: i32, reason: String) {
        if let Some(last) = self
            .unobserved
            .last_mut()
            .filter(|last| last.last_hour.checked_add(1) == Some(hour))
        {
            last.last_hour = hour;
        } else {
            self.unobserved.push(HourSpan {
                first_hour: hour,
                last_hour: hour,
            });
        }
        self.reasons.insert(reason);
    }
    pub(super) fn observe(
        &mut self,
        frame: &DecodedFrame,
        country_tags: Option<&Arc<HashMap<u32, String>>>,
        vps: Option<&HashMap<u32, i64>>,
    ) {
        if let Some(tags) = country_tags {
            let identities: HashMap<_, _> = frame
                .complete::<tables::Countries>()
                .unwrap_or_default()
                .iter()
                .filter_map(|row| {
                    Some((
                        std::str::from_utf8(&row.tag).ok()?,
                        (row.canonical_slot, row.id),
                    ))
                })
                .collect();
            // Retain every observed mapping without copying an unchanged map
            // for each hour. A missing read breaks each reference's run.
            for (&raw, tag) in tags.iter() {
                let Some(&identity) = identities.get(tag.as_str()) else {
                    continue;
                };
                let reads = self.country_tags.entry(raw).or_default();
                if let Some(last) = reads.last_mut().filter(|last| {
                    last.last_hour.checked_add(1) == Some(frame.hour)
                        && last.tag == *tag
                        && last.identity == identity
                }) {
                    last.last_hour = frame.hour;
                } else {
                    reads.push(TagRead {
                        first_hour: frame.hour,
                        last_hour: frame.hour,
                        tag: tag.clone(),
                        identity,
                    });
                }
            }
        }
        let rows = match &*frame.tables[TableId::PeaceSettlements.index()] {
            TableView::Complete(rows) => rows.rows::<tables::PeaceSettlements>(),
            TableView::Truncated { rows, missing } => {
                self.missing(
                    frame.hour,
                    format!(
                        "Finalizer table truncated: at least {missing} lost conferences or unread operands."
                    ),
                );
                rows.rows::<tables::PeaceSettlements>()
            }
            TableView::Unavailable { reason } => {
                self.missing(frame.hour, format!("Finalizer table unavailable: {reason}"));
                None
            }
        };
        let Some(rows) = rows else {
            return;
        };
        let tokens = frame
            .complete::<tables::LexicalTokens>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| {
                        std::str::from_utf8(&r.literal)
                            .ok()
                            .map(|name| (r.token, name.to_owned()))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        for row in rows {
            if !self.seen.insert(row.sequence) {
                continue;
            }
            // Resolve countries only through a read at the event hour. A new
            // country's tag can remain unknown while its raw reference survives.
            self.captures.push(Capture {
                row: row.clone(),
                publication_hour: frame.hour,
                tags: row
                    .hour
                    .map(|hour| {
                        row.winners
                            .iter()
                            .chain(&row.losers)
                            .copied()
                            .chain(
                                row.actions
                                    .iter()
                                    .flat_map(|a| [a.country, a.taker, a.giver, a.negotiator]),
                            )
                            .chain(row.transfers.iter().map(|t| t.taker))
                            .chain(row.subjects.iter().flat_map(|s| [s.overlord, s.subject]))
                            .chain(row.actions.iter().filter_map(|a| a.country_operand))
                            .filter_map(|raw| {
                                let reads = self.country_tags.get(&raw)?;
                                let index = reads.partition_point(|read| read.first_hour <= hour);
                                let read = reads.get(index.checked_sub(1)?)?;
                                (hour <= read.last_hour).then(|| (raw, read.tag.clone()))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                countries: row
                    .hour
                    .map(|hour| {
                        row.transfers
                            .iter()
                            .map(|transfer| transfer.taker)
                            .chain(row.actions.iter().map(|action| action.giver))
                            .collect::<HashSet<_>>()
                            .into_iter()
                            .filter_map(|raw| {
                                let reads = self.country_tags.get(&raw)?;
                                let index = reads.partition_point(|read| read.first_hour <= hour);
                                let read = reads.get(index.checked_sub(1)?)?;
                                (hour <= read.last_hour).then(|| {
                                    (
                                        raw,
                                        PeaceTransferCountry {
                                            slot: read.identity.0,
                                            country_id: read.identity.1,
                                            tag: read.tag.clone(),
                                            hour,
                                        },
                                    )
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                tokens: row
                    .actions
                    .iter()
                    .filter_map(|a| tokens.get(&a.token).cloned().map(|name| (a.token, name)))
                    .collect(),
                vps: vps.map(|vps| {
                    row.transfers
                        .iter()
                        .filter_map(|t| {
                            u32::try_from(t.state)
                                .ok()
                                .and_then(|state| vps.get(&state).copied().map(|vp| (state, vp)))
                        })
                        .collect()
                }),
                vps_hour: vps.map(|_| frame.hour),
                state_action_key: state_action_key(frame).ok(),
            });
        }
    }
    pub(super) fn apply(self, reply: &mut PeaceDealsReply) {
        reply.unobserved.settlements = self.unobserved;
        reply.warnings.extend(self.reasons);
        let mut indices: HashMap<_, _> = reply
            .deals
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.conference_id.as_ref().map(|id| (id.clone(), i)))
            .collect();
        let mut exact_states = HashSet::new();
        for capture in self.captures {
            let row = &capture.row;
            let Some(id) = row.conference else {
                reply.warnings.push(format!(
                    "A finalizer conference identity was unavailable: {}",
                    String::from_utf8_lossy(row.failure.as_deref().unwrap_or_default())
                ));
                continue;
            };
            let id = id.to_string();
            let index = *indices.entry(id.clone()).or_insert_with(|| {
                let mut deal = card(
                    format!("captured:{id}:{}", row.sequence),
                    PeaceObservation::RecordedConference,
                    row.hour.unwrap_or(capture.publication_hour),
                );
                deal.conference_id = Some(id.clone());
                deal.capture_observed_hour = Some(capture.publication_hour);
                reply.deals.push(deal);
                reply.deals.len() - 1
            });
            let deal = &mut reply.deals[index];
            if row.failure.is_some() || row.omitted != 0 || row.hour.is_none() {
                let reason = format!(
                    "Finalizer capture unavailable or truncated (at least {} unread operands): {}",
                    row.omitted,
                    row.failure
                        .as_deref()
                        .map(String::from_utf8_lossy)
                        .unwrap_or_else(|| if row.hour.is_none() {
                            "finalizer hour unavailable".into()
                        } else {
                            "capture capacity exceeded".into()
                        })
                );
                deal.capture_unavailable = Some(reason.clone());
                deal.capture_unavailable_hour = Some(capture.publication_hour);
                deal.limitations.push(reason);
                continue;
            }
            let hour = row.hour.unwrap();
            deal.derived = false;
            deal.capture_unavailable = None;
            deal.capture_unavailable_hour = None;
            deal.participants_derived = false;
            deal.finalized = Some(true);
            deal.first_finalized_hour = Some(hour);
            deal.last_observed_hour = deal.last_observed_hour.max(hour);
            deal.end_hour = Some(hour);
            deal.end_is_by = false;
            deal.limitations = vec!["Finalization destroys ordered actions; a completed save can validate retained outcomes only.".into()];
            deal.winners = row
                .winners
                .iter()
                .filter_map(|raw| capture.tags.get(raw).cloned())
                .collect();
            deal.losers = row
                .losers
                .iter()
                .filter_map(|raw| capture.tags.get(raw).cloned())
                .collect();
            if deal.winners.len() != row.winners.len() || deal.losers.len() != row.losers.len() {
                deal.limitations.push("Some participant tags are not recorded at the finalizer hour; their raw references are preserved.".into());
            }
            let mut givers: HashMap<i32, Option<u32>> = HashMap::new();
            for action in &row.actions {
                if let Some(state) = action.state {
                    givers
                        .entry(state)
                        .and_modify(|old| {
                            if *old != Some(action.giver) {
                                *old = None;
                            }
                        })
                        .or_insert(Some(action.giver));
                }
            }
            let transfers = row
                .transfers
                .iter()
                .map(|t| {
                    let giver = if t.automatic {
                        None
                    } else {
                        givers.get(&t.state).copied().flatten()
                    };
                    PeaceCapturedTransfer {
                        derived: false,
                        hour,
                        state: t.state,
                        taker: t.taker,
                        to: capture.tags.get(&t.taker).cloned(),
                        giver,
                        from: giver.and_then(|r| capture.tags.get(&r).cloned()),
                        from_country: giver.and_then(|raw| capture.countries.get(&raw).cloned()),
                        to_country: capture.countries.get(&t.taker).cloned(),
                        automatic: t.automatic,
                    }
                })
                .collect::<Vec<_>>();
            let fallback = std::mem::take(&mut deal.items);
            let undrawable = transfers
                .iter()
                .filter(|t| t.from_country.is_none() || t.to_country.is_none())
                .filter_map(|t| u32::try_from(t.state).ok())
                .collect::<HashSet<_>>();
            for t in &transfers {
                if let (Some(from_country), Some(to_country), Ok(state)) =
                    (&t.from_country, &t.to_country, u32::try_from(t.state))
                {
                    exact_states.insert((t.state, capture.publication_hour));
                    let vp = capture
                        .vps
                        .as_ref()
                        .and_then(|vps| vps.get(&state))
                        .copied();
                    deal.items.push(PeaceItem {
                        derived: false,
                        dates: PeaceDates {
                            before_hour: hour,
                            after_hour: hour,
                        },
                        winner: Some(to_country.tag.clone()),
                        action: PeaceActionLabel {
                            kind: PeaceActionKind::StateTransfer,
                            key: capture.state_action_key.clone(),
                            name: None,
                        },
                        change: PeaceChange::StateTransfer {
                            state,
                            from: from_country.tag.clone(),
                            to: to_country.tag.clone(),
                            from_country: from_country.clone(),
                            to_country: to_country.clone(),
                            victory_points: vp.map(|vp| vp.to_string()),
                            victory_points_read_hour: vp.and(capture.vps_hour),
                        },
                    });
                }
            }
            // A sampled conference can already own the dated fallback item.
            // Keep it when the raw transfer cannot supply both exact endpoints.
            deal.items.extend(fallback.into_iter().filter(|item| {
                item.derived && item.dates.after_hour == capture.publication_hour
                    && matches!(&item.change, PeaceChange::StateTransfer { state, .. } if undrawable.contains(state))
            }));
            let unknown = transfers
                .iter()
                .filter(|t| t.from_country.is_none() || t.to_country.is_none())
                .count();
            if unknown != 0 {
                deal.limitations.push(format!("{unknown} transfers with unrecorded event-hour endpoint identities remain in captured transfers; the country graph cannot draw those endpoints."));
            }
            deal.captured = Some(PeaceCaptured {
                hour,
                publication_hour: capture.publication_hour,
                winners: row.winners.clone(),
                losers: row.losers.clone(),
                actions: row
                    .actions
                    .iter()
                    .map(|a| action(a, &capture.tokens, &capture.tags))
                    .collect(),
                transfers,
                subjects: row
                    .subjects
                    .iter()
                    .map(|s| PeaceCapturedSubject {
                        derived: false,
                        overlord: s.overlord,
                        subject: s.subject,
                        overlord_tag: capture.tags.get(&s.overlord).cloned(),
                        subject_tag: capture.tags.get(&s.subject).cloned(),
                    })
                    .collect(),
            });
        }
        // Rebuild graphs after overrides. A derived unassigned card must not
        // retain the same witnessed state outcome as an exact conference.
        let mut campaign = GraphBuilder::default();
        for deal in &mut reply.deals {
            deal.items.retain(|item| {
                !item.derived
                    || !matches!(&item.change, PeaceChange::StateTransfer { state, .. }
                    if exact_states.contains(&(*state as i32, item.dates.after_hour)))
            });
            let mut graph = GraphBuilder::default();
            for item in &deal.items {
                graph.add(item);
                campaign.add(item);
            }
            deal.transfers = graph.finish();
        }
        reply.transfers = campaign.finish();
        reply
            .deals
            .retain(|deal| !deal.derived || deal.conference_id.is_some() || !deal.items.is_empty());
        reply.derived = reply
            .deals
            .iter()
            .any(|deal| deal.derived || deal.items.iter().any(|item| item.derived));
    }
}
fn action(
    a: &SettledPeaceAction,
    tokens: &HashMap<u32, String>,
    tags: &HashMap<u32, String>,
) -> PeaceCapturedAction {
    PeaceCapturedAction {
        derived: false,
        country: a.country,
        country_tag: tags.get(&a.country).cloned(),
        turn: a.turn,
        ordinal: a.ordinal,
        token: a.token,
        token_name: tokens.get(&a.token).cloned(),
        name_key: a
            .name_key
            .as_ref()
            .and_then(|key| std::str::from_utf8(key).ok().map(str::to_owned)),
        name: None,
        cached_name: a.cached_name.clone(),
        taker: a.taker,
        giver: a.giver,
        negotiator: a.negotiator,
        taker_tag: tags.get(&a.taker).cloned(),
        giver_tag: tags.get(&a.giver).cloned(),
        negotiator_tag: tags.get(&a.negotiator).cloned(),
        status: a.status,
        cost: a.cost,
        form: a.form,
        state: a.state,
        flags: a.flags,
        country_operand: a.country_operand,
        country_operand_tag: a.country_operand.and_then(|raw| tags.get(&raw).cloned()),
        puppet_auxiliary: a.puppet_auxiliary.map(|v| v.to_string()),
        ship: a.ship.map(|v| v.to_string()),
        ratio: a.ratio.map(|v| v.to_string()),
    }
}
