# recorder-platform

Windows process, memory, module and event operations, and installation/user/capture directory defaults. Directory discovery preserves registry and environment search order; capture/finalization policy belongs to recorder. Used by recorder and its comparison tools.

- [windows.rs](src/windows.rs)
- [directories.rs](src/directories.rs)

Shared development and content rules live in [AGENTS.md](../../AGENTS.md). Capture and fresh-save checks are described in the [recorder guide](../recorder/README.md).
