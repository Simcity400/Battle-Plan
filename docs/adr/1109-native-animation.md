# 1109-B. Preserve the sprite animation draw contract

Animation resources load eagerly. Each retained instance gates its records from its own clock and trigger starts, or from its first draw for a non-trigger record. A missing clock, record, or required trigger start is reported as unavailable. A trigger whose start depends on a pointer-driven state change needs the retained instance. Nonlooping records stop once elapsed time is greater than their duration. The comparison and saturating arithmetic preserve NaN handling because those values affect which records run.

The first two active records choose the animated shader technique and bind a pair of mask and animation textures per record. An unavailable animation mask binds the null texture. The sprite's own mask keeps its regular binding while the animated program omits the masking variant. The CPU produces each record's texture bindings and constants, including rotation, UV scale, offsets, blend factor and mode, and clamp state. Only supported animation types write the type and animation offset components. The unwritten tail and any unwritten type-dependent components retain their previous contents.

[Animation records](../../crates/viewer-native-gui/src/native_gui/animation.rs)
