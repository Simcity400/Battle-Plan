# dashboard

Native widget data: opened recordings, typed products, the data service and the
host/viewer protocol. This crate reads recordings and their content stack; it
neither reads live game memory nor draws map pixels. See the
[architecture](../../docs/dashboard.md), [desktop host](../desktop/README.md) and
[widget framework](../../ui/src/widgets/README.md) for the surrounding flow.

## Layout

| Module | What it does |
|---|---|
| [source.rs](src/source.rs) | One recording revision, metadata, resolved content and typed per-source caches. |
| [metadata.rs](src/metadata.rs) | Branch bounds, gaps, initial country catalogue, frame checks, country-link resolution and supplying-read detection. |
| [catalogue.rs](src/catalogue.rs) | Shared lazy localisation, content databases, native GUI/sprite lookup, texture decoding and PNG replies. |
| [products/](src/products/mod.rs) | Product registry and shared domain helpers; each product's behaviour lives beside its code or in its widget README. |
| [country_history.rs](src/products/country_history.rs) | Shared native country identities, tag aliases and requested balance histories. |
| [read_runs.rs](src/read_runs.rs), [ranking.rs](src/ranking.rs) | Read-derived value runs, known coverage and event-based top-N selection/clipping. |
| [triggers.rs](src/triggers.rs), [scope.rs](src/scope.rs), [character_scope.rs](src/character_scope.rs) | Recorded-hour trigger evaluation, scope links, aliases and character operands. |
| [scripted_text.rs](src/scripted_text.rs), [effect_description.rs](src/effect_description.rs) | Scoped localisation and native effect descriptions, sharing the trigger/variable evaluator. |
| [modifier_text.rs](src/modifier_text.rs), [faction_names.rs](src/faction_names.rs), [unit_names.rs](src/unit_names.rs) | Shared modifier formatting and native stored/generated names. |
| [cryptology.rs](src/cryptology.rs), [naval_battle_name.rs](src/naval_battle_name.rs), [state_resources.rs](src/state_resources.rs) | Native helpers shared by products and viewer tooltips. |
| [content_check.rs](src/content_check.rs) | Offline preparation and diagnostics for every registered product. |
| [service.rs](src/service.rs), [service/](src/service/) | Source leases, loader jobs, request admission and the bounded response pump. |
| [transport.rs](src/transport.rs), [protocol.rs](src/protocol.rs) | Generic data requests, fragmented replies and host/viewer control messages. |
| [config.rs](src/config.rs), [content.rs](src/content.rs), [map_settings.rs](src/map_settings.rs) | Game folders, content verification/cache keys and map preferences. |
| [dto.rs](src/dto.rs), [lib.rs](src/lib.rs) | Shared wire types, public facade and TypeScript contract assembly. |
| [bin/dashboard-bridge.rs](src/bin/dashboard-bridge.rs) | Development stdio adapter for browser UI work. |
| [bin/export-types.rs](src/bin/export-types.rs) | Generates the frontend contract. |

## Products

A `Product` turns a `Source` plus typed parameters into a typed reply. History
products cover a whole branch, so playback is a local lookup. Hour-specific
artwork and formula details use separate transient requests; explicitly windowed
products reuse a branch scan rather than decoding it for each window.

Each product defines a unique `KEY`, `CONTENT_DEPENDENT`, `Params`, `Reply` and
`load`; `validate` checks parameters before loading. Parameters deserialize and
export through `TS`; replies serialize and export through `TS` and implement
`ReplyWarnings`. Override `types()` when nested wire types need declarations.
Use the shared parameter and selection validators in `products/mod.rs`.

`CONTENT_DEPENDENT` is true when any reply field reads localisation, artwork or
content definitions. The generated `CONTENT_PRODUCTS` list controls frontend
content-cache invalidation. Service, transport and host route by key; widgets
and their data helpers choose products. The shared serialization boundary adds
current uncovered-content-read warnings through `ReplyWarnings`, including for
cached replies. Preserve independent known values when a name or image refuses.

Keep caches in `source.state::<YourState>()`. Share scans between consumers of
the same inputs instead of decoding a branch again. Numeric history and
content-dependent presentation can have separate cache keys. State is revision
local; append inheritance is an explicit `append_state` opt-in after the source
verifies the recorded prefix. Its clone shares immutable history and gives the
new revision an independent mutable continuation. Cache eviction changes reuse,
never the completeness of a reply.

### Add a product

1. Add `products/<name>.rs` implementing `Product`, with validation, cancellation
   checks and the complete-reply budget. Reuse shared history/content helpers.
2. Export the module and register it once in `PRODUCTS` in `products/mod.rs`.
   Implement `ReplyWarnings` for its reply, normally through `reply_warnings!`.
3. Add its dispatch in `content_check.rs`, reusing the actual naming/artwork
   loaders. State which inputs require a recording, even for recording-only products.
4. Export its nested wire types through `types()` and regenerate the contract.
5. Add/register the widget under `ui/src/widgets/`, following the
   [widget guide](../../ui/src/widgets/README.md). Put product-specific behaviour
   in its code or widget README, then run the affected checks below.

### Exact and derived peace outcomes

See [peace-deals.rs](src/products/peace_deals.rs) and the
[Territory transfers guide](../../ui/src/widgets/territory-transfers/README.md).

## Recorded histories

Preserve exact recorded values and distinguish zero, absent rows, unavailable
reads and values that cannot be computed. `read_runs` tracks known coverage;
outside it, an absent run cannot establish zero. Gaps and identity changes end
continuity. Never backfill a missing hour from a later observation.

A held daily value retains its **supplying read hour**. Use `metadata::slot_read`
and check table completeness; a held table or an unchanged value is not evidence
of another successful read. Keep read confirmations separate from numeric
changes. Carry values only over the coverage allowed by the supplying table.
Cadence belongs to [replay](../replay/README.md), not a product's private schedule.
Keep native event dates, observation dates and publication/read dates distinct;
first/last sightings establish observed bounds, not an invented exact start or end.

Tags are opaque lookup keys, not paths or a fixed spelling format. Resolve raw
country references through recorded CountryLinks. Native identity is
`(canonical slot, country_id)`; a slot or tag alone is insufficient. Shared
country-history identity runs let manual picks follow the holders recorded
under selected tags through renames and aliases, without including unrelated
later holders of an alias. An origin-hour selection binds the holder at that
hour; smart groups use identities at their membership dates. Ambiguous links,
missing hours and a replacement holder break continuity; do not infer identities
from tag-change events or bridge a gap.

Metadata's country catalogue is an initial observation, not the campaign census.
Initial selection uses that frame's LocalPlayer, including observer choice,
joined through CountryLinks; the multiplayer human roster does not identify the
recording client. A terminal partial marker is separate from branch gaps: an
unknown failed hour supplies no inferred gap. See
[replay compatibility and recovery](../replay/README.md#compatibility-and-recovery)
for readable prefixes and field availability. Refuse only dependent fields that
were not recorded when the rest of an older recording remains usable.

## Content and text

Resolve content through the recording's stack and the shared `catalogue`;
localisation, native GUI/sprite lookup and texture decoding must not acquire
parallel parsers. The [installed-content contract](../installed-content/README.md) owns content
verification and uncovered-read reporting. Keep its named, counted diagnostics
visible after lazy reads. Missing artwork leaves independent text and values
available with its reason.

Use the shared localiser and plain-text helpers. Stored display text without a
proven localisation key keeps the shared recording-language guard; a literal
never becomes a key. Scoped text, triggers and descriptions use the same
recorded-hour evaluator and caller-owned event graph/variable session. Missing
operands and unsupported reached forms retain named, counted refusals, without
invented scopes, roster order or rebuilt native caches. Native evidence and exact
form handling belong beside those helpers.

`content_check::check` visits products in registry order, shares preparation per
stack and calls their real loaders. Offline findings belong to that preparation,
not later live replies. Preserve producer diagnostic kinds: `Content` requires
cited native parity, while `Offline` identifies a recording dependency. Keep
counts/examples separate from diagnostic identity and avoid duplicating
reader-owned notes in product checks.

## Service and limits

`Service` shares sources by canonical path and revision and issues independently
releasable leases; `open_map` uses the same sources. An outdated expected revision
is refused. A new revision uses a separate source and caches. Every product reply
echoes the requesting lease, revision and key so it cannot answer another recording.

Loader jobs run off rendering. Ordinary product caches serialize their own
requests; scans with their own gates do expensive work outside short cache locks.
Open/append work stays outside the global sources lock. Check cancellation during
reads, scans and serialization, and never publish a cancelled partial result.
Different products and sources can run in parallel; replies are fragmented and bounded.

Measure complete product replies against `products::MAX_PRODUCT_REPLY_BYTES`
with `transport::json_len` (or the shared budget helper), including identity,
read-date and warning metadata. Refuse oversized replies explicitly; never
truncate countries, entries or runs to fit. Preflight large allocations as well
as checking final serialized size. Request envelopes share
`protocol::MAX_MESSAGE_BYTES` across the desktop host, native reader and browser
bridge. These are byte/retention budgets, not chosen country limits: size buffers
from recorded data, design for 1,500 countries and verify at 5,000 synthetic countries.

Capacity errors use `transport::TRANSIENT_ERROR_NAMES` and its shared classifier;
the generated frontend contract exports the same names. They decide no country
fact and can be retried. Other errors remain failures until an explicit retry or
content/configuration change, never successful empty data.

## Check a change

Follow [AGENTS.md](../../AGENTS.md) for the build wrapper and
crate-scoped formatting, clippy and affected synthetic tests. Exercise changed
identity, gap/read-date, mod-content and complete-reply-budget behaviour at the
required scale. Capture integration also needs the relevant fresh-save oracle
and smoke workflow in [recorder](../recorder/README.md).

Rust owns [ui/src/generated/dashboard.ts](../../ui/src/generated/dashboard.ts);
do not hand-edit it. From the worktree root, generate changed types and check
that the committed contract matches (every cargo invocation uses the wrapper):

```bash
bash tools/build-slot.sh "dev/<task>" "cargo run -q --profile test -p dashboard --bin export-types"
bash tools/build-slot.sh "dev/<task>" "cargo run -q --profile test -p dashboard --bin export-types -- --check"
```

UI changes also need `npm.cmd run check:ui` and `npm.cmd run test:ui`, as described
in [ui](../../ui/README.md). Documentation-only changes need link and diff checks.
