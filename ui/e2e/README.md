# Widget comparison

The [coverage inventory](../../docs/rebuild-acceptance.md) identifies native,
synthetic-scale and remaining family checks separately from these browser captures.

The harness opens two prepared instrumented worktrees in installed Microsoft
Edge. At most two workers capture every registered widget's visible text, complete
ordered model and screenshot. Each widget has one absolute 60-second budget,
including recording open, settling, history preparation and evidence publication.
Readiness waits for lazy widget code as well as products, fonts and images; a
loading fallback cannot certify a capture. Exact JSON operands and RGBA pixels
produce PASS or DIFF. Missing, incomplete,
failed or over-budget captures produce ERROR and fail the command.

```powershell
$env:PATH = 'C:/Program Files/Git/bin;' + $env:PATH
bash captures/g0/central/wait-quiet.sh
if ($LASTEXITCODE -ne 0) { throw 'Quiet gate failed' }
$env:BATTLE_PLAN_GAME_DIR = '<installed game directory>'
$env:BATTLE_PLAN_USER_DIR = '<hoi4-user-directory>'
npm.cmd run parity:compare -- <prepared-instrumented-worktree> <prepared-instrumented-worktree>
bash captures/g0/central/wait-quiet.sh
if ($LASTEXITCODE -ne 0) { throw 'Quiet gate failed' }
npm.cmd run test:parity
```

Run the quiet gate before every browser command. Do not start while
`build_slots/timed_window` or `bench_pause` exists, or the game
lock owner mentions `diag`. Both worktrees must already have dependencies installed
(or a prepared dependency junction). Their package locks must match the installed
versions and integrity records. Builds without all parity hooks refuse before
recording open. Keep source, dependencies, installed content and shader tools
unchanged during comparison. Both roles always capture afresh.

The fixed recording is `GER 1943-10-24 to 1944-10-24.battleplan` in
`captures/widgetsyear/library`, engine hour 60836486. The existing
release bridge and shader tools are in `captures/bridge-release`.
`BATTLE_PLAN_PARITY_BRIDGE` can select another existing compatible release bridge.
The harness never builds Rust and refuses known debug bridge input paths. Each
role uses a disposable adapter with its own Vite and native scratch. The existing
native content hash index and history cache seed private copies in each adapter;
`BATTLE_PLAN_PARITY_NATIVE_SEED` selects a prepared native input directory. Native readers
check file identities, recording bytes, build identity and request/content keys;
writes stay in the private copies. Linked history inputs refuse.
Prepared native inputs measure warm-input capture time; they do not demonstrate
cold native product performance or reuse model/screenshot evidence.

`BATTLE_PLAN_PARITY_PREPARED_SEED` explicitly selects a `HOI4Replay/prepared` directory
created by the same release preparation pipeline for the unchanged recording and
content inputs. Each adapter copies every supplied page and manifest into its
private `LOCALAPPDATA/HOI4Replay/prepared`; writes and Clear affect only that copy.
Linked inputs refuse. The native bridge still checks recording/content stamps and
domain schemas, so a supplied seed cannot establish readiness by itself. Each
role retains `native-inputs.json` with the input path, per-file byte/mtime stamps
and total copied bytes. Warm seeding is part of capture provenance and does not
establish cold preparation, open, query or frame-rate performance.
Source marker timestamps reflect committed source changes, rather than checkout time. Closing
the server awaits its retained bridge child's close before scratch removal.
Teardown unlinks the shader-tool junction without traversing its input.

The Python launcher starts Node suspended, assigns it to one retained Windows Job,
then resumes it. All browser and bridge descendants inherit ownership. The preload
refuses PID termination and Playwright's taskkill fallback. Edge's exit cleanup is
handed to the Job at launch; one browser serves both builds with fresh contexts.
The runner publishes comparison results before exiting, then the supervisor reaps
its retained Job and reports `activeAfterClose: 0`. No process-name, profile or
ancestry sweep supplies termination authority.

Edge uses software rasterization, 1440x1000, DPR 1, English/UTC, dark theme,
bundled fonts and reduced motion. Readiness waits for requests, fonts, images and
stable visible text, geometry and canvas. Independent image resource identities
prevent previous decode sizes changing screenshot pixels. Screenshots use
Playwright's persistent session. Models preserve actual row order, exact operands,
referenced content and selected history boundaries. Preparation holds the original
history readers; offscreen labels and artwork keep their ordinary demand.
Production builds expose no parity hooks. Comparison aliases volatile lease IDs
while preserving shared and distinct ownership.

Evidence lives in `captures/parity-baselines`;
`BATTLE_PLAN_PARITY_CACHE` selects another directory outside Git. Each role writes
`manifest.json` and widget JSON/PNG files. The head also writes `comparison.json`
with PASS/DIFF/ERROR counts and phase wall times. Evidence remains until its owner
removes it. Installed content and recordings stay read-only.

## Explicit capture cases

`BATTLE_PLAN_PARITY_MATRIX` selects a version-1 JSON file outside Git. Without it, the
runner captures each registered widget at its defaults. A matrix can request
several countries, time modes or option sets for the same widget:

```json
{
  "version": 1,
  "cases": [{
    "id": "focus-follow",
    "kind": "national-focus",
    "hour": 100,
    "countries": {"kind": "tags", "tags": ["recorded opaque tag"]},
    "time": {"mode": "follow"},
    "models": ["Completed focuses in order"],
    "focusDetails": "first-visible"
  }]
}
```

Replace example dates/tags with recording facts. Each case has a unique `id`,
registered `kind` and native integer `hour`; optional `height`, `countries`, `time`
and `options` use the application's saved widget schemas. Unknown matrix fields,
unsupported widget options/time modes and choices that would be changed during
storage validation refuse. Both instrumented worktrees must expose
`capabilities().scenarioVersion === 1`; an older hook cannot silently ignore the
requested countries or options. The instrumentation lives only in parity builds.
An optional `models` list requires those exact model labels; missing providers
fail the case instead of certifying numeric equality from a screenshot.

`focusDetails` is an exact decimal completion ordinal, or `first-visible`. The
runner clicks the actual completion control and records the reached ordinal;
different reached ordinals are errors. A requested offscreen control that cannot
be reached fails the case instead of substituting another row. The unchanged
60-second case budget includes open, action, settling, full-model gathering and
publication. Case IDs, complete requested choices and actions remain in the
manifests and comparison. Multiple cases never overwrite another case's PNG or
JSON. The comparator preserves actual returned order, exact operands, nulls and
refusals; its existing aliases cover only capture-local runtime ownership.

Prepared focus has a complete ordered model hook. Produced and balance charts
also require their exact numeric model hooks in the compared worktrees; screenshot
and visible-text equality alone do not establish numeric parity. Width-summary
geometry follows the approved chart summary change and is measured separately
from exact values, read dates, keyboard stops and list rows.

Model providers use `windowed`, `table`, `log` or `chart`, a stable semantic label,
`complete`, `total` and every ordered row. Rows retain contiguous `index` and
unique occurrence `key`; repeated observations use distinct occurrence keys.
Additional semantic fields and raw evidence are retained. Top-level `id`,
`source`, `steps`, `traversal`, `geometry` and `diagnostics` belong to capture
instrumentation and are omitted from comparison; recorded source/read identity,
refusals and other semantic facts belong inside rows or a separate model field.
Explicit `prepare()` gathers any additional pages, `ready()` remains false until
complete and `error()` preserves failures. Capture then checks completeness and
the complete ordered row inventory.
