# 1394. Viewer foundation crates

## Decision

Viewer consumers depend directly on the crates that own the types and functions
they use. The runtime owns scheduling, frame pacing and diagnostics shared
across the viewer and desktop app. Foundation owns recorded map data, including
perspectives, visibility and movement timing. The GPU crate owns map geometry,
textures, and the coordinate contracts that position icons and define their
tracks. Unit reads build on recorded data and geometry, while engagements own
combat selection and the history of those engagements. The native GUI owns
sprite submission, windows, text, and hit testing where those parts share the
same layout and draw types. Fixtures live with the interfaces they exercise,
and tests that cross layers live with the consumer.
