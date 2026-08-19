//! Timeline abstraction: who owns and controls each state at each hour.
//!
//! Scrubbing consumes an [`OwnerProvider`]; the renderer rewrites only the
//! compact state -> color table from `controllers_at(hour)` for validation
//! while visible political recoloring is supplied by the authenticated XMAP05
//! path. `owners_at(hour)` stays
//! exposed because occupied-territory striping needs both maps.
//!
//! Frames are returned as `Arc` so a recording-backed provider can load
//! them lazily (a real recording is far too large to materialize every
//! hour up front) while the static provider stays allocation-free on repeat
//! calls.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use replay_core::{Leaf, ReplaySource, WorldFrame, WorldTimeline};

use crate::anomaly::AnomalyPolicy;
use crate::bundle::MapBundle;
use crate::{Result, TimelineMode};

/// A headless recording selection. Canonical hours are absolute `hour_index`
/// values; ordinals are zero-based positions in the verified recording index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpHourRequest {
    Canonical(i64),
    Ordinal(usize),
}

/// Loud, typed failures for recording-hour selection and frame loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineSelectionError {
    Empty,
    BeforeFirst {
        requested: i64,
        first: i64,
        last: i64,
    },
    AfterLast {
        requested: i64,
        first: i64,
        last: i64,
    },
    MissingCanonical {
        requested: i64,
        first: i64,
        last: i64,
    },
    GapUnavailable {
        requested: i64,
        from: i64,
        to: i64,
        missing: Vec<i64>,
    },
    OrdinalOutOfRange {
        requested: usize,
        count: usize,
        first: i64,
        last: i64,
    },
    FrameLoad {
        requested: i64,
        first: i64,
        last: i64,
        detail: String,
    },
}

impl std::fmt::Display for TimelineSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "recording has no canonical hours"),
            Self::BeforeFirst { requested, first, last } => write!(
                f,
                "requested canonical hour {requested} is before the recording's valid canonical range {first}..={last}"
            ),
            Self::AfterLast { requested, first, last } => write!(
                f,
                "requested canonical hour {requested} is after the recording's valid canonical range {first}..={last}"
            ),
            Self::MissingCanonical { requested, first, last } => write!(
                f,
                "requested canonical hour {requested} is not a recorded snapshot in the recording's valid canonical range {first}..={last}"
            ),
            Self::GapUnavailable {
                requested,
                from,
                to,
                missing,
            } => write!(
                f,
                "requested hour {requested} is unavailable inside the recording gap {from}->{to} (missing {missing:?}); refusing interpolation"
            ),
            // `count == 0` never escapes the constructors today
            // (`canonical_range` returns `Empty` first and `WorldTimeline`
            // rejects an empty hour series), but a `pub` `Display` must not
            // hide a latent `usize` underflow panic behind that invariant.
            Self::OrdinalOutOfRange { requested, count: 0, first, last } => write!(
                f,
                "requested dump snapshot ordinal {requested} but the recording has no dump snapshots (canonical range {first}..={last})"
            ),
            Self::OrdinalOutOfRange { requested, count, first, last } => write!(
                f,
                "requested dump snapshot ordinal {requested} is outside the recording's valid ordinal range 0..={} (canonical range {first}..={last})",
                count - 1
            ),
            Self::FrameLoad { requested, first, last, detail } => write!(
                f,
                "failed to load requested canonical hour {requested} from the recording's valid canonical range {first}..={last}: {detail}"
            ),
        }
    }
}

impl std::error::Error for TimelineSelectionError {}

fn canonical_range(hours: &[i64]) -> std::result::Result<(i64, i64), TimelineSelectionError> {
    let first = *hours.first().ok_or(TimelineSelectionError::Empty)?;
    let last = *hours.last().ok_or(TimelineSelectionError::Empty)?;
    Ok((first, last))
}

/// Resolve either public dump selector to an exact canonical recorded hour.
/// This is deliberately stricter than timeline scrubbing, which may resolve
/// an in-between time to the preceding frame.
pub fn resolve_dump_hour(
    hours: &[i64],
    request: DumpHourRequest,
) -> std::result::Result<i64, TimelineSelectionError> {
    let (first, last) = canonical_range(hours)?;
    match request {
        DumpHourRequest::Ordinal(requested) => {
            hours
                .get(requested)
                .copied()
                .ok_or(TimelineSelectionError::OrdinalOutOfRange {
                    requested,
                    count: hours.len(),
                    first,
                    last,
                })
        }
        DumpHourRequest::Canonical(requested) if requested < first => {
            Err(TimelineSelectionError::BeforeFirst {
                requested,
                first,
                last,
            })
        }
        DumpHourRequest::Canonical(requested) if requested > last => {
            Err(TimelineSelectionError::AfterLast {
                requested,
                first,
                last,
            })
        }
        DumpHourRequest::Canonical(requested) => {
            if hours.binary_search(&requested).is_ok() {
                Ok(requested)
            } else {
                Err(TimelineSelectionError::MissingCanonical {
                    requested,
                    first,
                    last,
                })
            }
        }
    }
}

/// FW7/FW11 seam closure: before a bundle's province table is allowed to act
/// as the anomaly/province authority for a recording, the bundle's declared
/// capture-side modset and the recording's own declared modset must be
/// reconciled.
///
/// A definite class disagreement (one side vanilla, the other modded) is a
/// typed refusal: the [`replay_core::SourceError::ModsetMismatch`] value is
/// boxed AS ITSELF, never flattened to a string, so a caller can still
/// downcast it and classify the rejection (`validate::rejection_kind` maps it
/// to `RejectionKind::Modset`). Anything the recorded evidence cannot decide
/// is logged as a typed degradation. There is no silent third path: every
/// outcome either refuses or prints.
fn reconcile_bundle_against_recording(
    bundle: Option<&MapBundle>,
    manifest: Option<&replay_core::ContentManifest>,
) -> Result<()> {
    let Some(bundle) = bundle else {
        // No bundle: nothing of the bundle's is used as an authority, so there
        // is no cross-check to make.
        return Ok(());
    };
    // `?` on a `SourceError` boxes the typed value (it implements
    // `std::error::Error`); do NOT stringify it here.
    let agreement = crate::bundle::reconcile_bundle_modset(&bundle.provenance, manifest)?;
    // Positive agreement is the only quiet-ish outcome, and it still logs what
    // was established. A non-decidable outcome must never be silence.
    eprintln!("replay_app: {}", agreement.diagnostic());
    Ok(())
}

/// Build the owner provider + mode label for a timeline mode. Shared by the
/// interactive window loop and the headless frame-dump path.
pub fn build_provider(
    bundle: Option<&MapBundle>,
    mode: &TimelineMode,
) -> Result<(Arc<dyn OwnerProvider>, String)> {
    match mode {
        TimelineMode::Recording {
            path,
            logical_stream_id,
            timeline: config,
        } => {
            let source =
                crate::recording_open::open_product_recording_segment(path, *logical_stream_id)
                    .map_err(|err| format!("open recording {}: {err}", path.display()))?;
            // F2: the decoded-frame LRU budget rides the mode (default 16;
            // `--frame-cache` shrinks it for weak-PC profiles).
            let timeline = replay_core::WorldTimeline::open_with_config(source, *config)
                .map_err(|err| format!("open timeline {}: {err}", path.display()))?;
            if let Some(receipt) = timeline.prefold_receipt() {
                eprintln!(
                    "replay_app: compact source certified by {} ({} bytes, sha256 {})",
                    receipt.decoder_id(),
                    receipt.source_len(),
                    receipt.source_sha256()
                );
            }
            let warning_count = timeline.warnings().len();
            for warning in timeline.warnings() {
                eprintln!("replay_app: recording warning: {warning}");
            }
            // The bundle's tables may not act as an authority for this
            // recording until the two declared modsets have been reconciled.
            reconcile_bundle_against_recording(bundle, timeline.content_manifest())?;
            // F4 anomaly validation is incremental. Opening establishes the
            // gap policy from index metadata only; frames are authenticated and
            // observed in order through the requested hour before that frame
            // can cross the provider boundary.
            let known_provinces = bundle.map(|bundle| {
                bundle
                    .province_to_state
                    .keys()
                    .copied()
                    .collect::<BTreeSet<u32>>()
            });
            let policy = AnomalyPolicy::with_known_provinces(timeline.hours(), known_provinces);
            for diagnostic in policy.diagnostics() {
                eprintln!("{}", diagnostic.log_line());
            }
            let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));
            // Coverage/ingest warnings must stay visible in a windowed launch
            // (stderr is invisible on a double-click start): fold a non-zero
            // count into the mode label, which flows into the window title.
            let label = if warning_count == 0 {
                format!("recording {}", recording.game_id())
            } else {
                format!(
                    "recording {} [{warning_count} recording warning(s) — see log]",
                    recording.game_id()
                )
            };
            Ok((Arc::new(recording), label))
        }
        TimelineMode::Static1936 => {
            let bundle = bundle.ok_or(
                "static 1936 debug mode requires --bundle; production playback uses --recording and never requires a pre-generated map bundle",
            )?;
            Ok((
                Arc::new(Static1936::new(bundle.state_owner_1936.clone())),
                String::from("static 1936"),
            ))
        }
    }
}

/// Build a recording provider from an already-admitted source. Headless
/// rollover playback uses this entry so the retained container is never
/// translated back into a pathname and reopened.
pub fn build_recording_provider(
    bundle: Option<&MapBundle>,
    source: Box<dyn ReplaySource>,
    config: replay_core::TimelineConfig,
    source_label: &str,
) -> Result<(Arc<dyn OwnerProvider>, String)> {
    let timeline = replay_core::WorldTimeline::open_with_config(source, config)
        .map_err(|err| format!("open timeline {source_label}: {err}"))?;
    if let Some(receipt) = timeline.prefold_receipt() {
        eprintln!(
            "replay_app: compact source certified by {} ({} bytes, sha256 {})",
            receipt.decoder_id(),
            receipt.source_len(),
            receipt.source_sha256()
        );
    }
    let warning_count = timeline.warnings().len();
    for warning in timeline.warnings() {
        eprintln!("replay_app: recording warning: {warning}");
    }
    // Same seam as `build_provider`: the retained-container rollover/bench
    // entry must reconcile before the bundle becomes an authority too.
    reconcile_bundle_against_recording(bundle, timeline.content_manifest())?;
    let known_provinces = bundle.map(|bundle| {
        bundle
            .province_to_state
            .keys()
            .copied()
            .collect::<BTreeSet<u32>>()
    });
    let policy = AnomalyPolicy::with_known_provinces(timeline.hours(), known_provinces);
    for diagnostic in policy.diagnostics() {
        eprintln!("{}", diagnostic.log_line());
    }
    let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));
    let label = if warning_count == 0 {
        format!("recording {}", recording.game_id())
    } else {
        format!(
            "recording {} [{warning_count} recording warning(s) - see log]",
            recording.game_id()
        )
    };
    Ok((Arc::new(recording), label))
}

/// Hour-indexed state ownership. Hours are ascending and non-empty.
pub trait OwnerProvider: Send + Sync {
    fn hours(&self) -> &[i64];
    fn owners_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>>;
    fn controllers_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>>;
    /// C7 render wiring: the typed replay_core frame for this hour. Static
    /// providers return `None`; consumers never parse recording bytes.
    fn world_frame_at(&self, _hour: i64) -> Result<Option<Arc<WorldFrame>>> {
        Ok(None)
    }
    /// Human-readable label for the title bar; recording-backed providers
    /// add the in-game date.
    fn hour_label(&self, hour: i64) -> Result<String> {
        Ok(format!("hour {hour}"))
    }
    /// The recorded `date` string for an exact recorded hour, when the frame
    /// carried one. Used to build the transport's date spine (seek-bar ticks +
    /// jump-to-date). Defaults to `None` (dateless providers degrade honestly).
    fn date_at(&self, _hour: i64) -> Result<Option<String>> {
        Ok(None)
    }
    /// E6: the observer country picker source, derived from the recording's
    /// complete d18 roster. Defaults to fail-closed (no typed recording frame).
    ///
    /// `Ok(PovPicker::Unavailable)` is reserved for a GENUINELY absent or
    /// unusable session carrier in an authenticated frame; a selection or
    /// frame authentication failure is `Err` — an authentication failure must
    /// never be downgraded to a typed-absent picker.
    fn pov_picker(&self) -> Result<crate::observer::PovPicker> {
        Ok(crate::observer::PovPicker::Unavailable(
            "static baseline has no typed d18 country roster".to_string(),
        ))
    }

    /// F4: the marked held-value note (mid-playback anomaly holds) for `hour`,
    /// or `None` when nothing is held. Defaults to `None` (no anomaly policy).
    fn anomaly_note(
        &self,
        _hour: i64,
    ) -> std::result::Result<Option<String>, TimelineSelectionError> {
        Ok(None)
    }

    /// Install a stop switch the frame path may consult, so a scrub that has
    /// moved on can abandon a reconstruction still in flight. Providers that do
    /// no abandonable work ignore it, which is the default.
    fn set_scrub_interrupt(&self, _interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {}

    fn content_manifest(&self) -> Option<&replay_core::ContentManifest> {
        None
    }


    fn decision_catalog(&self) -> Option<&replay_core::PackagedDecisionCatalog> {
        None
    }

    fn static_artifact(&self, _name: &str) -> Option<&replay_core::PackagedStaticArtifact> {
        None
    }

    fn admit_playback_content(
        &self,
        _inventory: &replay_core::PlaybackContentInventory,
    ) -> std::result::Result<
        replay_core::ValidatedContentAdmission,
        replay_core::ContentValidationReport,
    > {
        Err(replay_core::ContentValidationReport {
            compatible: false,
            verified_required_content: 0,
            issues: vec![replay_core::ContentIssue {
                code: replay_core::ContentIssueCode::InvalidManifest,
                subject: "timeline".into(),
                message: "static timeline has no portable content manifest".into(),
            }],
        })
    }
}

/// Single-frame provider: the grounded 1936 start owners from
/// `state_owner_1936.json`.
pub struct Static1936 {
    hours: Vec<i64>,
    owners: Arc<BTreeMap<u32, String>>,
}

impl Static1936 {
    pub fn new(owners: BTreeMap<u32, String>) -> Static1936 {
        Static1936 {
            hours: vec![0],
            owners: Arc::new(owners),
        }
    }
}

impl OwnerProvider for Static1936 {
    fn hours(&self) -> &[i64] {
        &self.hours
    }

    fn owners_at(&self, _hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(Arc::clone(&self.owners))
    }

    fn controllers_at(&self, _hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(Arc::clone(&self.owners))
    }
}

/// Recording-backed provider over a `replay_core::WorldTimeline`.
///
/// Trichotomy rule: only a `Leaf::Measured` owner becomes a map entry;
/// `Absent` and `Unobtained` states stay off the table, so the renderer
/// shows the no-owner sentinel (the grounded underlay) instead of a
/// fabricated owner color.
pub struct RecordingTimeline {
    timeline: WorldTimeline,
    /// One-slot cache: (requested hour, owners, controllers, date label). The
    /// WorldTimeline keeps its own LRU of decoded frames, so a miss here is
    /// only a cheap re-conversion, but the probe path calls per mouse-move
    /// and must not re-walk the leaf map every event.
    cache: Mutex<Option<CachedFrame>>,
    /// F4 mid-playback anomaly policy (populated at open by `build_provider`).
    /// Holds the marked last-good state so the UI can surface held domains.
    anomaly: Option<Mutex<IncrementalAnomalyScan>>,
}

struct IncrementalAnomalyScan {
    policy: AnomalyPolicy,
    /// First canonical hour not yet authenticated and observed.
    next_index: usize,
}

struct CachedFrame {
    hour: i64,
    owners: Arc<BTreeMap<u32, String>>,
    controllers: Arc<BTreeMap<u32, String>>,
    date: Option<String>,
}

impl RecordingTimeline {
    pub fn new_with_policy(
        timeline: WorldTimeline,
        anomaly: Option<AnomalyPolicy>,
    ) -> RecordingTimeline {
        RecordingTimeline {
            timeline,
            cache: Mutex::new(None),
            anomaly: anomaly.map(|policy| {
                Mutex::new(IncrementalAnomalyScan {
                    policy,
                    next_index: 0,
                })
            }),
        }
    }

    fn scan_anomalies_through(
        &self,
        target_hour: i64,
    ) -> std::result::Result<(), TimelineSelectionError> {
        let Some(scan) = &self.anomaly else {
            return Ok(());
        };
        let mut scan = scan.lock().map_err(|_| TimelineSelectionError::FrameLoad {
            requested: target_hour,
            first: self.timeline.first_hour(),
            last: self.timeline.last_hour(),
            detail: "F4 incremental anomaly policy lock poisoned".to_string(),
        })?;
        let end = self
            .timeline
            .hours()
            .partition_point(|hour| *hour <= target_hour);
        while scan.next_index < end {
            let hour = self.timeline.hours()[scan.next_index];
            let frame =
                self.timeline
                    .frame(hour)
                    .map_err(|error| TimelineSelectionError::FrameLoad {
                        requested: target_hour,
                        first: self.timeline.first_hour(),
                        last: self.timeline.last_hour(),
                        detail: format!(
                            "F4 fail-closed incremental validation failed at hour {hour}: {error}"
                        ),
                    })?;
            for diagnostic in scan.policy.observe(&frame) {
                eprintln!("{}", diagnostic.log_line());
            }
            scan.next_index += 1;
        }
        Ok(())
    }

    /// Resolve an exact recorded scrub position. Any in-range hole is a typed
    /// gap refusal before a frame, cached map, or date can cross the provider
    /// boundary.
    fn selected_canonical_hour(
        &self,
        hour: i64,
    ) -> std::result::Result<i64, TimelineSelectionError> {
        let (first, last) = canonical_range(self.timeline.hours())?;
        if hour < first {
            return Err(TimelineSelectionError::BeforeFirst {
                requested: hour,
                first,
                last,
            });
        }
        if hour > last {
            return Err(TimelineSelectionError::AfterLast {
                requested: hour,
                first,
                last,
            });
        }
        match self.timeline.hours().binary_search(&hour) {
            Ok(index) => Ok(self.timeline.hours()[index]),
            Err(index) => {
                let from = self.timeline.hours()[index - 1];
                let to = self.timeline.hours()[index];
                Err(TimelineSelectionError::GapUnavailable {
                    requested: hour,
                    from,
                    to,
                    missing: ((from + 1)..to).collect(),
                })
            }
        }
    }

    /// F4: the marked held-value note for the exact recorded `hour`, e.g.
    /// "anomaly: HELD weather from hour 41", or `None` when nothing is held
    /// (pre-anomaly hours, or a clean recording). An hour that lands inside a
    /// detected gap is reported as unavailable, never interpolated.
    fn anomaly_note_at(
        &self,
        hour: i64,
    ) -> std::result::Result<Option<String>, TimelineSelectionError> {
        let frame_hour = self.selected_canonical_hour(hour)?;
        self.scan_anomalies_through(frame_hour)?;
        let Some(scan) = &self.anomaly else {
            return Ok(None);
        };
        let scan = scan.lock().map_err(|_| TimelineSelectionError::FrameLoad {
            requested: hour,
            first: self.timeline.first_hour(),
            last: self.timeline.last_hour(),
            detail: "F4 incremental anomaly policy lock poisoned".to_string(),
        })?;
        let policy = &scan.policy;
        let mut parts: Vec<String> = Vec::new();
        // Note a gap immediately preceding this frame (the after-gap hour).
        if let Some(gap) = policy.gaps().iter().find(|g| g.to == frame_hour) {
            parts.push(format!("gap before (missing {:?})", gap.missing));
        }
        for hold in policy.active_holds_at(frame_hour) {
            parts.push(hold.marker);
        }
        if parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(format!("anomaly: {}", parts.join("; "))))
        }
    }

    pub fn game_id(&self) -> &str {
        self.timeline.game_id()
    }

    /// E1 wiring: random-access seek through the `replay_core` hook. Returns
    /// the exact recorded hour, or a typed selection/load error. Playback uses
    /// [`WorldTimeline::seek`] / [`WorldTimeline::play_to`]
    /// which resolve identically today (see the `seek_equals_play_to`
    /// conformance gate); the interactive scrub reads owners through the same
    /// `state_at` primitive.
    ///
    /// **Callers today are gates, not the render loop** (the compiler reports
    /// it unused in the non-test build). The Win32 loop drives
    /// [`crate::transport`] and reads `current_hour()` back through
    /// `selected_frame`/`owners_at`. This method and
    /// [`Self::play_to_hour`] are the pair the `seek_equals_play_to`
    /// conformance gate and the G2 end-to-end script assert resolve
    /// identically on every recorded hour, over the same
    /// `selected_canonical_hour` + `scan_anomalies_through` prologue the render
    /// loop takes. They stay because that equivalence is the claim, and because
    /// The `PROJECT.md` heading "Time, identity, sessions, and files" requires
    /// restoration at an hour rather than a frame index.
    pub fn seek_hour(&self, hour: i64) -> std::result::Result<i64, TimelineSelectionError> {
        let canonical_hour = self.selected_canonical_hour(hour)?;
        self.scan_anomalies_through(canonical_hour)?;
        self.timeline
            .seek(hour)
            .map(|frame| frame.hour_index())
            .map_err(|error| TimelineSelectionError::FrameLoad {
                requested: hour,
                first: self.timeline.first_hour(),
                last: self.timeline.last_hour(),
                detail: error.to_string(),
            })
    }

    /// E1 wiring: linear play-to through the `replay_core` hook. The linear
    /// half of the conformance pair documented on [`Self::seek_hour`]; same
    /// gate-only caller set today.
    pub fn play_to_hour(&self, hour: i64) -> std::result::Result<i64, TimelineSelectionError> {
        let canonical_hour = self.selected_canonical_hour(hour)?;
        self.scan_anomalies_through(canonical_hour)?;
        self.timeline
            .play_to(hour)
            .map(|frame| frame.hour_index())
            .map_err(|error| TimelineSelectionError::FrameLoad {
                requested: hour,
                first: self.timeline.first_hour(),
                last: self.timeline.last_hour(),
                detail: error.to_string(),
            })
    }

    fn selected_frame(&self, hour: i64) -> Result<Arc<WorldFrame>> {
        let canonical_hour = self.selected_canonical_hour(hour)?;
        self.scan_anomalies_through(canonical_hour)?;
        self.timeline.state_at(hour).map_err(|err| {
            TimelineSelectionError::FrameLoad {
                requested: hour,
                first: self.timeline.first_hour(),
                last: self.timeline.last_hour(),
                detail: err.to_string(),
            }
            .into()
        })
    }

    fn frame_maps(
        &self,
        hour: i64,
    ) -> Result<(
        Arc<BTreeMap<u32, String>>,
        Arc<BTreeMap<u32, String>>,
        Option<String>,
    )> {
        // Validate before consulting the one-slot cache. A map cached for an
        // prior exact lookup must never make a later unavailable gap look
        // playable.
        self.selected_canonical_hour(hour)?;
        if let Ok(guard) = self.cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.hour == hour {
                    return Ok((
                        Arc::clone(&cached.owners),
                        Arc::clone(&cached.controllers),
                        cached.date.clone(),
                    ));
                }
            }
        }
        // Any selection or snapshot-body load failure remains an error all the
        // way to the renderer; an empty political table is valid data only
        // when the decoded typed view itself is genuinely empty.
        let frame = self.selected_frame(hour)?;
        let owners = Arc::new(owners_from_leaves(frame.owner_by_state()));
        let controllers = Arc::new(owners_from_leaves(frame.control_by_state()));
        let date = frame.date().map(str::to_string);
        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some(CachedFrame {
                hour,
                owners: Arc::clone(&owners),
                controllers: Arc::clone(&controllers),
                date: date.clone(),
            });
        }
        Ok((owners, controllers, date))
    }
}

impl OwnerProvider for RecordingTimeline {
    fn hours(&self) -> &[i64] {
        self.timeline.hours()
    }

    fn set_scrub_interrupt(&self, interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {
        self.timeline.set_scrub_interrupt(interrupt);
    }

    fn owners_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(self.frame_maps(hour)?.0)
    }

    fn controllers_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(self.frame_maps(hour)?.1)
    }

    fn world_frame_at(&self, hour: i64) -> Result<Option<Arc<WorldFrame>>> {
        self.selected_frame(hour).map(Some)
    }

    fn hour_label(&self, hour: i64) -> Result<String> {
        // D6 replay-native clock: format the recording date with the FILE
        // topbar clock law (`interface/topbar.gui` DateText placeholder +
        // `core_l_english.yml` months) via game_ui; fall back to the raw
        // Clausewitz string when it does not parse (fail-closed, never a
        // fabricated date). Closes the former raw-date GAP note in app.rs.
        Ok(match self.frame_maps(hour)?.2 {
            Some(date) => match game_ui::binding::format_topbar_date_english(&date) {
                Some(formatted) => format!("{formatted} (hour {hour})"),
                None => format!("{date} (hour {hour})"),
            },
            None => format!("hour {hour}"),
        })
    }

    fn date_at(&self, hour: i64) -> Result<Option<String>> {
        // Validate the selection before exposing index metadata, then use the
        // authenticated date index directly. Building the transport date spine
        // must not decode every frame in a long recording.
        self.selected_canonical_hour(hour)?;
        Ok(self.timeline.date_at(hour)?)
    }

    fn pov_picker(&self) -> Result<crate::observer::PovPicker> {
        // Read the first frame's typed d18 roster. Multiplayer participation
        // and the recorder's local country do not constrain replay-owned panel
        // selection, and d94/d95 fog evidence is checked only if fog is
        // explicitly requested.
        //
        // R7 F2: a `selected_frame` failure here is the F4 fail-closed
        // authentication path (SHA/CRC/decode at the first hour); it
        // PROPAGATES rather than becoming `PovPicker::Unavailable`, which is
        // reserved for authenticated frames whose session carriers are
        // genuinely absent or unusable.
        let frame = self.selected_frame(self.timeline.first_hour())?;
        Ok(crate::observer::povs_from_frame(&frame))
    }

    fn anomaly_note(
        &self,
        hour: i64,
    ) -> std::result::Result<Option<String>, TimelineSelectionError> {
        self.anomaly_note_at(hour)
    }

    fn content_manifest(&self) -> Option<&replay_core::ContentManifest> {
        self.timeline.content_manifest()
    }


    fn decision_catalog(&self) -> Option<&replay_core::PackagedDecisionCatalog> {
        self.timeline.decision_catalog()
    }

    fn static_artifact(&self, name: &str) -> Option<&replay_core::PackagedStaticArtifact> {
        self.timeline.static_artifact(name)
    }

    fn admit_playback_content(
        &self,
        inventory: &replay_core::PlaybackContentInventory,
    ) -> std::result::Result<
        replay_core::ValidatedContentAdmission,
        replay_core::ContentValidationReport,
    > {
        self.timeline.admit_playback_content(inventory)
    }
}

/// Measured owners only; `Absent`/`Unobtained` stay off the map (sentinel).
pub fn owners_from_leaves(leaves: &BTreeMap<u32, Leaf<String>>) -> BTreeMap<u32, String> {
    leaves
        .iter()
        .filter_map(|(state_id, leaf)| leaf.measured().map(|tag| (*state_id, tag.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture_owners() -> BTreeMap<u32, String> {
        let mut owners = BTreeMap::new();
        for state_id in 1..=30u32 {
            let tag = if state_id % 2 == 0 { "GER" } else { "FRA" };
            owners.insert(state_id, tag.to_string());
        }
        owners
    }

    /// RES1 R2 — at the actual seam, a bundle/recording modset disagreement
    /// must still be classifiable: the boxed error has to downcast back to
    /// `SourceError::ModsetMismatch` (which `validate::rejection_kind` maps to
    /// `RejectionKind::Modset`). Flattening it to a string would silently
    /// demote the rejection class.
    #[test]
    fn seam_refusal_preserves_the_typed_modset_class() {
        use crate::bundle::tests::{recording_manifest, unique_temp_dir, write_fixture_bundle};

        let dir = unique_temp_dir("timeline_modset_seam");
        write_fixture_bundle(&dir);
        // The fixture bundle declares no `modset` block => the vanilla identity.
        let bundle = MapBundle::load(&dir).expect("fixture bundle loads");
        assert_eq!(bundle.provenance.mod_count, 0);

        // Vanilla bundle vs a recording that admitted two mod layers.
        let modded = recording_manifest(2);
        let error = reconcile_bundle_against_recording(Some(&bundle), Some(&modded))
            .expect_err("a class disagreement must refuse at the seam");
        let typed = error
            .downcast_ref::<replay_core::SourceError>()
            .expect("the seam must box the typed SourceError, not a String");
        assert!(
            matches!(typed, replay_core::SourceError::ModsetMismatch(_)),
            "expected ModsetMismatch, got {typed:?}"
        );

        // The agreeing case passes through, and no bundle is a no-op.
        let vanilla = recording_manifest(0);
        reconcile_bundle_against_recording(Some(&bundle), Some(&vanilla))
            .expect("matching vanilla identities must not refuse");
        reconcile_bundle_against_recording(None, Some(&modded))
            .expect("no bundle means no authority to cross-check");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn static_1936_has_single_hour_and_constant_owners() {
        let provider = Static1936::new(fixture_owners());
        assert_eq!(provider.hours(), &[0]);
        assert_eq!(*provider.owners_at(0).expect("owners"), fixture_owners());
        assert_eq!(
            provider.controllers_at(0).expect("controllers"),
            provider.owners_at(0).expect("owners")
        );
        assert!(provider.world_frame_at(0).expect("world frame").is_none());
        // Out-of-range hours still resolve to the single frame.
        assert_eq!(
            *provider.owners_at(9999).expect("constant owners"),
            fixture_owners()
        );
        assert_eq!(
            provider.controllers_at(9999).expect("constant controllers"),
            provider.owners_at(9999).expect("constant owners")
        );
    }

    #[test]
    fn dump_hour_selection_distinguishes_canonical_hours_from_ordinals() {
        let hours = [16_959_372, 16_959_373, 16_959_375];
        assert_eq!(
            resolve_dump_hour(&hours, DumpHourRequest::Canonical(16_959_373)),
            Ok(16_959_373)
        );
        assert_eq!(
            resolve_dump_hour(&hours, DumpHourRequest::Ordinal(2)),
            Ok(16_959_375)
        );
        assert!(matches!(
            resolve_dump_hour(&hours, DumpHourRequest::Canonical(71)),
            Err(TimelineSelectionError::BeforeFirst {
                requested: 71,
                first: 16_959_372,
                last: 16_959_375,
            })
        ));
        assert!(matches!(
            resolve_dump_hour(&hours, DumpHourRequest::Canonical(16_959_374)),
            Err(TimelineSelectionError::MissingCanonical { .. })
        ));
        assert!(matches!(
            resolve_dump_hour(&hours, DumpHourRequest::Ordinal(3)),
            Err(TimelineSelectionError::OrdinalOutOfRange { .. })
        ));
    }

    /// Synthetic three-hour source whose last frame fails to load: exercises
    /// the in-process provider seam the interactive scrub uses, so selection
    /// and load errors can never degrade to an empty (all-underlay) map.
    struct MidstreamLoadFailureSource {
        hours: Vec<i64>,
        warnings: Vec<String>,
    }

    impl MidstreamLoadFailureSource {
        fn new() -> MidstreamLoadFailureSource {
            MidstreamLoadFailureSource {
                hours: vec![0, 1, 2],
                warnings: Vec::new(),
            }
        }
    }

    impl replay_core::ReplaySource for MidstreamLoadFailureSource {
        fn game_id(&self) -> &str {
            "midstream-failure"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &self.warnings
        }

        fn frame(
            &self,
            hour_index: i64,
        ) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
            if hour_index == 2 {
                return Err(replay_core::SourceError::Open(
                    "synthetic mid-stream load failure".into(),
                ));
            }
            Ok(replay_core::RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: Default::default(),
            })
        }
    }

    struct CountingSource {
        hours: Vec<i64>,
        loads: Arc<AtomicUsize>,
    }

    impl replay_core::ReplaySource for CountingSource {
        fn game_id(&self) -> &str {
            "counting"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn date_at(
            &self,
            hour_index: i64,
        ) -> std::result::Result<Option<String>, replay_core::SourceError> {
            Ok(Some(format!("1936.1.{}", hour_index + 1)))
        }

        fn frame(
            &self,
            hour_index: i64,
        ) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(replay_core::RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: Default::default(),
            })
        }
    }

    #[test]
    fn anomaly_validation_is_lazy_ordered_and_does_not_scan_at_open() {
        let loads = Arc::new(AtomicUsize::new(0));
        let timeline = WorldTimeline::open(Box::new(CountingSource {
            hours: vec![10, 11, 12, 13],
            loads: Arc::clone(&loads),
        }))
        .expect("counting timeline");
        let baseline = loads.load(Ordering::SeqCst);
        let policy = AnomalyPolicy::with_known_provinces(timeline.hours(), None);
        let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));
        assert_eq!(
            loads.load(Ordering::SeqCst),
            baseline,
            "provider construction must not decode the campaign"
        );

        recording.world_frame_at(10).expect("first frame");
        let after_first = loads.load(Ordering::SeqCst);
        assert!(after_first > baseline);
        recording.world_frame_at(10).expect("repeat first frame");
        assert_eq!(
            loads.load(Ordering::SeqCst),
            after_first,
            "already-observed prefix must not be rescanned"
        );
        recording.world_frame_at(13).expect("last frame");
        assert!(
            loads.load(Ordering::SeqCst) >= after_first + 3,
            "forward seek must validate each previously unobserved hour in order"
        );
    }

    #[test]
    fn date_spine_reads_index_metadata_without_loading_frames() {
        let loads = Arc::new(AtomicUsize::new(0));
        let timeline = WorldTimeline::open(Box::new(CountingSource {
            hours: vec![10, 11, 12, 13],
            loads: Arc::clone(&loads),
        }))
        .expect("counting timeline");
        let baseline = loads.load(Ordering::SeqCst);
        let recording = RecordingTimeline::new_with_policy(timeline, None);

        let dates = recording
            .hours()
            .iter()
            .map(|hour| recording.date_at(*hour).expect("indexed date"))
            .collect::<Vec<_>>();
        assert_eq!(dates.first().and_then(Option::as_deref), Some("1936.1.11"));
        assert_eq!(dates.last().and_then(Option::as_deref), Some("1936.1.14"));
        assert_eq!(
            loads.load(Ordering::SeqCst),
            baseline,
            "enumerating the transport date spine must not decode frames"
        );
    }

    #[test]
    fn recording_provider_never_replaces_selection_or_load_errors_with_empty_maps() {
        let timeline = WorldTimeline::open(Box::new(MidstreamLoadFailureSource::new()))
            .expect("failing timeline");
        let policy = AnomalyPolicy::with_known_provinces(timeline.hours(), None);
        let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));

        let before = recording.owners_at(-1).expect_err("before-first must fail");
        assert!(
            before.to_string().contains(
                "requested canonical hour -1 is before the recording's valid canonical range 0..=2"
            ),
            "{before}"
        );
        let after = recording
            .controllers_at(3)
            .expect_err("after-last must fail");
        assert!(
            after.to_string().contains(
                "requested canonical hour 3 is after the recording's valid canonical range 0..=2"
            ),
            "{after}"
        );
        let load = recording
            .owners_at(2)
            .expect_err("snapshot load failure must fail");
        assert!(
            load.to_string().contains(
                "failed to load requested canonical hour 2 from the recording's valid canonical range 0..=2"
            ),
            "{load}"
        );
    }

    #[test]
    fn gap_unavailable_is_rejected_before_any_timeline_view_is_exposed() {
        let timeline = WorldTimeline::open(Box::new(CountingSource {
            hours: vec![10, 12],
            loads: Arc::new(AtomicUsize::new(0)),
        }))
        .expect("gapped timeline");
        let policy = AnomalyPolicy::with_known_provinces(timeline.hours(), None);
        let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));

        let assert_gap = |result: Result<()>| {
            let error = result.expect_err("hour inside a gap must fail closed");
            assert!(
                error
                    .to_string()
                    .contains("requested hour 11 is unavailable inside the recording gap 10->12"),
                "{error}"
            );
        };
        assert_gap(recording.owners_at(11).map(|_| ()));
        assert_gap(recording.controllers_at(11).map(|_| ()));
        assert_gap(recording.world_frame_at(11).map(|_| ()));
        assert_gap(recording.hour_label(11).map(|_| ()));
        assert_gap(recording.date_at(11).map(|_| ()));
        assert!(matches!(
            recording.anomaly_note(11),
            Err(TimelineSelectionError::GapUnavailable {
                requested: 11,
                from: 10,
                to: 12,
                ..
            })
        ));
        assert!(matches!(
            recording.seek_hour(11),
            Err(TimelineSelectionError::GapUnavailable { requested: 11, .. })
        ));
        assert!(matches!(
            recording.play_to_hour(11),
            Err(TimelineSelectionError::GapUnavailable { requested: 11, .. })
        ));
    }

    /// R7 F2 regression: a first-hour authentication/load failure must
    /// PROPAGATE out of `pov_picker` — never degrade into the semantic
    /// `PovPicker::Unavailable`, which is reserved for authenticated frames
    /// whose session carriers are genuinely absent.
    #[test]
    fn pov_picker_propagates_a_first_frame_authentication_failure() {
        struct FirstFrameFailureSource;
        impl replay_core::ReplaySource for FirstFrameFailureSource {
            fn game_id(&self) -> &str {
                "first-frame-failure"
            }
            fn hours(&self) -> &[i64] {
                &[0, 1, 2]
            }
            fn warnings(&self) -> &[String] {
                &[]
            }
            fn frame(
                &self,
                hour_index: i64,
            ) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
                Err(replay_core::SourceError::Integrity(format!(
                    "synthetic payload digest mismatch at hour {hour_index}"
                )))
            }
        }

        let timeline =
            WorldTimeline::open(Box::new(FirstFrameFailureSource)).expect("index-only open");
        let policy = AnomalyPolicy::with_known_provinces(timeline.hours(), None);
        let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));
        let error = recording
            .pov_picker()
            .expect_err("an authentication failure must not become PovPicker::Unavailable");
        assert!(
            error.to_string().contains("payload digest mismatch"),
            "the propagated error names the underlying integrity failure: {error}"
        );
    }

    /// The companion direction for R7 F2: an AUTHENTICATED frame with no
    /// session carriers stays a semantic `Ok(Unavailable)` — propagation is
    /// only for failures.
    #[test]
    fn pov_picker_stays_semantic_unavailable_for_an_authenticated_sessionless_frame() {
        let timeline = WorldTimeline::open(Box::new(CountingSource {
            hours: vec![0, 1],
            loads: Arc::new(AtomicUsize::new(0)),
        }))
        .expect("timeline");
        let recording = RecordingTimeline::new_with_policy(timeline, None);
        match recording.pov_picker().expect("authenticated frame") {
            crate::observer::PovPicker::Unavailable(reason) => {
                assert!(!reason.is_empty(), "carries the fail-closed reason");
            }
            crate::observer::PovPicker::Available(choices) => {
                panic!("sessionless frame cannot offer POVs: {choices:?}")
            }
        }
    }

    /// R7 F6 regression: formatting `OrdinalOutOfRange` with `count == 0`
    /// must not underflow-panic in the `pub` `Display` impl, even though no
    /// current constructor produces that variant.
    #[test]
    fn ordinal_out_of_range_display_does_not_panic_on_zero_count() {
        let zero = TimelineSelectionError::OrdinalOutOfRange {
            requested: 3,
            count: 0,
            first: 0,
            last: 0,
        };
        let text = zero.to_string();
        assert!(
            text.contains("has no dump snapshots"),
            "zero-count message is honest, not an underflowed range: {text}"
        );
        let normal = TimelineSelectionError::OrdinalOutOfRange {
            requested: 3,
            count: 2,
            first: 10,
            last: 11,
        };
        assert!(normal.to_string().contains("0..=1"), "{normal}");
    }

    #[test]
    fn anomaly_note_validates_the_frame_before_reporting_no_hold() {
        let timeline = WorldTimeline::open(Box::new(MidstreamLoadFailureSource::new()))
            .expect("failing timeline");
        let policy = AnomalyPolicy::with_known_provinces(timeline.hours(), None);
        let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));

        let error = recording
            .anomaly_note(2)
            .expect_err("anomaly note must not hide validation failure as no anomaly");
        assert!(matches!(
            error,
            TimelineSelectionError::FrameLoad { requested: 2, .. }
        ));
    }

    #[test]
    fn owners_from_leaves_keeps_only_measured() {
        let mut leaves: BTreeMap<u32, Leaf<String>> = BTreeMap::new();
        leaves.insert(1, Leaf::Measured("GER".to_string()));
        leaves.insert(2, Leaf::Unobtained);
        leaves.insert(3, Leaf::Absent);
        let owners = owners_from_leaves(&leaves);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners.get(&1).map(String::as_str), Some("GER"));
        // Unobtained/Absent must NOT appear (they render as the underlay
        // sentinel, never as a fabricated owner).
        assert!(!owners.contains_key(&2));
        assert!(!owners.contains_key(&3));
    }

    fn fixture_path(rel: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
            .join(rel)
    }

    #[test]
    fn seek_equals_play_to_on_every_recorded_hour() {
        // E1/A4: the replay_core seek/play_to hooks resolve identically, and
        // date_at surfaces the recorded date for the transport spine.
        let path = fixture_path("positive/fx_owner_controller_flip.battleplan");
        let source = replay_core::open_recording(&path).expect("open fixture");
        let timeline = replay_core::WorldTimeline::open(source).expect("timeline");
        let recording = RecordingTimeline::new_with_policy(timeline, None);
        for &hour in recording.hours() {
            assert_eq!(
                recording.seek_hour(hour),
                recording.play_to_hour(hour),
                "seek != play_to at hour {hour}"
            );
            assert_eq!(recording.seek_hour(hour), Ok(hour), "exact-hour resolves");
            // Every recorded frame in this fixture carries a Clausewitz date.
            assert!(
                recording.date_at(hour).expect("date query").is_some(),
                "date at hour {hour}"
            );
            assert_eq!(
                recording
                    .world_frame_at(hour)
                    .expect("world frame query")
                    .map(|frame| frame.hour_index()),
                Some(hour),
                "typed WorldFrame is exposed at the exact recorded hour"
            );
        }
    }

    /// Scrubbing the recorded timeline actually changes recorded state.
    ///
    /// This is the surviving half of the retired `replay_qa`
    /// "recording ingest + scrub" gate. That gate asserted the same property by
    /// dumping two hours to BMP and byte-diffing them, which is no longer
    /// reachable: a production recording invocation now refuses `--bundle`,
    /// `--capture`, `--dump-zoom` and `--dump-center`, and the bundle-less
    /// production dump path needs carriers the synthetic fixture never had plus a
    /// machine-local the game install. The property itself is not about pixels, so it
    /// is asserted here on the typed maps the renderer keys off, through the
    /// product's own scrub provider -- no bundle, no GPU, no the game install.
    ///
    /// Nothing else covered this: `seek_equals_play_to_on_every_recorded_hour`
    /// above uses the same fixture but asserts seek/play_to equivalence, and the
    /// `e2e_playback` gate drives `fx_pov_visibility` / `fx_topbar_scalars_sweep`
    /// for POV and topbar bindings. Ownership divergence across the scrub was
    /// left unasserted in this crate when the harness gate died.
    #[test]
    fn scrubbing_the_recorded_timeline_changes_owner_and_controller_state() {
        let path = fixture_path("positive/fx_owner_controller_flip.battleplan");
        let source = replay_core::open_recording(&path).expect("open fixture");
        let timeline = replay_core::WorldTimeline::open(source).expect("timeline");
        let recording = RecordingTimeline::new_with_policy(timeline, None);

        let hours: Vec<i64> = recording.hours().to_vec();
        assert!(
            hours.len() > 1,
            "a single-hour recording cannot demonstrate a scrub"
        );
        let first = hours[0];
        let last = *hours.last().expect("last recorded hour");

        let first_owners = recording.owners_at(first).expect("owners at first hour");
        let last_owners = recording.owners_at(last).expect("owners at last hour");
        let first_controllers = recording
            .controllers_at(first)
            .expect("controllers at first hour");
        let last_controllers = recording
            .controllers_at(last)
            .expect("controllers at last hour");

        // Non-empty, or "they differ" could be satisfied by one side being empty
        // because the frame failed to decode.
        assert!(
            !first_owners.is_empty() && !last_owners.is_empty(),
            "owner map is empty at an endpoint: {} then {}",
            first_owners.len(),
            last_owners.len()
        );
        assert!(
            !first_controllers.is_empty() && !last_controllers.is_empty(),
            "controller map is empty at an endpoint: {} then {}",
            first_controllers.len(),
            last_controllers.len()
        );

        assert_ne!(
            first_owners, last_owners,
            "owners are identical at hour {first} and hour {last}; the recorded \
             ownership scrub is a no-op"
        );
        assert_ne!(
            first_controllers, last_controllers,
            "controllers are identical at hour {first} and hour {last}; the \
             recorded control scrub is a no-op"
        );

        // Divergence must be a CHANGED assignment on a state present in BOTH
        // hours, not merely states appearing in or disappearing from the map.
        // Asserted for owners and controllers alike: "non-empty and unequal" on
        // its own would be satisfied by a map that only gained or lost entries.
        let reassigned = |before: &BTreeMap<u32, String>, after: &BTreeMap<u32, String>| {
            before
                .iter()
                .filter(|(state, tag)| after.get(state).is_some_and(|later| later != *tag))
                .count()
        };
        assert!(
            reassigned(&first_owners, &last_owners) > 0,
            "no state kept its identity while changing OWNER between hour {first} \
             and hour {last}"
        );
        assert!(
            reassigned(&first_controllers, &last_controllers) > 0,
            "no state kept its identity while changing CONTROLLER between hour \
             {first} and hour {last}"
        );

        // Re-reading is stable, so the differences above are recorded state and
        // not decode nondeterminism. BOTH maps at BOTH endpoints: the two
        // endpoints are separate cache loads, so checking only the first hour
        // would still admit an alternating or hour-specific last-frame decode
        // that satisfied each reassignment assertion on a different read.
        for (hour, expected_owners, expected_controllers) in [
            (first, &first_owners, &first_controllers),
            (last, &last_owners, &last_controllers),
        ] {
            assert_eq!(
                &recording.owners_at(hour).expect("owners re-read"),
                expected_owners,
                "re-reading hour {hour} owners is not stable"
            );
            assert_eq!(
                &recording.controllers_at(hour).expect("controllers re-read"),
                expected_controllers,
                "re-reading hour {hour} controllers is not stable"
            );
        }
    }
}
