# Production content interpreters

Date: 2026-07-18  
Scope: replay production content only; no extractor catalog or replay-app entry changes.

## Result

Production now has one admission-bound content interpreter seam. Every request
starts from a complete `UsedContentKey` in the portable replay, is checked for
full equality with the validated admission, and is read again through the
launcher-equivalent VFS. Returned values carry all consumed keys as provenance.
No install path, vanilla catalog, asset list, layout, localisation value, or
content default is embedded in the interpreter.

Promoted exact routes:

- Clausewitz/Jomini text syntax, GUI element trees, GFX sprite definitions,
  scripted-GUI compilation, and recorded-language localisation.
- Shader effect syntax and PDX binary/text mesh/animation containers.
- DDS/TGA/PNG decode and BMFont load through the admitted asset adapter.
- An exact-byte route for the existing specialised define, map, entity,
  model/material and particle consumers; those consumers retain ownership of
  their proven engine semantics.

All tolerant Clausewitz parser recoveries are rejected in production. A
language file whose header differs from the replay manifest is rejected. A
path with the wrong file family, a changed key, an unrecorded installed file,
missing required content, changed DLC selection, or changed payload is rejected.

## VFS and portability gates

The production test creates base and mod content independently, mounts the mod
at a sparse recorded ordinal, exercises `replace_path`, and proves that GUI,
GFX, localisation, scripted GUI, entity/material/particle definitions, shader,
PDX mesh, define source and positional map source all come from effective VFS
winners. A second gate rejects a changed DLC set and missing/replaced content.
A third rebuilds the same manifest against different absolute game, profile and
mod roots and parses successfully; the replay file therefore remains the only
binding between independently distributed extractor and replayer.

## Deliberately omitted semantic families

These are data needs, not approximations or production defaults:

- Arbitrary Lua execution. Existing consumer-specific, proven define readers
  remain admitted; a generic Lua VM law is not claimed.
- Arbitrary Clausewitz trigger/effect/controller execution. Syntax is retained,
  while only specialised authenticated evaluators execute semantics.
- Generic entity/particle/material execution. Definitions and bytes resolve
  through VFS, while only specialised proven render consumers execute them.
- Generic positional map-text grammar. Railways, supply nodes and related files
  remain with their exact map-specific interpreters.

The machine-readable inventory and test receipt are under
`replay_interface/re_evidence/production_content_interpreters/`.

## Verification

- `cargo test -p game_ui --lib content_interpreter --no-fail-fast`: 2/2,
  repeated twice.
- `cargo test -p game_ui --test production_content_interpreter --no-fail-fast`:
  3/3, repeated twice after the final fixture expansion.
- Final integration coverage includes mod overrides, `replace_path`, changed
  DLC, missing content, recorded non-English localisation, sparse layer ordinal,
  relocated install/profile/mod roots, shader, mesh, definitions and map bytes.

