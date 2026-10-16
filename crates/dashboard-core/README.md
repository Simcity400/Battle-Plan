# dashboard-core

The dashboard's shared contract: the host/viewer protocol, the data transport,
the recording metadata replies, configuration, map settings, content findings
and the block scanner. Nothing here names a product; the
[dashboard](../dashboard/README.md) builds its sources, products and service on
these types and re-exports each module at its old path.

| Module | What it does |
|---|---|
| [protocol.rs](src/protocol.rs) | Native control messages between the desktop host and the viewer (including the full map's camera mode with its overlay choice both ways, and forwarded controller state), map options and the requested-tag check. |
| [transport.rs](src/transport.rs) | Data commands and events, preparation actions, bounded JSON replies and their fragmentation, and the transient refusals a reader may retry. |
| [dto.rs](src/dto.rs) | Recording metadata (branches, gaps, partial and damaged recordings, countries) and token artwork replies. |
| [config.rs](src/config.rs) | The host's saved game/user folders and preparation choice (`APPDATA/battle-plan/content.json`, written under a lock), and the content options a product load receives. |
| [map_display.rs](src/map_display.rs) | Every map choice and show/hide switch, the map volumes and the counter filter, each resolving on its own through the HOI4, global, campaign and widget scopes. |
| [map_settings.rs](src/map_settings.rs) | The stored global, campaign and widget map preferences and the game's own graphics and volume settings. |
| [map_pairs.rs](src/map_pairs.rs) | Serde helpers that persist hash maps and sets as key-ordered pair arrays, maps and sets. |
| [findings.rs](src/findings.rs) | Content findings by kind, as readers, the catalogue and product checks record them. |
| [blocks.rs](src/blocks.rs) | Whole-branch scans with their blocks read ahead on spare cores, in order. |
| [test_fixtures.rs](src/test_fixtures.rs) | Recording, session and decoded-frame builders for tests. |

The `test-support` feature exposes `test_fixtures` and the scanned-block log
(`blocks::SCANNED_BLOCKS`) to the tests of crates above this one.
