use super::*;
use engine_allocation::reserve;

#[derive(Default)]
pub(super) struct NativeShadow<R> {
    pub(super) rows: Vec<R>,
    pub(super) bytes: Vec<u8>,
    required: usize,
    pub(super) output: Weak<TableRows>,
}

impl<R> NativeShadow<R> {
    fn clear(&mut self) {
        self.rows.clear();
        self.bytes.clear();
        self.required = 0;
        self.output = Weak::new();
    }
}

#[derive(Default)]
pub struct CharacterShadow {
    branch: Option<BranchId>,
    pub(super) training: Option<((u64, u64), Weak<TableRows>)>,
    pub(super) characters: NativeShadow<ccharacter::Row>,
    pub(super) leaders: NativeShadow<ccountry_leader::Row>,
    pub(super) advisors: NativeShadow<cadvisor::Row>,
    pub(super) traits: NativeShadow<cunit_leader::TraitRow>,
}

pub struct CharacterConversion<'a> {
    pub(super) shadow: &'a mut CharacterShadow,
    pub(super) cached: [Option<&'a Arc<TableRows>>; 4],
}

impl<'a> CharacterConversion<'a> {
    pub fn new(
        branch: BranchId,
        shadow: &'a mut CharacterShadow,
        cached: [Option<&'a Arc<TableRows>>; 4],
    ) -> Self {
        if shadow.branch != Some(branch) {
            shadow.characters.clear();
            shadow.leaders.clear();
            shadow.advisors.clear();
            shadow.traits.clear();
            shadow.training = None;
            shadow.branch = Some(branch);
        }
        Self { shadow, cached }
    }
}

pub(super) fn shadow_input<R: Copy + PartialEq>(
    table: TableId,
    buffer: &Buffer<R>,
    shadow: &mut NativeShadow<R>,
    cached: Option<&Arc<TableRows>>,
    valid_row: impl Fn(&R, &[u8]) -> bool,
    convert: impl FnOnce() -> TableView,
) -> TableInput {
    let source = match buffer.rows.outcome {
        Outcome::Rows { len } => buffer.rows.rows.get(..len),
        _ => None,
    };
    let source = source.zip(buffer.bytes.first.get(..buffer.bytes.len));
    if let Some((rows, bytes)) = source
        && let Some(cached) = cached
        && cached.table() == table
        && let Some(previous) = shadow.output.upgrade()
        && Arc::ptr_eq(cached, &previous)
        && shadow.rows == rows
        && shadow.required == buffer.bytes.required
        && shadow.bytes == bytes
    {
        return TableInput::SharedComplete(Arc::clone(cached));
    }
    let skipped = matches!(
        buffer.rows.outcome,
        Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON
        }
    );
    let source = source.filter(|(rows, bytes)| rows.iter().all(|row| valid_row(row, bytes)));
    match (source, convert()) {
        (Some((rows, bytes)), TableView::Complete(output)) => {
            let output = Arc::new(output);
            shadow.rows.clear();
            shadow.bytes.clear();
            if reserve(&mut shadow.rows, rows.len()).is_err()
                || reserve(&mut shadow.bytes, bytes.len()).is_err()
            {
                return unavailable("collector allocation failed").into();
            }
            shadow.rows.extend_from_slice(rows);
            shadow.bytes.extend_from_slice(bytes);
            shadow.required = buffer.bytes.required;
            shadow.output = Arc::downgrade(&output);
            TableInput::SharedComplete(output)
        }
        (_, view) => {
            if !skipped {
                shadow.clear();
            }
            TableInput::Sampled(view)
        }
    }
}
