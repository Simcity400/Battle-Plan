/// Attribution of the pinned native caller, independently of source kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Cause {
    #[default]
    Unknown = 0,
    AirMission = 1,
    AirNaval = 2,
    LandCombat = 3,
    Scripted = 4,
    Weekly = 5,
    ProgramTeardown = 6,
    AbortDismantle = 7,
}
