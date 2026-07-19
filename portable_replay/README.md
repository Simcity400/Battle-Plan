# portable_replay

Neutral build-time format crates shared by both halves of the product. Both
distributions statically link these; **neither installed distribution needs
the other at runtime — the `.battleplan` file is the only runtime interface.**

| Crate dir | Package | Role |
| --- | --- | --- |
| `native/snapshot` | `portable_replay_format` | `.battleplan` container + snapshot format |
| `native/content_manifest` | `portable_content_manifest` | Portable content manifest (see `extractor/documents/PORTABLE_CONTENT_MANIFEST.md`) |
| `native/render_stream` | `portable_render_stream` | Render stream format |
| `native/compact_stream` | `portable_compact_stream` | Compact typed record stream |

Change ownership: format changes land here first, versioned, and must keep
both halves' readers/writers in lockstep within one commit. Spec layer above
these crates: `replay_interface/documents/HOI4REPLAY_CONTAINER_SPEC.md` and
`replay_interface/documents/REPLAY_SCHEMA.md`.
