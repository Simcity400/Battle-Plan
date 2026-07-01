# air_variant_archetypes_live domain 46 registration patch plan

This lane does not edit shared collector or recorder entrypoints directly.
The orchestrator-owned integration should be a registration hook only.

## Collector hook

Shared file: `replay_native/collector/src/lib.rs`.

1. Keep the existing air compact domains and trackers as the source rows:
   `air_variant_stats`, `air_superiority_inputs`, `air_variant_defs`,
   `stockpile_variant_defs`, and the current shared token dictionary route.
2. Add the final compact id constant:
   `COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES_LIVE: u16 = 46`.
3. Reuse the lane crate helper:
   `build_air_variant_archetype_payload_with_token_dictionary`.
4. Build lane input rows from the validated live variant stat rows:
   `variant_id`, `equipment_token_raw`, `equipment_type_token_raw`,
   `interface_type_raw`, and optional air-superiority weight.
5. Pass the shared same-session token dictionary as
   `BTreeMap<u32, String>` keyed by `equipment_token_raw`. The lane helper
   fills missing `equipment_token_name` values before encoding. This is the
   hook that turns raw fallback rows like `equipment_token_raw_20378` into
   semantic rows such as `large_plane_maritime_patrol_plane_airframe_1`.
6. Emit the encoded payload on domain `46` with wire name
   `air_variant_archetypes`.
7. Preserve the existing fail-closed behavior: if the token dictionary is
   unavailable or partial, emit raw-token fallback badges rather than guessing.

## Recorder hook

Shared file: `replay_native/recorder_host/src/main.rs`.

1. Map wire domain name `air_variant_archetypes` and registry name
   `air_variant_archetypes_live` to compact domain id `46`.
2. Dispatch domain `46` payloads to the lane decoder gate:
   `decode_payload(offset, payload)`.
3. Merge the lane decoder counters into the existing compact-stream summary:
   `air_variant_archetype_rows`,
   `air_variant_archetype_live_token_dictionary_rows`,
   `air_variant_archetype_static_heuristic_rows`,
   `air_variant_archetype_base_airframe_string_rows`,
   `air_variant_archetype_role_string_rows`, and parse errors.
4. Keep `.battleplan` packing unchanged: domain `46` remains another compact
   domain data record in the single collector/recorder stream.

## Scope line

This patch plan closes the domain-46 shared token/name hook and reduces raw
token fallbacks where the shared dictionary resolves a token. It does not claim
full air operational or combat replay. Aces, sortie/combat pairing, loss cause,
exact displayed air-superiority percentages, and any non-extractable lifecycle
fields still require their own roots/domains or exact blockers.
