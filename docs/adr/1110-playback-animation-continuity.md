# 1110. Preserve viewer animation history across continued playback

The host clock's seek counter distinguishes an intentional seek from playback that outruns frame preparation. It survives coalesced dashboard and full-map clock updates. An explicit seek resets history even if it targets the current or adjacent hour. Moving a pinned target is a seek because it has no running clock. Pause, camera and fog changes alone keep the current seek identity.

A forward step or skipped hour preserves runtime history on the same branch when the viewer continues, including when preparation finishes after the clock pauses. The resolved viewing country participates in history. A viewing-country change recreates unit entities in provinces owned by the new country and retains other unit histories. Initial loads, seeks, steps backward and branch switches recreate all unit entities. Crossing a recorded gap resets the presentation cycle for trains, raids and weather while preserving unit histories, because the game can continue running across a recording gap.

[Presentation continuity](../../crates/viewer-runtime/src/presentation.rs)
