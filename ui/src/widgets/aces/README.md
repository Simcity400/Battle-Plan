# Aces

**What it shows:** Every country's recorded hall of fame at the shown moment, with portraits, callsigns, assigned aircraft and bounded first-sighting/death dates.

**Product(s) it reads:** [`air-aces`](../../../../crates/dashboard/src/products/air_aces.rs) supplies the manifest and recording-block roster windows; `air-ace-portraits` in the same module supplies artwork. Country filtering uses [`country-groups`](../../../../crates/dashboard/src/products/country_groups.rs).

**Settings:** All countries by default, with manual and smart-group filters; newest-first or name sorting; Campaign, Follow or Pin.

**Notable behaviour:**

- Creation is **Ace by** the first sighting. **Killed by** requires an alive read followed by a dead read; an ace first seen dead has no known death date. Jump uses those observation bounds.
- Names, flags, aircraft and killers use the supplying read. Unknown owners appear only in All countries; pending/refused flags retain their reasons. Killed portraits grey out.
- Manual tags select their recorded native holders and those holders' aliases, excluding unrelated alias holders and identity gaps. Pending identity history stays loading.
- Windows retain the supplying dates of carried rows. While a new window loads, the previous roster remains at its own hour and cannot survive a client/recording change. Portrait selection resolves through recorded content; no artwork is invented.

**Files:** [AcesWidget.tsx](AcesWidget.tsx) loads windows and renders the shared table; [acesData.ts](acesData.ts) validates and projects rows; `acesData.test.ts` covers those rules. `aces.css` owns local layout; `definition.ts` and `index.ts` declare settings and registration.

Shared framework conventions: [widgets](../README.md).
