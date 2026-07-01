# Domain 59 registration patch plan

Lane files only were changed. Shared entrypoints remain untouched.

## Static artifact registration

- Domain id: `59`
- Domain name: `templates_equipment_catalog_live`
- Static artifact schema: `templates_equipment_catalog_live.static_artifact.v1`
- Replay payload file: `domain59_static_catalog.json`
- Manifest snapshot schema: `templates_equipment_catalog_live.static_snapshot.v1`

Patch plan when the orchestrator serializes shared registrations:

1. Add `templates_equipment_catalog_live` to the static artifact section of the replay manifest writer.
2. Copy `domain59_static_catalog.json` into the replay snapshot store before final pack.
3. Add a manifest gate that requires the catalog SHA-256 in `snapshot_000000.json` to match the packed payload.
4. Do not mark live template mutation rows complete from this patch.

## Future live compact registration

The remaining live rows need a DLL compact stream after a root is validated:

- `collector/src/lib.rs`: add `COMPACT_DOMAIN_TEMPLATES_EQUIPMENT_CATALOG: u16 = 59`.
- `collector/src/lib.rs`: schedule a domain-59 worker from the one existing injected collector/coordinator.
- `collector/src/lib.rs`: emit typed compact records for live template create/edit/delete and live id-to-catalog joins.
- `recorder_host/src/main.rs`: decode domain 59 into static artifact joins plus live mutation overlays.
- `recorder_host/src/compact_domain_registry.rs`: register domain 59 as `templates_equipment_catalog_live`.

No shared patch is justified until a live root/function/offset probe validates the in-memory template manager or conclusively rejects a candidate.
