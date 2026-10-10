//! `CContainerWindow` primary object, with a CGuiObject subobject at +0x30.
//!
//! These are retained presentation layouts. No GUI method is called by the collector.

/// Primary container vtable.
pub const VTABLE_RVA: usize = 0x02b2_e4b8;
/// GUI subobject vtable.
pub const GUI_VTABLE_RVA: usize = 0x02b2_e730;
/// GUI subobject displacement.
pub const GUI_OBJECT: usize = 0x30;
/// Enclosing flags byte aliasing GUI flags.
pub const FLAGS: usize = 0xa5;
/// Authored definition pointer.
pub const DEFINITION: usize = 0x15d0;
/// Content-extent getter virtual byte offset, returning i32[2].
pub const CONTENT_EXTENT_VSLOT: usize = 0xe8;
/// Vertical scrollbar pointer.
pub const VERTICAL_SCROLLBAR: usize = 0x15e0;
/// Horizontal scrollbar pointer.
pub const HORIZONTAL_SCROLLBAR: usize = 0x15e8;
/// Graphical wrapper pointer.
pub const GRAPHICAL_WRAPPER: usize = 0x15f0;
/// Paint-root pointer.
pub const PAINT_ROOT: usize = 0x15f8;
/// Scroll synchronization function.
pub const SYNCHRONIZE_SCROLL_RVA: usize = 0x022a_fa30;

/// Constructor initializing movement, opacity and anchored positions.
pub const CONSTRUCT_RVA: usize = 0x0229_f050;
/// Background-pointer array used by parent clipping.
pub const BACKGROUNDS: usize = 0x10d8;
/// Background count, i32, used to derive unspecified clipping.
pub const BACKGROUND_COUNT: usize = 0x10e4;
/// Child-container array used for recursive fades.
pub const CHILD_CONTAINERS: usize = 0x1330;
/// Child-container count, i32.
pub const CHILD_CONTAINER_COUNT: usize = 0x133c;
/// Shown position adjusted by its anchor.
pub const SHOW_POSITION: usize = 0x1438;
/// Hidden position adjusted by its anchor.
pub const HIDE_POSITION: usize = 0x1468;
/// Base position updated before retargeting.
pub const POSITION: usize = 0x1498;
/// Unadjusted original shown position.
pub const ORIGINAL_SHOW_POSITION: usize = 0x14c8;
/// Unadjusted original hidden position.
pub const ORIGINAL_HIDE_POSITION: usize = 0x14f8;
/// Evaluated base position, i32[2].
pub const EVALUATED_BASE_POSITION: usize = 0x1528;
/// Authored size record.
pub const AUTHORED_SIZE: usize = 0x1530;
/// Margin record with separate edge coordinates and percentage bytes.
pub const MARGIN: usize = 0x1564;
/// Independent secondary margin record.
pub const SECONDARY_MARGIN: usize = 0x157c;
/// Owned logical clip rectangle, i32[4]; propagation checks pointer identity.
pub const OWN_CLIP: usize = 0x1594;
/// Final movement target, i32[2].
pub const MOVEMENT_TARGET: usize = 0x15a4;
/// Opacity, f32, initially one.
pub const OPACITY: usize = 0x15d8;
/// Parent-owned clip pointer; preserve its identity when receiving a distinct rectangle.
pub const PARENT_CLIP: usize = 0x1628;
/// Fade direction, i32; one shows and two hides.
pub const FADE_DIRECTION: usize = 0x1630;
/// Fade duration, f32 seconds.
pub const FADE_DURATION: usize = 0x1634;
/// Fade delay, f32 seconds.
pub const FADE_DELAY: usize = 0x1638;
/// Fade easing kind, i32.
pub const FADE_KIND: usize = 0x163c;
/// Elapsed fade time, f32 seconds.
pub const FADE_ELAPSED: usize = 0x1640;
/// Current movement duration, f32 seconds.
pub const MOVEMENT_DURATION: usize = 0x1644;
/// Shown movement duration, f32 seconds.
pub const SHOW_DURATION: usize = 0x1648;
/// Hidden movement duration, f32 seconds.
pub const HIDE_DURATION: usize = 0x164c;
/// Elapsed movement time, f32, cleared on retarget and incremented by frame delta.
pub const MOVEMENT_ELAPSED: usize = 0x1650;
/// X distance, f32, converted from signed wrapping target-minus-current i32.
pub const MOVEMENT_DISTANCE_X: usize = 0x1654;
/// Y distance, f32.
pub const MOVEMENT_DISTANCE_Y: usize = 0x1658;
/// Movement mode, i32; one shows, two hides, completion clears it.
pub const MOVEMENT_MODE: usize = 0x1678;
/// Active movement easing, i32.
pub const MOVEMENT_KIND: usize = 0x167c;
/// Authored show easing, i32.
pub const SHOW_KIND: usize = 0x1680;
/// Authored hide easing, i32.
pub const HIDE_KIND: usize = 0x1684;
/// Orientation, i32, used by anchored placement.
pub const ORIENTATION: usize = 0x1688;
/// Origin, i32, passed to the anchor helper.
pub const ORIGO: usize = 0x168c;
/// Movement-complete byte, initially one.
pub const MOVEMENT_COMPLETE: usize = 0x16ac;
/// Hide-children-on-completion byte.
pub const HIDE_CHILDREN: usize = 0x16ad;
/// Hide-root-on-completion byte.
pub const HIDE_ROOT: usize = 0x16ae;
/// Moveable byte checked before retargeting.
pub const MOVEABLE: usize = 0x16af;
/// Clipping byte selecting the clip branch.
pub const CLIPPING: usize = 0x16b1;
/// Optional custom clip-provider pointer.
pub const CLIP_PROVIDER: usize = 0x16f0;
/// Primary position-setter virtual byte offset, receiving an interpolated i32 pair.
pub const SET_POSITION_VSLOT: usize = 0x30;
/// GUI screen-position getter virtual byte offset, returning logical pixels.
pub const GUI_SCREEN_POSITION_VSLOT: usize = 0x30;
/// Primary clip-propagation virtual byte offset.
pub const PROPAGATE_CLIP_VSLOT: usize = 0x170;
/// Primary opacity-setter virtual byte offset, receiving f32 opacity and descendant flag.
pub const SET_OPACITY_VSLOT: usize = 0x180;
/// GUI frame-update virtual byte offset; fade precedes movement with the same delta.
pub const GUI_UPDATE_VSLOT: usize = 0x248;
/// Retarget function updating target, distance, elapsed time and completion.
pub const RETARGET_RVA: usize = 0x022a_f130;
/// Fade-out request with an early return before descendant traversal.
pub const BEGIN_HIDE_FADE_RVA: usize = 0x022a_f4b0;
/// Fade-in request retaining the already-active fade branch.
pub const BEGIN_SHOW_FADE_RVA: usize = 0x022a_f540;
/// GUI show handler adjusting to the primary receiver before fade and retarget.
pub const GUI_SHOW_RVA: usize = 0x022a_f5d0;
/// Primary show handler selecting duration and easing before retargeting.
pub const SHOW_RVA: usize = 0x022a_f6b0;
/// Fade update applying the new f32 opacity.
pub const UPDATE_FADE_RVA: usize = 0x022a_ffa0;
/// Movement update with complete and active interpolation branches.
pub const UPDATE_MOVEMENT_RVA: usize = 0x022b_01e0;
/// Clip calculation using current parent, margins and logical size.
pub const COMPUTE_CLIP_RVA: usize = 0x022b_03e0;
/// Clip propagation preserving parent pointer identity and content order.
pub const PROPAGATE_CLIP_RVA: usize = 0x022a_e870;
/// Layout update anchoring coordinates before recomputing and propagating clips.
pub const UPDATE_LAYOUT_RVA: usize = 0x022a_ef40;
/// Anchor helper receiving child/parent extents, orientation and origin with a separate output pair.
pub const COMPUTE_ANCHOR_RVA: usize = 0x0229_e0e0;
/// Smoothstep helper clamping before the cubic expression.
pub const SMOOTHSTEP_RVA: usize = 0x022a_f780;
