#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct EffectExtra {
    pub child_sources: u32,
    pub custom_tooltips: u32,
}
