//! The proof tool's line for the template roster as a panel (`--rows roster@<TAG>`): the panel's data
//! over rows at the head against the reply the request path answers for the same hour, and what
//! evaluating it costs where the follow host evaluates it (each new frame of its rows, the data
//! that differs pushed).

use super::{Cadence, RowReader, Rows};
use dashboard_follow::panel::{Panel, PanelData, message};
use installed_content_core::ResolveOptions;
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{
    io,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

pub(crate) struct RosterLine {
    tag: String,
    branch: u64,
    source: dashboard::Source,
    context: dashboard::config::LoadContext,
    panel: Box<dyn Panel>,
    pub(super) reader: RowReader,
    last: Option<Arc<DecodedFrame>>,
    data: Option<PanelData>,
    /// Each evaluation's nanoseconds, and the data that differed with their message bytes.
    evaluations: Vec<u64>,
    pushed: Vec<usize>,
}

impl RosterLine {
    /// The roster panel of `tag` on `rows`, its sections in the reader's language of `content`.
    pub(crate) fn open(rows: &Arc<Rows>, path: &Path, branch: u64, tag: &str, content: &ResolveOptions, cadence: Cadence) -> io::Result<Self> {
        let cancelled = AtomicBool::new(false);
        let source = dashboard::Source::open(
            dashboard::SourceKey { source_id: "roster-proof".into(), revision: dashboard::protocol::replay_revision(path)? },
            path.to_path_buf(),
            &cancelled,
        )?;
        let context = dashboard::config::LoadContext {
            content: Some(dashboard::config::ContentOptions { options: content.clone(), allow_unverified: false, presentation: Default::default() }),
        };
        let panel = dashboard::answer::panel(&source, &context, &json!({ "panel": "template-roster", "branchId": branch.to_string(), "tag": tag }))
            .ok_or_else(|| io::Error::other("The service shows no template roster panel"))?
            .map_err(io::Error::other)?;
        let reader = rows.reader(panel.tables(), cadence);
        Ok(Self { tag: tag.to_owned(), branch, source, context, panel, reader, last: None, data: None, evaluations: Vec::new(), pushed: Vec::new() })
    }

    /// What the follow host does each frame: a new frame of rows evaluated, its data pushed when it differs.
    pub(crate) fn poll(&mut self) {
        let Some(frame) = self.reader.frame() else { return };
        if self.last.as_ref().is_some_and(|last| Arc::ptr_eq(last, &frame)) {
            return;
        }
        let started = Instant::now();
        let data = self.panel.data(&frame);
        self.evaluations.push(started.elapsed().as_nanos() as u64);
        if self.data.as_ref() != Some(&data) {
            self.pushed.push(message(&[(1, frame.hour, &data)]).len());
            self.data = Some(data);
        }
        self.last = Some(frame);
    }

    /// The panel's data over `frame` against the request path's reply at `hour`: (entries, differing, the first difference).
    pub(crate) fn compare(&mut self, frame: &DecodedFrame, hour: replay_data::Hour) -> (u64, u64, Option<String>) {
        let ours = self.panel.data(frame);
        let params = json!({ "branchId": self.branch.to_string(), "tag": self.tag, "hour": hour });
        let asked = dashboard::answer::answer(&self.source, "template-roster", &params, &self.context, &AtomicBool::new(false)).map_err(|error| error.to_string());
        let texts = |data: &PanelData, name: &str| data.as_ref().ok().and_then(|reply| reply.get(name)).and_then(Value::as_array).map(|list| list.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
        let entries = (texts(&asked, "sections").len() + texts(&asked, "warnings").len()).max(1) as u64;
        if ours == asked {
            return (entries, 0, None);
        }
        let (our_sections, asked_sections) = (texts(&ours, "sections"), texts(&asked, "sections"));
        let first = if ours.as_ref().err() != asked.as_ref().err() {
            format!("refused: ours {:?}, asked {:?}", ours.as_ref().err(), asked.as_ref().err())
        } else if let Some(at) = our_sections.iter().zip(&asked_sections).position(|(a, b)| a != b) {
            format!("section {at}: ours {:?}, asked {:?}", our_sections[at], asked_sections[at])
        } else {
            format!("sections {} and {}, warnings {} and {}", our_sections.len(), asked_sections.len(), texts(&ours, "warnings").len(), texts(&asked, "warnings").len())
        };
        (entries, 1, Some(first))
    }

    pub(crate) fn summary(&self, distribution: fn(Vec<f64>) -> Value) -> Value {
        json!({
            "tag": self.tag,
            "evaluations": self.evaluations.len(),
            "evaluation_ms": distribution(self.evaluations.iter().map(|&ns| ns as f64 / 1e6).collect()),
            "pushes": self.pushed.len(),
            "push_bytes": distribution(self.pushed.iter().map(|&bytes| bytes as f64).collect()),
        })
    }
}
