# weather/ - live WEATHER / environment layer (build 16022851)

> STATUS: **NOT PINNED / LOW confidence.** Weather is the only live layer with NO
> save oracle (weather is dynamic and not persisted in saves). No specific weather
> array address is confirmed. See `../../RESEARCH_weather.md` for the full method,
> dead ends, and residual.

## Files
- `weather_reader.py` - the read-only validation FUNNEL: build-gate, climatology +
  live-month, JOINT array-of-structs scan over the 304 strategic regions
  (temp-in-band AND phenomenon-possible-this-month, with SOFT physical penalties),
  shuffled-rid + wrong-month negative controls, and a temporal-drift gate
  (the decisive test - requires the game UNPAUSED; stub). Fails closed.
- `weather_offsets.draft.json` - semantics, phenomenon enum, method, candidate
  status, residual. NOT validated to the 1081/1081 standard of the other layers.
- `weather_climatology.json` - per-region/month temperature band + the set of
  phenomena climatologically possible that month (the validation ground truth;
  built from `map/strategicregions/*.txt`).

## Why this layer is different
The owner/control/economy/division/combat layers were anchored against a
time-aligned melted save (1081/1081, 1603/1603). Weather is NEVER written to a
save, so there is no oracle. Validation relies on the static climatology, internal
consistency, negative controls, and - the only strong discriminator - GAME-TIME-
COUPLED TEMPORAL DRIFT, which needs the simulation running. This session was
paused, so the drift gate could not run.

## Run
    python weather_reader.py                 # auto: read month, scan, report
    python weather_reader.py --month 8 --budget 120
The session this was built in was paused at ~1937.9 (September). A single-field
temperature scan found 4.66M false positives; the joint+control scan rejects them
(0 control-beating candidates at stride 16) but a confirmed positive needs an
unpaused session for the drift gate. READ-ONLY, arena-scoped (0x1F6..0x1FA).
