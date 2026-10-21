# shader-pipeline

Shared executable PdxShader backend for map passes and native GUI. [installed_content_core::pdxshader](../installed-content-core/src/pdxshader/mod.rs) owns parsing and DX11 source assembly; DXC emits SPIR-V and naga validates/translates it to WGSL. This crate has no GUI layout dependency and never chooses a substitute program by effect filename.

Its content dependency is `installed-content-core`; it does not depend on the `installed-content` facade or recorded frames, so engine and replay edits do not rebuild it.

## Source map

| Module | Responsibility |
|---|---|
| [library.rs](src/library.rs) | Content-scoped effects, renderer globals and lazy technique/define variants. |
| [artwork.rs](src/artwork.rs) | Isolated native sprite shader output for dashboard PNGs, with content-scoped sources and native register/texture binds; the stack's map size GUI effect globals carry (`map_size`). |
| [lib.rs](src/lib.rs), [source.rs](src/source.rs) | Compiler discovery, compilation, reflection and stage/register contracts. |
| [cache.rs](src/cache.rs) | Persistent compiled payloads and diagnostics; owns the compiler/translation ABI. |
| [gpu.rs](src/gpu.rs), [gpu/arena.rs](src/gpu/arena.rs) | Device-local pipelines, state lowering, bindings and staged uploads. |
| [stream.rs](src/stream.rs) | Retained native program, declaration, resources and render states. |
| [vertices.rs](src/vertices.rs), [vertices/native.rs](src/vertices/native.rs) | Vertex formats and native overlapping-stride repacking. |
| [reads.rs](src/reads.rs), [clock.rs](src/clock.rs) | Conservative known-byte checks and clock dependency/settling analysis. |
| [atlas.rs](src/atlas.rs) | The proof that a masked flag program reads its atlas rectangle only to map its texture coordinate. |
| [sampling.rs](src/sampling.rs) | Sampling forms WebGPU cannot express directly. |
| [tests.rs](src/tests.rs), [tests/](src/tests/) | Synthetic compiler/GPU checks and opt-in installed-effect checks. |

## Effect selection and failures

`Library` owns one resolved content stack. Effect names, includes and paths keep their bytes through lookup. `load` parses; `Effect::select` compiles only a technique first used. `Effect::variant` keeps resolved source, globals and cache while changing caller defines; repeated variants retain handle identity for GPU reuse. Vanilla and mods take the same path. Over a presentation stack, an effect a visual mod replaces is used only when every technique the recorded effect (`ResolvedLayers::without_overlays`) compiles also compiles from the mod's sources; otherwise `load` returns the recorded effect and names the reason with the stack's overlay refusals. An accepted replacement keeps its recorded counterpart: each of its variants is held to the same rule under the variant's defines, and a pass whose pipeline the replacement cannot build falls back to the recorded effect, each refusal named: passes built from an effect through `library::with_recorded`, those built from a selected technique through `library::with_recorded_selection` (`Selection::fall_back`), and native stream draws by drawing the same technique of the recorded program with the bound state. Effects the mods leave alone resolve to the same identity and compile once.

Construct the library with `library::globals` from map size and the client's texture-decode settings. Globals precede caller defines; technique defines follow. GUI effects need these globals too. Never substitute vanilla dimensions, define lists or source paths in consumers.

Missing effect files and undeclared techniques select `None`, preserving native retained-program behavior. Malformed source or a declared technique that fails is a named error. Consumers preserve the binds the native draw makes, forget unavailable bindings, count failures and omit affected draws. A missing texture must not silently use an earlier texture; a failed declared technique must not run an earlier program. Surface `cache::Diagnostics` and library/GPU/stream refusals alongside ordinary missing-input diagnostics.

## GPU draw contract

`gpu::Pass` accepts caller vertex/instance layouts, topology, targets, multisampling, culling and depth/stencil. Authored blend, depth/stencil and raster states lower independently: consumers may retain an engine-forced state for one while using authored states for others. Constant/texture slots have independent vertex/pixel namespaces; use `binding(slot, stage)`. Vertex reflection preserves float/signed/unsigned kinds. Callers own vertex/index buffers, draw order, viewport, scissor, stencil reference and blend constants.

`Stream` retains the native program and input declaration, sampler/texture/constant bindings, strides and independent render-state handles. Null program selection changes neither program nor declaration; later buffer binds can still change individual strides. Explicit null textures sample zero; missing bindings refuse. Scoped `inherit_bindings` copies state without sharing later mutations. `prepare_pass`/`prepare_native_pass` return the cached pipeline, resource bindings and required dynamic state; they do not establish submission order or infer absent native inputs. An unchanged program, declaration, strides, native states and complete pass reuse the preceding pipeline resolution; constant bytes, initialized ranges and textures are still checked for every draw. `select_program` borrows an immutable native declaration and clones it only when selection or layout changes.

A single-layer 2D SRV sampled as a cube reproduces the native hardware GPU's zero result, without reshaping the image or changing the retained binding for later 2D draws. Native D3D11 software-driver results for this invalid binding are device-dependent; 2D reflection textures carry a viewer diagnostic. Binding errors name the selected effect and technique.

Use one `gpu::Arena` per frame being encoded. Reset before the next frame; flush before submitting commands that reference staged uploads, including earlier text-bake submissions. Bindings remain valid until reset. A retained pass may own a separate arena, flushed when its inputs change and not reset while its submissions are reused; no frame arena may overwrite those uploads. `Arena::write` also defers caller-buffer writes until flush. Upload constant blocks and fetched/repacked streams, not whole native allocation capacities. Pipeline/bind-group caches avoid per-draw buffer/group creation and retire unused groups after a frame.

Test-support `Arena::checkpoint` snapshots the next flush's exact caller writes and initialized chunk suffixes for ordered-byte fixture comparisons. It is absent from production; ordinary rendering uses `Arena::flush`. The read-only Arena proof and checkpoint tests retain raw byte, offset, padding and destination-order coverage.

Supply physical allocation size separately from known written ranges. `Buffer::reads` conservatively reports post-elimination byte reads; unknown bytes inside an allocation refuse with stage, slot and range, while reads past its allocation are zero. A short initialized prefix is not the physical allocation. Partial constants retain validity across null binds/inheritance; complete uploads replace it.

`NativeBuffer` describes physical size, written prefix and sorted known ranges. Repacking preserves scalar formats, per-vertex/instance stepping, offsets and native retained strides. Out-of-allocation fetches and explicit null buffers supply zeros; omitted buffers and unknown in-allocation bytes refuse. Do not upload an entire ring or large allocation to work around overlapping attributes.

Consumers request available dynamic-uniform limits and optional native sampling/blend features. Programs beyond device limits refuse by name. Core conveniences for Sprite/Text/Circular layouts do not limit general passes.

## Sampling, signatures and clocks

Preserve D3D11 register/order semantics. Vertex outputs and pixel inputs link by declaration order; reordered signatures and non-prefix subsets refuse rather than being relinked by semantic. Raw code using registers the engine does not declare/bind, out-of-range D3D11 slots and unsupported resources refuse. Texture/sampler pairing comes from compiled code, including pairs with different register numbers. Active raw HLSL includes refuse; PdxShader includes resolve through the content stack.

`sampling.rs` lowers `MIRROR_ONCE` with native addressing/filter/LOD behavior; anisotropic mirror-once is a named platform gap. Mip bias applies to explicit-LOD sampling too. Point samplers use nonfiltering bindings. Without `FLOAT32_FILTERABLE`, filtered float32 2D views use a cached shader variant with texel/mip filtering at the original value precision; unsupported cube/anisotropic cases refuse. Unsupported address/blend features must remain refusals, not substitutions.

`Compiled::settles` derives clock behavior from the program, not effect names or a fixed timeout. Analysis is conservative: unknown forms remain active. Saturating duration expressions can settle; other direct clock dependence cannot. Consumers retain the native graphics/restart clocks and keep frames coming while the selected or inherited program changes.

## Cache

The CPU cache is `%LOCALAPPDATA%/HOI4Replay/shader-cache/v2/<sha256>.json`. Keys cover resolved root/ordered includes (including absence), DX11 header, emitted sources/defines and compiler/translation ABI. Entries verify key, ABI and checksum; changed/corrupt entries recompile. Writes rename completed temporary files. Write failures warn without refusing a successful compilation. There is no eviction.

Once cells serialize identical in-process requests; cross-process races leave a complete valid entry. Failures stay in memory only, so a newly installed compiler retries on the next content load/process. GPU objects remain device-local and key the complete pass/sampler state. Change the ABI when generated code, compiler or translation meaning changes; do not copy an old payload into a new ABI.

Isolated artwork retains parsed effect sources and map dimensions once per
resolved content stack. It uses the same recorded reads and diagnostics for
cache hits; icons do not reread the province bitmap for each draw. Stack clones
share these inputs, and a replacement stack has fresh inputs.

## Compiler and packaging

Discovery uses explicit `BATTLE_PLAN_SHADER_COMPILER` or exactly the sibling `shader-tools/dxc/dxc.exe`; there is no PATH/SDK search or runtime download. `dxc --version` must match the pin in [lib.rs](src/lib.rs), also named by the cache ABI. Discovery runs on the first cache miss, once per content load. Missing/wrong compilers and launch/DLL/compiler failures name the technique; cached programs remain usable.

Desktop builds stage the pinned official archive via [install-shader-compiler.ps1](../../tools/install-shader-compiler.ps1), with hash verification. Ship the whole `shader-tools/dxc/` directory beside `viewer.exe` and `replay-desktop.exe`: compiler/runtime DLLs and all three upstream licence files. Preserve Microsoft's distribution terms; its archive refers to a distributables list it does not include. The build produces an executable directory, not an installer.

The compiler uses the pinned HLSL/SPIR-V/naga settings in code. DXC has no native D3DCompile backwards-compatibility mode; legacy syntax unsupported by this route remains a named refusal. Source assembly, compilation and state lowering share the same stage and register contracts.

## Verification

From the repository root, set `BATTLE_PLAN_GAME_DIR` to the supported installation for tests that parse effects. Their authored sources are synthetic, while the shared lexer uses verified runtime metadata. Set `BATTLE_PLAN_SHADER_COMPILER` to the pinned `dxc.exe` for compiler/GPU tests and run:

```powershell
cargo fmt -p shader-pipeline -- --check
cargo clippy -p shader-pipeline --all-targets -- -D warnings
cargo test -p shader-pipeline
```

Synthetic tests cover lazy selection/variants, invalidation, native state lowering, partial constants/vertices, resource namespaces, sampling and clock analysis. Consumer submission and pixel comparisons belong in the [viewer checks](../viewer/README.md#compare-changes).

Installed checks use `BATTLE_PLAN_GAME_DIR` and `BATTLE_PLAN_SHADER_COMPILER`, without launching the game. Run `cargo test -p shader-pipeline -- --ignored --skip installed_vanilla_persistent_cache_across_processes --nocapture --test-threads=1` for installed GUI compilation and GPU cases.

For the separate disk-cache process check, set `BATTLE_PLAN_CACHE_TEST_DIR` to new external scratch. Run `cargo test -p shader-pipeline installed_vanilla_persistent_cache_across_processes -- --ignored --nocapture` twice: `BATTLE_PLAN_EXPECT_COMPILES=1`, then `BATTLE_PLAN_EXPECT_COMPILES=0`. The second process must not discover the compiler. Map effects also require their consumer-supplied globals. Cache reuse, identical-input pixels and a matched live game frame are distinct checks.

`artwork::render_batch` renders independent targets in one submission with one readback buffer. Only draw preparation and submission hold the renderer mutex; mapping, waiting and copying happen after releasing it. Uploaded textures reuse already-read decoded bytes within a 64 MiB budget; the least recently used uploads leave when the budget is exceeded. Submission releases cached texture bind groups so they cannot retain removed uploads.

`artwork::render_layers` draws a drawable's passes in order into one half-float target, such as a masked shield's flag and overlay. Each pass keeps its technique's colour blend while alpha accumulates coverage, and the result returns as straight alpha, so the page compositing the image gives what the passes give over the frame. Only a source-over colour blend flattens exactly. Any other pass is refused by its effect, never drawn with a substitute blend. `Draw::pixel_constants` binds b0 to the vertex stage only where the native draw does, as a masked flag pass does.

The game's flag atlas isn't recorded, so a masked flag pass is drawn from a page holding only its flag, at the rectangle `[0, 0, 1, 1]`. `artwork::flag_rectangle_mapped` accepts a flag program only when the proof in [atlas.rs](src/atlas.rs) holds over its compiled Up stages (re-parsed from their cached WGSL). The rectangle may reach the vertex output only as `offset + texcoord · size` on each axis, from the unchanged `TEXCOORD0` input. The pixel stage may use that output only as the `x, y` coordinate of an ordinary sample of the flag's texture, which nothing else samples. The proof accepts only straight-line code, run in statement order: an entry point alone, or the SPIR-V front's entry wrapper that stores its inputs in variables, calls one body function without arguments or a result, and returns what the body stored. Variables start at their initialiser, or zero, and each store replaces what it covers. A branch, loop, switch, `select`, discard, any other call, or any dynamic index is refused. Under that proof the draw matches any atlas cell, except for which texels a filter reads past the cell's edge. Anything else, or anything the proof doesn't follow, is refused by the effect's name.
