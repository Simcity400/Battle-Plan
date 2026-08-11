# Extraction to Replay boundary

`documents/DECISIONS.md` defines intent. Current implementation status belongs
to each layer's status authority, not to a second seam ledger.

Replay's complete machine-readable list of externally supplied requirements is:

`replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json`

Its mandatory generated human view is:

`replay_interface/documents/EXTRACTOR_INPUT_RESIDUALS.md`

Each ledger row binds the carrier, exact identity requirements, Replay consumer,
packet authority, unavailable behavior, and evidence. Producer implementation
status remains owned by the Extractor authorities:

- `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
- `extractor/documents/RE_REMAINING.md`

Replay must not infer producer completion, copy volatile row counts, or maintain
a parallel waiting-list table. Packet-local schemas and rendering laws may add
field-level requirements, but they must link the ledger row and must not redefine
its status. Missing or invalid external data remains feature-local unavailable;
it is never measured empty or substituted.

Git history preserves closed seam chronology.
