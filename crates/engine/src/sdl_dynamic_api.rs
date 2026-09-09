//! SDL dynamic-API and event-name utility metadata in the pinned engine image.
//! These addresses do not establish C++ object ownership.
//! Sources below are under portable_replay/native/engine_evidence/.

/// Event-state thunk RVA; src/xmap20_particle_queue_producer.rs:38 and func_14234ddb0.decompile.c call it with event 0x1000 and enable value 1.
pub const EVENT_STATE_RVA: usize = 0x020a_16c0;
/// Dynamic-API initializer RVA; src/xmap20_particle_queue_producer.rs:67 and func_1420a42b0.decompile.c invoke it before the indirect call.
pub const INITIALIZE_RVA: usize = 0x020a_8760;
/// Engine event-name lookup RVA; src/xmap20_particle_queue_producer.rs:294 and func_1420c3060.decompile.c:386-387 map 0x1000 to SDL_DROPFILE.
pub const EVENT_NAME_RVA: usize = 0x020c_3060;
/// SDL_DROPFILE event ID; src/xmap20_particle_queue_producer.rs:292 and func_1420c3060.decompile.c:386-387 confirm the literal name.
pub const DROP_FILE_EVENT: u32 = 0x1000;
/// One-argument event-value thunk RVA; src/xmap20_particle_queue_producer.rs:316 and func_1420a42b0.decompile.c forward the value, without establishing free/disposal semantics.
pub const RETURN_EVENT_VALUE_RVA: usize = 0x020a_42b0;
/// Dynamic-API function-pointer slot RVA; src/xmap20_particle_queue_producer.rs:319 and func_1420a42b0.decompile.c dispatch through PTR_FUN_14309f190.
pub const RETURN_EVENT_VALUE_SLOT_RVA: usize = 0x0309_f190;
