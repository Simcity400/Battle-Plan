# Unit and counter production law

## Authority and document role

This is the current normative law for XMAP13 unit entities, counter grouping,
counter presentation, selection, hit targets, and light-depth caster rows. The
complete packet contract and exact-a729 evidence inventory are recorded in
`replay_interface/re_evidence/rendering_laws/XMAP13/PRODUCTION_CONTRACT.md`.

Historical CAP6 captures are comparison oracles for natural counter size,
observed frames, and operation order. They are not product identity, camera,
grouping, placement, visibility, or draw authority. The product path does not
call the retained CAP6 constructors.

## Semantic input

XMAP13 consumes the top-level `map_unit_presentation` domain with schema
`hoi4.xmap13.unit-semantic.a729.v5`. The envelope is viewer-independent,
current-hour, complete, pointer-free, and digest-bound. It contains one
engine-enumerator-ordered row for every typed division, task force, and air
wing, plus the complete Army/ArmyGroup identity census.

Each unit row binds exact owner and anchor-province joins; entity, state,
PdxMesh, mesh, LOD, shape, material, animation, transform, and skin-palette
selection; counter grouping and presentation alternatives; shadow eligibility;
and the admitted effective-content identities that prove those selections.
Skinned rows carry the exact current 3,200-byte palette, not only its digest.

Task-force rows also carry the complete current CShip member census. Formation
rows bind the complete orders-group manager and country ownership. Dependent
packets consume the stable XMAP13 identity provider rather than reconstructing
unit or formation ownership.

If the semantic domain is absent or explicitly unavailable, `UnitsCounters`
is typed unavailable for that hour. There is no previous-hour carry-forward,
recorder-selected draw import, or guessed transform/counter.

## Effective-content admission

Every selected asset is named by an admitted `UsedContentKey`. XMAP13 verifies:

1. the effective `.asset` entity/clone binding;
2. the effective `.gfx` PdxMesh registry;
3. the selected `.mesh` LOD, shape, draw ranges, material overrides, texture
   closure, and hit-test AABB;
4. selected `.anim` states, events, particle and sound invocations;
5. the complete effective counter GUI/GFX/DDS/font surface; and
6. all three effective `NMapIcons` grouping thresholds.

The grouping thresholds are divided by the exact f32 `100000.0` and compared
with viewer-owned XMAP03 current-eye Y. The viewer camera also owns projection,
frustum selection, and mesh hit testing. XMAP05 owns land/task-force disclosure;
its authenticated air-wing spotting absence keeps foreign air-wing presentation
unavailable rather than inventing a visibility rule.

Once a required dependency is present, any digest, join, content, model,
animation, GUI, sprite, font, or texture disagreement is corruption. It is not
downgraded to an unavailable family.

## Exact grouping and placement

The engine forms groups incrementally in source order and inserts a member into
the first compatible `CUnitCounterItem`. Compatibility includes domain,
country, owner, command container, parent, and icon predicates; Replay does not
replace it with a distance-only rule.

For every grouping tier and each allegiance (`world_observer`, `ours`, `enemy`,
and `other`), the producer supplies exact stable group/item/member identities,
dense ordinals/counts, world anchors, and complete membership alternatives.
Tier zero is singleton. Higher tiers emit one displayed counter group rather
than one counter per member.

Replay selects the live allegiance alternative, applies current disclosure and
viewer-camera culling, preserves surviving source order, and recomputes the
dense group/item census and sequential f32 arithmetic-mean anchor for the
surviving subset. It never preserves producer counts from a different
allegiance or full-camera census.

Placement uses the viewer camera's homogeneous projection, NDC-to-viewport
conversion, authored alignment, half-away-from-zero rounding, accumulated
ancestor offsets, and child-height-plus-four stacking. Immediate-parent
centering preserves half-pixel results. Unsupported orientations or numeric
forms fail closed.

## Complete counter surface

Schema v5 carries the complete painter-ordered primitive surface for every
tier, allegiance, and selected/unselected alternative. Each surface includes
an explicit primitive or explicit hidden row for every authored drawable
control: backing, overlay, flag, symbol, portrait, bar, badge, active ability,
HQ/leader marker, naval/air surface, and text.

Completeness is checked against the effective GUI, including unnamed drawables,
modded widget kinds, nested offsets, omitted-axis zero defaults, and exact
`100%`/`100%%` parent-relative extents. Unsupported relative/unit forms fail
closed. Exactly one primitive represents each static drawable control.

The external `active_abilities` grid has an exact measured empty-or-dense
instance census. Its supported template has exactly one drawable child,
`active_ability`; an additional drawable child fails closed. Template absence
is rejected only when a land counter needs it.

Sprite rows authenticate authored and runtime-selected identities, atlas frame
source, rectangle, extent, rotation, modulation, GFX binding, and texture. Only
the air `wing_type` atlas permits runtime frame selection over its authored
initial frame. Code-textured flags retain their authored sprite binding.
Organisation and strength bars carry exact visible fractions and the proved
`1.5708` rotated clip law.

Text rows bind the authored control rectangle, value source, font alias,
BMFont descriptor, selected atlas pages, horizontal format, and top-origin
vertical placement. Native sprites use f32 modulation, linear sampling, and
the proved `-0.8` mip bias.

## Native execution and dependent packets

Static PdxMesh geometry uses the exact 48-byte vertex layout. Skinned geometry
uses the exact 76-byte layout plus the 3,200-byte VS b2 palette. Unit entities
execute on MainScene; counter primitives execute on the separately
authenticated presentation target.

The packet emits independent sealed component receipts for unit entities and
counter quads. A nonempty receipt binds every actual ordered target-bound draw;
an empty semantic census produces an authenticated-empty enumerator witness.
Selection and caster rows cannot manufacture draw receipts.

XMAP09 consumes the same selected geometry as ordered light-depth casters.
XMAP14 and XMAP15 consume the stable identity provider for anchors and
selection, and XMAP20 consumes admitted entity/state/authored-event identity.
Those packets do not infer or reorder XMAP13 rows.

## Fail-closed boundary

Production rejects incomplete or duplicate enumerations, stale hours, wrong
binary/content identities, absent current viewer state, malformed grouping or
alternative matrices, incomplete GUI painter censuses, unsupported modded
drawables, missing native resources, and target/assignment/receipt drift. It
does not substitute selected-Present pixels, a recorder camera, CAP6 grouping,
static country colors, guessed badges, placeholder text, or synthetic unit
geometry.
