# A28/d71 heterogeneous materialized-slot templates

Status: `PRODUCTION_SCHEMA_CORRECTED_FAIL_CLOSED`.

The retained A37/A28 slot/clip evidence establishes that `decision_grid` is a
heterogeneous collection. Its materialized slots may use ordinary decision,
targeted-decision, timed-decision, and on-map templates in one controller
collection. The prior d71 validation incorrectly required every
`MaterializedSlotTrace.row_template_id` to equal the collection-level
`row_template_id`.

The carrier already serialized a template identity on every materialized slot,
so no wire field or inferred discriminator was needed. Production validation is
now corrected at all three boundaries:

- extractor d71 requires every per-slot template identity to be nonempty and
  preserves it independently of the controller-level default/family template;
- replay rejects empty per-slot template identities but admits exact
  heterogeneous values without collapsing them;
- game_ui performs the same nonempty check and retains each slot's value for
  composition.

Slot IDs, bound item IDs, uniqueness, final-order membership, rectangles,
controller authority, and frame identity remain unchanged and fail closed.
TCV1 technology-canvas exactness is unaffected: its drawable kind, hierarchy
path, slot identity, final mixed order, and canonical hashes remain governed by
the separate A29 carrier and validator.

Static regression cases cover a heterogeneous round trip, UI admission, replay
preservation, and rejection of an empty per-slot template. Per repository
policy this subagent did not run builds, tests, verifiers, formatters, or Cargo.
