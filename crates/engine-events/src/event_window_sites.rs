use crate::script_event_sites::Site;
use engine_sites::Rva;

pub const PAST_AI: Site = Site {
    name: "EventWindowPastAi",
    rva: Rva(0x00a5_b3c4),
};
pub const PAST_VIEWER: Site = Site {
    name: "EventWindowPastViewer",
    rva: Rva(0x00a5_b3ee),
};
pub const SENDER_TEST: Site = Site {
    name: "EventWindowSenderTest",
    rva: Rva(0x00a5_b40c),
};
pub const PLAYER_RESTRICTED: Site = Site {
    name: "EventWindowPlayerRestricted",
    rva: Rva(0x00a5_b523),
};
pub const OPEN_TEST: Site = Site {
    name: "EventWindowOpenTest",
    rva: Rva(0x00a5_b57c),
};
pub const REQUEST: Site = Site {
    name: "EventWindowRequest",
    rva: Rva(0x00a5_b5a3),
};
pub const SECTION_END: Site = Site {
    name: "EventWindowSectionEnd",
    rva: Rva(0x00a5_befa),
};
pub const VERDICT_TAPS: [Site; 7] = [
    PAST_AI,
    PAST_VIEWER,
    SENDER_TEST,
    PLAYER_RESTRICTED,
    OPEN_TEST,
    REQUEST,
    SECTION_END,
];

pub const TITLE: Site = Site {
    name: "EventWindowTitle",
    rva: Rva(0x012c_8a43),
};
pub const PICTURE: Site = Site {
    name: "EventWindowPicture",
    rva: Rva(0x012c_7da4),
};
pub const OPTION: Site = Site {
    name: "EventWindowOption",
    rva: Rva(0x012c_8ebe),
};
pub const FALLBACK_OPTION: Site = Site {
    name: "EventWindowFallbackOption",
    rva: Rva(0x012c_90da),
};
pub const DESCRIPTION: Site = Site {
    name: "EventWindowDescription",
    rva: Rva(0x012c_95ee),
};
pub const COMMIT: Site = Site {
    name: "EventWindowCommit",
    rva: Rva(0x012c_99be),
};
pub const RELOAD: Site = Site {
    name: "EventWindowReload",
    rva: Rva(0x012c_7564),
};
pub const PRESENTATION_TAPS: [Site; 7] = [
    TITLE,
    PICTURE,
    OPTION,
    FALLBACK_OPTION,
    DESCRIPTION,
    COMMIT,
    RELOAD,
];

pub const WINDOW_VTABLE_RVA: Rva = Rva(0x02a2_0830);
pub const WINDOW_PENDING_INSTANCE: usize = 0x60;
pub const WINDOW_EVENT: usize = 0x68;
pub const WINDOW_ACTOR_TAG: usize = 0x70;

pub const TITLE_STRING_FROM_FRAME: isize = -0x20;
pub const DESCRIPTION_STRING_FROM_FRAME: isize = -0x40;
pub const PICTURE_RESOLVED_FROM_FRAME: isize = -0x50;
pub const PICTURE_KEY_FROM_STACK: usize = 0x30;

pub const VERDICT_FAILURE: &str = "Event window verdict / tap sentinel, construction or publication failed";
pub const PRESENTATION_FAILURE: &str =
    "Event window presentation / tap sentinel, construction or publication failed";
