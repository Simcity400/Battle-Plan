# collector-weather

Weather capture: province and region weather records, the window-following carry that reads only the records the native hourly pass visited, region combat modifiers, every province record's whole modifier accumulator ([entries.rs](src/weather/entries.rs), packed in one buffer and carried with its record) and the weather display. The country input family in collector-observers counts the weather writers that explain a difference and reports the carry's diagnostics; the host owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-weather` and `cargo clippy -p collector-weather --all-targets -- -D warnings` for its synthetic checks.

WeatherRegions also projects its raw Air detection entry from the same complete region-accumulator read. The native rotating window and sampled carry verification govern this entry together with ground-combat terms; it adds no native read, hook or periodic work. Outer absence refuses the accumulator; inner absence is a known missing entry.

WeatherProvinces slot zero carries the Air fuel forecast's day/night direction. Clear update gates certify direction and feathering from the hour and recorded content, with no added native reads. Suspended or unread gates acquire only the held direction and two feather words inside the existing worker pass. Those optional failures leave ordinary weather rows intact; the wire suffix changes only with its operands. No province forecast vector is captured.

WeatherRegions projects raw naval detection and Air attack/defence map terms from that same complete accumulator read, with no additional native bytes or lookups. One known-read gate distinguishes a refused observation from absent keys; independent numeric columns delta separately. The existing rotating-window carry and sampled verification cover all terms.
