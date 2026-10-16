//! Native air shape/projection supplied to the shared prepared parts-race flow.
use super::*;
use crate::products::parts_race_prepared as shared;
use replay::{
    format::FormatVersion,
    frame::{DecodedFrame, Frame},
    rows::TableId,
};
use serde_json::Value;

pub struct Air;
pub type Prepared = shared::Prepared<Air>;
pub const QUERY: shared::Query = shared::query::<Air>;
pub use QUERY as query;

fn parameters(
    branch_id: String,
    tags: Vec<String>,
    top: Option<u32>,
    index: usize,
) -> AirForceParams {
    AirForceParams {
        branch_id,
        tags,
        top,
        measure: if index.is_multiple_of(2) {
            AirForceMeasure::Planes
        } else {
            AirForceMeasure::Wings
        },
        ownership: if index < 2 {
            AirForceOwnership::Roster
        } else {
            AirForceOwnership::Effective
        },
    }
}
impl shared::Family for Air {
    type Scan = scan::Scan;
    type Raw = RawRun;
    type Scanned = ScannedAir;
    type Part = AirForcePart;
    type Reply = AirForceReply;
    type PointContent = ();
    const POINT_PARTS: bool = false;
    const NAME: &'static str = "air";
    const PRODUCT: &'static str = "air-force-size";
    const GROUPS: usize = 1;
    const MIXES: usize = 1;
    const REFERENCES: usize = 0;
    const FIELDS: &'static [&'static str] = &["measure", "ownership"];
    const TABLES: &'static [TableId] = &[TableId::AirWings, TableId::WingEquipment];
    const SAMPLE_TABLES: &'static [TableId] = &[
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::AirWings,
        TableId::WingEquipment,
        TableId::VariantDefinitions,
        TableId::LexicalTokens,
        TableId::CountryAppearance,
        TableId::VariantModules,
        TableId::VariantUpgrades,
    ];
    #[cfg(test)]
    fn work(scan: &Self::Scan) -> dashboard_base::prepared::observer_tests::Work {
        dashboard_base::prepared::observer_tests::Work {
            history_visits: scan.histories.projected,
            ..scan.work
        }
    }
    fn holders(scan: &Self::Scan) -> &[(Vec<u8>, Option<(u32, u32)>)] {
        &scan.countries.tags
    }
    fn point_content(_source: &Source, _context: &LoadContext) {}
    fn point_reply(
        _scan: &Self::Scan,
        _observed: &BTreeMap<String, Self::Raw>,
        _selection: &shared::Selection,
        _content: &Self::PointContent,
        _cancelled: &AtomicBool,
    ) -> io::Result<Option<Self::Reply>> {
        // Recorded variant spellings are a dated catalogue retained by the native scan.
        Ok(None)
    }
    fn configure(source: &Source, context: &LoadContext) {
        super::configure(source, context);
    }
    fn applied(scan: &Self::Scan) -> Option<i32> {
        scan.applied
    }
    fn changed(scan: &Self::Scan, _index: usize, _hour: i32) -> BTreeMap<String, Self::Raw> {
        scan.histories
            .observed
            .iter()
            .filter_map(|tag| {
                let track = scan.histories.tags[tag];
                scan.histories.tracks[track]
                    .runs
                    .last()
                    .cloned()
                    .map(|run| (tag.clone(), run))
            })
            .collect()
    }
    fn removed(scan: &Self::Scan) -> &[String] {
        &scan.histories.removed
    }
    fn projection_scan(_scan: &Self::Scan) -> Self::Scanned {
        ScannedAir::default()
    }
    fn step(
        scan: &mut Self::Scan,
        raw: &Frame,
        frame: &DecodedFrame,
        format: FormatVersion,
        cancelled: &AtomicBool,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        scan.step_changes(raw, frame, format, cancelled, changes)
    }
    fn resume(
        scan: &mut Self::Scan,
        raw: &Frame,
        frame: &DecodedFrame,
        format: FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        scan.rejoin(raw, frame, format, cancelled)
    }
    fn observed(scan: &Self::Scan, _index: usize, hour: i32) -> BTreeMap<String, Self::Raw> {
        if scan.histories.through != Some(hour) {
            return BTreeMap::new();
        }
        scan.histories
            .active
            .iter()
            .zip(&scan.histories.active_tags)
            .filter_map(|(&(index, _), tag)| {
                scan.histories.tracks[index]
                    .runs
                    .last()
                    .cloned()
                    .map(|mut run| {
                        run.last_hour = hour;
                        (tag.clone(), run)
                    })
            })
            .collect()
    }

    fn runs(scan: &Self::Scan, _index: usize) -> BTreeMap<String, &[Self::Raw]> {
        scan.histories
            .tags
            .iter()
            .map(|(tag, &index)| (tag.clone(), scan.histories.tracks[index].runs.as_slice()))
            .collect()
    }
    fn finish_hour(scan: &mut Self::Scan) {
        let through = scan.histories.through;
        scan.histories.flush();
        scan.histories.through = through;
    }
    fn prune(scan: &mut Self::Scan) {
        for track in &mut scan.histories.tracks {
            shared::retain_tail(&mut track.runs);
            shared::retain_tail(&mut track.segments);
        }
        scan.published();
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
    fn total(raw: &Self::Raw, index: usize) -> Option<i64> {
        raw.total(&parameters(String::new(), Vec::new(), None, index))
    }
    fn transient(
        run: dashboard_base::prepared::series::Run,
        index: usize,
        _maximum: Option<i128>,
    ) -> io::Result<Self::Raw> {
        let total = i64::try_from(run.value).map_err(|_| {
            dashboard_core::invalid("Air force sample exceeds native integer range")
        })?;
        let counts = Arc::new(Counts {
            planes: if index.is_multiple_of(2) { total } else { 0 },
            wings: if index.is_multiple_of(2) { 0 } else { total },
            planes_by_token: BTreeMap::new(),
            wings_by_tokens: BTreeMap::new(),
        });
        Ok(RawRun {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            starts_segment: run.break_before,
            roster: (index < 2).then(|| Arc::clone(&counts)),
            effective: (index >= 2).then_some(counts),
        })
    }
    type Tail = scan::Tail;
    fn tail(scan: &Self::Scan, hour: i32) -> Self::Tail {
        scan.tail(hour)
    }
    /// Air names its equipment by token: nothing is renumbered.
    type Numbering = ();
    fn numbering(_: &Self::Scan, _: &Self::Scan) -> Self::Numbering {}
    fn renumber(_: &Self::Numbering, _: &mut Self::Raw) {}
    fn renumber_base(_: &Self::Numbering, _: &mut Self::Scanned) {}
    fn continues(
        carried: &Self::Scan,
        tail: &Self::Tail,
        block: &Self::Scan,
        _: &Self::Numbering,
    ) -> bool {
        carried.continues(tail, block)
    }
    fn carry(carried: &mut Self::Scan, tail: Self::Tail, block: Self::Scan, _: Self::Numbering) {
        carried.carry(tail, block);
    }
    fn part_key(part: &Self::Part) -> &str {
        &part.key
    }
    fn selection(params: Value) -> io::Result<shared::Selection> {
        let params: AirForceParams = serde_json::from_value(params)?;
        AirForceSize::validate(&params).map_err(dashboard_core::invalid)?;
        Ok(shared::Selection {
            channel: params.ownership.index() * 2
                + usize::from(params.measure == AirForceMeasure::Wings),
            group: 0,
            branch_id: params.branch_id,
            tags: params.tags,
            top: params.top,
        })
    }
    fn insert(
        scanned: &mut Self::Scanned,
        _index: usize,
        tag: String,
        runs: Vec<Self::Raw>,
        segments: Vec<HourSpan>,
    ) {
        scanned.series.insert(tag.clone(), runs);
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
        let params = parameters(
            selection.branch_id.clone(),
            selection.tags.clone(),
            selection.top,
            selection.channel,
        );
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
