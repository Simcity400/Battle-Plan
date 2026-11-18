/// Native effect-block nonempty gate. Numeric child contributions are already
/// included in the root's aggregate entries; this hover prints no child tree.
/// Custom tooltips are a separate collection, whose keys are not retained here.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct EffectExtra {
    pub child_sources: u32,
    pub custom_tooltips: u32,
}
