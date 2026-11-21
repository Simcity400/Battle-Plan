# 0007. Snapshot a verified Boat toolchain and dependency cache

Routine Rust checks clone the `hoi4-check` image. Updating that image is a provisioning task. Start a fresh VM, push a clean main worktree to the source directory expected by `boat_setup.sh`, copy the setup script, and run it with that worktree's commit SHA. Snapshot `hoi4-check` after provisioning and verification succeed, then shorten the lifetime of the provisioning VM.

The image warms test targets and operational artifacts with the toolchain pinned by the worktree. Its Clang wrapper translates the forced-include argument used by `libudis86-sys` to the spelling supported by the CL driver. The release snapshot includes the viewer, recorder, collector and columnar check tool under Cargo's normal release profile. Provisioning records the source revision and toolchain versions so failed or mismatched image changes are traceable.

[Image setup](../../tools/boat_setup.sh) · [Boat client](../../tools/boat.py)
