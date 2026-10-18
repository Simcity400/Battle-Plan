//! Exact finalizer observations override the derived card of the same identity.
use super::*;
use replay_data::rows::SettledPeaceAction;
use replay_frame::frame::TableView;

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
/// Finalizer facts projected through the same event-hour naming and identity
/// joins as the reply. No replay row is retained in a prepared checkpoint.
#[derive(Serialize, Deserialize)]
struct Settlement {
    sequence: u64,
    conference: Option<u64>,
    hour: Option<i32>,
    omitted: u64,
    failure: Option<Vec<u8>>,
    winners: Vec<u32>,
    losers: Vec<u32>,
    actions: Vec<PeaceCapturedAction>,
    transfers: Vec<PeaceCapturedTransfer>,
    subjects: Vec<PeaceCapturedSubject>,
    cached_names: Vec<Option<Vec<u8>>>,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Capture {
    row: Settlement,
    publication_hour: i32,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    tags: HashMap<u32, String>,
    #[serde(skip)]
    countries: HashMap<u32, PeaceTransferCountry>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_optional_map")]
    vps: Option<HashMap<u32, i64>>,
    vps_hour: Option<i32>,
    state_action_key: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct TagRead {
    first_hour: i32,
    last_hour: i32,
    tag: Tag,
    identity: (u32, u32),
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Observed {
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_set")]
    seen: HashSet<u64>,
    #[serde(skip)]
    pub(super) captures: Vec<Capture>,
    unobserved: Vec<HourSpan>,
    /// Ordered, so a reply's warnings do not depend on hashing.
    reasons: std::collections::BTreeSet<String>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    country_tags: HashMap<u32, Vec<TagRead>>,
    frontier: Option<i32>,
    materialized: Option<i32>,
    active: std::collections::BTreeSet<u32>,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) row_visits: usize,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) history_visits: usize,
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
    /// `action_key` is this frame's `state_action_key`, read once per LexicalTokens view.
    pub(super) fn seal(&mut self) {
        let Some((from, through)) = self
            .materialized
            .zip(self.frontier)
            .filter(|(from, through)| from < through)
        else {
            return;
        };
        for raw in &self.active {
            #[cfg(test)]
            {
                self.history_visits += 1;
            }
            if let Some(last) = self
                .country_tags
                .get_mut(raw)
                .and_then(|reads| reads.last_mut())
                .filter(|last| last.last_hour == from)
            {
                last.last_hour = through;
            }
        }
        self.materialized = Some(through);
    }
    pub(super) fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        countries: Option<&CountryTags>,
        vps: Option<&HashMap<u32, i64>>,
        action_key: Option<&str>,
        holders_changed: bool,
        changes: &FrameChanges,
    ) {
        let continuous = self.frontier.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        if holders_changed || !continuous {
            self.seal();
            self.active.clear();
            if let Some(countries) = countries {
                // Retain every observed mapping without copying an unchanged map
                // for each hour. A missing read breaks each reference's run.
                for (&raw, tag) in &countries.tags {
                    #[cfg(test)]
                    {
                        self.row_visits += 1;
                    }
                    let identity = countries.identities[tag];
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
            self.active.extend(
                countries
                    .into_iter()
                    .flat_map(|countries| countries.tags.keys())
                    .copied(),
            );
            self.materialized = Some(frame.hour);
        }
        self.frontier = Some(frame.hour);
        let view = &*frame.tables[TableId::PeaceSettlements.index()];
        if !changes.rows_changed[TableId::PeaceSettlements.index()]
            && matches!(view, TableView::Complete(_))
        {
            return;
        }
        if let Some(reason) = missing_reason(view) {
            self.missing(frame.hour, reason);
        }
        let rows = match view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
                rows.rows::<tables::PeaceSettlements>()
            }
            TableView::Unavailable { .. } => None,
        };
        let Some(rows) = rows else {
            return;
        };
        if !changes.rows_changed[TableId::PeaceSettlements.index()] {
            return;
        }
        // Late finalizer facts can name any accepted hour, including this one.
        self.seal();
        let tokens = frame.complete::<tables::LexicalTokens>().ok();
        for row in rows {
            #[cfg(test)]
            {
                self.row_visits += 1;
            }
            if !self.seen.insert(row.sequence) {
                continue;
            }
            // Resolve countries only through a read at the event hour. A new
            // country's tag can remain unknown while its raw reference survives.
            let mut capture = Capture {
                row: Settlement {
                    sequence: row.sequence,
                    conference: row.conference,
                    hour: row.hour,
                    omitted: row.omitted,
                    failure: row.failure.clone(),
                    winners: row.winners.clone(),
                    losers: row.losers.clone(),
                    actions: Vec::new(),
                    transfers: Vec::new(),
                    subjects: Vec::new(),
                    cached_names: Vec::new(),
                },
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
                                (hour <= read.last_hour).then(|| (raw, read.tag.to_string()))
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
                                            tag: read.tag.to_string(),
                                            hour,
                                        },
                                    )
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
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
                state_action_key: action_key.map(str::to_owned),
            };
            let tokens: HashMap<u32, String> = row
                .actions
                .iter()
                .filter_map(|a| {
                    let tokens = tokens?;
                    let index = tokens.binary_search_by_key(&a.token, |r| r.token).ok()?;
                    Some((
                        a.token,
                        std::str::from_utf8(&tokens[index].literal).ok()?.to_owned(),
                    ))
                })
                .collect();
            capture.row.actions = row
                .actions
                .iter()
                .map(|a| action(a, &tokens, &capture.tags))
                .collect();
            capture.row.cached_names = capture
                .row
                .actions
                .iter()
                .map(|a| a.cached_name.clone())
                .collect();
            let mut givers: HashMap<i32, Option<u32>> = HashMap::new();
            for action in &capture.row.actions {
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
            capture.row.transfers = row
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
                        hour: row.hour.unwrap_or(capture.publication_hour),
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

            capture.row.subjects = row
                .subjects
                .iter()
                .map(|s| PeaceCapturedSubject {
                    derived: false,
                    overlord: s.overlord,
                    subject: s.subject,
                    overlord_tag: capture.tags.get(&s.overlord).cloned(),
                    subject_tag: capture.tags.get(&s.subject).cloned(),
                })
                .collect();
            capture
                .tags
                .retain(|raw, _| row.winners.contains(raw) || row.losers.contains(raw));
            self.captures.push(capture);
        }
    }
    /// Whether `hour` follows the last hour observed.
    pub(super) fn continues(&self, hour: i32) -> bool {
        self.frontier.and_then(|frontier| frontier.checked_add(1)) == Some(hour)
    }
    /// Hours `first..=last` that continue the last hour observed, with the
    /// same holders and an unchanged finalizer table, as
    /// [`Self::observe_changed`] observes each: the frontier advances, and
    /// while the table is not complete (`missing`, its reason) each hour is
    /// a missing one.
    pub(super) fn advance_quiet(&mut self, first: i32, last: i32, missing: Option<String>) {
        if let Some(reason) = missing {
            self.missing(first, reason);
            if let Some(span) = self.unobserved.last_mut() {
                span.last_hour = last;
            }
        }
        self.frontier = Some(last);
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
        // A finalizer outcome for a released country can lack a giver. The ownership
        // comparison at its publication hour names each state's former owner.
        let releases: HashMap<(u32, i32), (u32, u32)> = reply
            .deals
            .iter()
            .flat_map(|deal| &deal.items)
            .filter(|item| item.derived)
            .flat_map(|item| match &item.change {
                PeaceChange::CountryReleased {
                    to_country,
                    transfers,
                    ..
                } => transfers
                    .iter()
                    .map(|transfer| {
                        (
                            (transfer.state, item.dates.after_hour),
                            to_country.identity(),
                        )
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
        let mut released_exact = HashSet::new();
        let mut kept_releases: HashMap<usize, Vec<PeaceItem>> = HashMap::new();
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
            let transfers = row.transfers.clone();
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
                    if releases.get(&(state, capture.publication_hour))
                        == Some(&to_country.identity())
                    {
                        released_exact.insert((state, hour, to_country.identity()));
                    }
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
            // A release's undrawable states feed only the graph: the captured
            // outcomes already list them for the card.
            let releases_kept: Vec<_> = fallback
                .iter()
                .filter(|item| item.derived && item.dates.after_hour == capture.publication_hour)
                .filter_map(|item| {
                    let mut kept = item.clone();
                    let PeaceChange::CountryReleased { transfers, .. } = &mut kept.change else {
                        return None;
                    };
                    transfers.retain(|transfer| undrawable.contains(&transfer.state));
                    (!transfers.is_empty()).then_some(kept)
                })
                .collect();
            if !releases_kept.is_empty() {
                kept_releases
                    .entry(index)
                    .or_default()
                    .extend(releases_kept);
            }
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
                    .cloned()
                    .zip(&row.cached_names)
                    .map(|(mut action, cached)| {
                        action.cached_name = cached.clone();
                        action
                    })
                    .collect(),
                transfers,
                subjects: row.subjects.clone(),
            });
        }
        // Rebuild graphs after overrides. A derived unassigned card must not
        // retain the same witnessed state outcome as an exact conference.
        let mut campaign = GraphBuilder::default();
        for (index, deal) in reply.deals.iter_mut().enumerate() {
            deal.items.retain(|item| {
                !item.derived
                    || !matches!(&item.change, PeaceChange::StateTransfer { state, .. }
                    if exact_states.contains(&(*state as i32, item.dates.after_hour)))
            });
            let kept = kept_releases.remove(&index).unwrap_or_default();
            let mut graph = GraphBuilder::default();
            for item in deal.items.iter().chain(&kept) {
                let released = match &item.change {
                    PeaceChange::StateTransfer {
                        state, to_country, ..
                    } => {
                        !item.derived
                            && released_exact.contains(&(
                                *state,
                                item.dates.after_hour,
                                to_country.identity(),
                            ))
                    }
                    _ => false,
                };
                // A released state the finalizer drew exactly belongs to its conference.
                let keep =
                    |state: u32| !exact_states.contains(&(state as i32, item.dates.after_hour));
                graph.add_with(item, released, &keep);
                campaign.add_with(item, released, &keep);
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
/// Why an hour's finalizer table misses conferences, as that hour records
/// it; None for a complete table.
pub(super) fn missing_reason(view: &TableView) -> Option<String> {
    match view {
        TableView::Complete(_) => None,
        TableView::Truncated { missing, .. } => Some(format!(
            "Finalizer table truncated: at least {missing} lost conferences or unread operands."
        )),
        TableView::Unavailable { reason } => Some(format!("Finalizer table unavailable: {reason}")),
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
