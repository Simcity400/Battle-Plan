# Viewer combat

Land and naval combat windows, retained result windows, bindings, calculations and hover text. Content contains the immutable operands each window reads; the viewer host builds a fresh content Arc with every verified content load and supplies the current frame, playhead and shared engagement history.

Native GUI layout, flags, textures, text, visibility and engagement history come from `viewer-foundation`. Combat refresh keeps its demand gates, retained naval contents, list membership order and presentation clocks. Source details are in [the module guide](src/README.md).

`test-support` exposes existing binding and GUI contracts for the viewer host's composition tests; shared synthetic inputs come from foundation's opt-in test support.
