//! Native ship shape/projection supplied to the shared prepared parts-race flow.
use super::*;
use crate::products::parts_race_prepared as shared;
use replay::{
    format::FormatVersion,
    frame::{DecodedFrame, Frame},
    rows::TableId,
};
use serde_json::Value;

pub(crate) struct Navy;
pub(crate) type Prepared = shared::Prepared<Navy>;
pub(crate) const QUERY: shared::Query = shared::query::<Navy>;
pub(crate) use QUERY as query;

impl shared::Family for Navy {
    type Scan = scan::Scan;
    type Raw = RawNavyRun;
    type Scanned = ScannedNavy;
    type Part = NavyPart;
    type Reply = NavyReply;
    type PointContent = (
        Option<Arc<Catalogue>>,
        Option<Arc<installed_content::unit_leader::SubUnits>>,
        Vec<String>,
        Option<Arc<PartResults>>,
    );
    const POINT_PARTS: bool = true;
    const NAME: &'static str = "navy";
    const GROUPS: usize = 2;
    const MIXES: usize = 4;
    const REFERENCES: usize = 2;
    const FIELDS: &'static [&'static str] = &["measure", "ownership", "grouping"];
    fn projectable(raw: &Self::Raw) -> bool {
        raw.displayable()
    }
    const TABLES: &'static [TableId] = &[TableId::Ships, TableId::ShipModels];
    const SAMPLE_TABLES: &'static [TableId] = &[
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Ships,
        TableId::TaskForces,
        TableId::ShipModels,
        TableId::LexicalTokens,
    ];
    #[cfg(test)]
    fn work(scan: &Self::Scan) -> crate::prepared::observer_tests::Work {
        scan.work
    }
    fn holders(scan: &Self::Scan) -> &[(Vec<u8>, Option<(u32, u32)>)] {
        &scan.countries.tags
    }
    fn point_content(source: &Source, context: &LoadContext) -> Self::PointContent {
        let mut warnings = Vec::new();
        match context
            .content()
            .and_then(|options| catalogue::catalogue(source, options))
        {
            Ok(catalogue) => {
                warnings.extend(catalogue.verification.iter().cloned());
                warnings.extend(catalogue.warnings.iter().cloned());
                let units = content_units(&catalogue, &mut warnings);
                (
                    Some(Arc::clone(&catalogue)),
                    units,
                    warnings,
                    Some(part_content(source, Some(&catalogue))),
                )
            }
            Err(error) => (
                None,
                None,
                vec![format!("Ship class names are unavailable: {error}")],
                Some(part_content(source, None)),
            ),
        }
    }
    fn point_reply(
        scan: &Self::Scan,
        observed: &BTreeMap<String, Self::Raw>,
        selection: &shared::Selection,
        content: &Self::PointContent,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<Self::Reply>> {
        let mut scanned = Self::projection_scan(scan);
        for (tag, run) in observed {
            Self::insert(
                &mut scanned,
                selection.channel,
                tag.clone(),
                vec![run.clone()],
                Vec::new(),
            );
        }
        let mut params = parameters(selection);
        params.tags = observed.keys().cloned().collect();
        params.top = None;
        reply_cached(
            &scanned,
            &params,
            content.1.as_deref(),
            |key| {
                let catalogue = content.0.as_ref()?;
                catalogue.localisation.get(key)?;
                catalogue.localised(key, catalogue::View::Plain)
            },
            content.2.clone(),
            cancelled,
            content.3.as_deref(),
        )
        .map(Some)
    }
    fn configure(source: &Source, context: &LoadContext) {
        super::configure(source, context);
    }
    fn applied(scan: &Self::Scan) -> Option<i32> {
        scan.applied
    }
    fn changed(scan: &Self::Scan, index: usize, _hour: i32) -> BTreeMap<String, Self::Raw> {
        scan.observed
            .iter()
            .filter_map(|tag| {
                let track = &scan.tracks[tag];
                let runs = if index.is_multiple_of(2) {
                    &track.runs[index / 2]
                } else {
                    &track.strength_runs[index / 2]
                };
                runs.last()
                    .filter(|run| run.first_hour == _hour)
                    .cloned()
                    .map(|run| (tag.clone(), run))
            })
            .collect()
    }
    fn removed(scan: &Self::Scan) -> &[String] {
        &scan.removed
    }
    fn projection_scan(scan: &Self::Scan) -> Self::Scanned {
        ScannedNavy {
            classes: Arc::clone(&scan.result.classes),
            ..Default::default()
        }
    }
    fn finish_hour(scan: &mut Self::Scan) {
        scan.seal();
    }
    fn step(
        scan: &mut Self::Scan,
        raw: &Frame,
        frame: &DecodedFrame,
        _format: FormatVersion,
        cancelled: &AtomicBool,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        scan.step_changes(raw, frame, cancelled, changes)
    }
    fn resume(
        scan: &mut Self::Scan,
        raw: &Frame,
        frame: &DecodedFrame,
        _format: FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        scan.rejoin(raw, frame, cancelled)
    }
    fn observed(scan: &Self::Scan, index: usize, hour: i32) -> BTreeMap<String, Self::Raw> {
        scan.countries
            .tags
            .iter()
            .filter_map(|(tag, identity)| {
                identity.as_ref()?;
                let tag = std::str::from_utf8(tag).ok()?;
                let track = scan.tracks.get(tag)?;
                let runs = if index.is_multiple_of(2) {
                    &track.runs[index / 2]
                } else {
                    &track.strength_runs[index / 2]
                };
                runs.last().cloned().map(|mut run| {
                    run.last_hour = hour;
                    (tag.to_owned(), run)
                })
            })
            .collect()
    }

    fn runs(scan: &Self::Scan, index: usize) -> BTreeMap<String, &[Self::Raw]> {
        scan.tracks
            .iter()
            .map(|(tag, track)| {
                (
                    tag.clone(),
                    if index.is_multiple_of(2) {
                        track.runs[index / 2].as_slice()
                    } else {
                        track.strength_runs[index / 2].as_slice()
                    },
                )
            })
            .collect()
    }
    fn prune(scan: &mut Self::Scan) {
        for track in scan.tracks.values_mut() {
            for runs in track.runs.iter_mut().chain(&mut track.strength_runs) {
                shared::retain_tail(runs);
            }
            shared::retain_tail(&mut track.segments);
        }
    }
    fn metadata(scan: &Self::Scan) -> Self::Scanned {
        scan.result.clone()
    }
    fn bounds(raw: &Self::Raw) -> (i32, i32, bool) {
        (raw.first_hour, raw.last_hour, raw.starts_segment)
    }
    fn window(raw: &mut Self::Raw, first: i32, last: i32, starts: bool) {
        raw.first_hour = first;
        raw.last_hour = last;
        raw.starts_segment = starts;
    }
    fn total(raw: &Self::Raw, _index: usize) -> Option<i64> {
        raw.total
    }
    fn transient(
        run: crate::prepared::series::Run,
        _index: usize,
        maximum: Option<i128>,
    ) -> io::Result<Self::Raw> {
        Ok(RawNavyRun {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            starts_segment: run.break_before,
            total: Some(
                i64::try_from(run.value)
                    .map_err(|_| crate::invalid("Navy sample exceeds native integer range"))?,
            ),
            classes: Arc::new(BTreeMap::new()),
            reference_total: maximum
                .map(i64::try_from)
                .transpose()
                .map_err(|_| crate::invalid("Navy maximum sample exceeds native integer range"))?,
        })
    }
    fn maximum(raw: &Self::Raw) -> Option<i64> {
        raw.reference_total
    }
    fn reference(index: usize) -> Option<usize> {
        (index % 2 == 1).then_some(index / 2)
    }
    fn part_key(part: &Self::Part) -> &str {
        &part.key
    }
    fn selection(params: Value) -> io::Result<shared::Selection> {
        let params: NavyParams = serde_json::from_value(params)?;
        NavySize::validate(&params).map_err(crate::invalid)?;
        Ok(shared::Selection {
            channel: params.ownership.index() * 2
                + usize::from(params.measure == NavyMeasure::Strength),
            group: usize::from(params.grouping == NavyGrouping::Type),
            branch_id: params.branch_id,
            tags: params.tags,
            top: params.top,
        })
    }
    fn insert(
        scanned: &mut Self::Scanned,
        index: usize,
        tag: String,
        runs: Vec<Self::Raw>,
        segments: Vec<HourSpan>,
    ) {
        if index.is_multiple_of(2) {
            scanned.series[index / 2].insert(tag.clone(), runs);
        } else {
            scanned.strength_series[index / 2].insert(tag.clone(), runs);
        }
        scanned.segments.insert(tag, segments);
    }
    fn render(
        source: &Source,
        selection: &shared::Selection,
        context: &LoadContext,
        cancelled: &AtomicBool,
        scanned: &Self::Scanned,
        history: Option<&country_history::RecordedHistories>,
        membership: Option<BTreeMap<String, Vec<HourSpan>>>,
    ) -> io::Result<Self::Reply> {
        let params = parameters(selection);
        present(
            source, &params, context, cancelled, scanned, history, membership,
        )
    }
    fn projection(reply: Self::Reply) -> (Vec<Self::Part>, BTreeMap<String, Vec<Option<i64>>>) {
        (
            reply.parts,
            reply
                .series
                .into_iter()
                .map(|country| {
                    (
                        country.tag,
                        country
                            .runs
                            .last()
                            .expect("native census projection")
                            .values
                            .clone(),
                    )
                })
                .collect(),
        )
    }
}

fn parameters(selection: &shared::Selection) -> NavyParams {
    NavyParams {
        branch_id: selection.branch_id.clone(),
        tags: selection.tags.clone(),
        top: selection.top,
        measure: if selection.channel.is_multiple_of(2) {
            NavyMeasure::Ships
        } else {
            NavyMeasure::Strength
        },
        ownership: if selection.channel < 2 {
            NavyOwnership::Roster
        } else {
            NavyOwnership::Logical
        },
        grouping: if selection.group == 0 {
            NavyGrouping::Class
        } else {
            NavyGrouping::Type
        },
    }
}
