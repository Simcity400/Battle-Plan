# 0903. Keep table identity, framing and admission independent

The table registry binds a table's wire ID, row type and operation descriptor. Wire IDs are positions in the original registry. Append new tables after the entire registry, including the derived entries, and keep retired IDs as tombstones. Playback indexes the tables densely without relying on a key's wire width. The registry golden records the complete ordered ID, name, key width and field sequence. The test compares it line by line because a Windows checkout can provide CRLF.

The compatibility catalogue records the first proven format for each table, nested record and framing kind. Its audited baseline is #302, with #323, #325, #328, #341 and #343 checked against the baseline through row and typed codec changes. A table established at the proven horizon can be older. Changes below the horizon do not participate in reader admission. Table lifetimes and frame framing versions advance independently. Common catalogue passes must retain a later field, layout or attribution that another pass already declared.

Each historical read uses its own recorded field count and nested format. A field appended in an admissible layout is missing in older captures. A raw or opaque suffix that has no missing representation requires whole-layout admission. Whole-field tombstones retain their original framing, and a nested retirement is interpreted using the nested record's source format. Skipping an incompatible row checks only the framing already proven for that format, including opaque byte lengths, without constructing today's row from it. Retired tables keep their framing for discard and are reported by name. File-level framing failure withholds the entire affected block.

## Historical cases

- The initial exact combat scalar aliases and province and factory-efficiency aliases were superseded by private-copy aliases without keeping their public markers. Private display aliases keep a literal value when there is no recorded exact demand. An alias can leave a table's prefix unchanged while a later daily cadence changes its meaning.
- The supply footer is recorded at display precision. Published formats 99–101 stored exact amounts and still decode unscaled. Two provisional format-102 cases held the former tombstone's empty payload or exact amounts before the retirement. The former is missing. The latter is scaled to native units at the specified step. Neither provisional encoding shipped. The restored field can re-emit a historical tombstone byte for byte when a reader works through finalization. An ordinary missing field still cannot be written as a restored tombstone.
- The nested naval discriminator for a native null preserves the historical unavailable/list boundary. The mission layout includes its raw spotting suffix, and shared immutable headquarters membership keeps the owned-value framing.
- ResearchTechnologies changed key meaning at the same key width. The historical five-field payload still has enough framing to skip safely. Wing naming changed the meaning of an existing packed family. Movement terms added army and attack speed to the `combat` payload.
- Directed wars switched to derivation from the hour's WarRelations in provisional format 102. Captures with a witnessed packet still answer from it when their layout admits it. Withholding that field leaves the containing table's framing readable.
- Format 135 appends the raid air menu and the AirCombatRecord equipment suffix. The older RaidAirScores layout predating format 104 is still refused. Formats 137 and 142 append fields without moving layout refusal boundaries. Hourly hosts that stopped writing the read hour keep their table framing and interpret the changed nested packet using its source format.
- The split reserved transport tables and nearest-cache tables were introduced and retired within their provisional formats. No published build wrote them. Older arrival-time, attrition, selected-attack and transport tables had actual payloads, which the catalogue can discard by name. The retired hover modifier tables and `OwnedWargoals` were retained only for tooltip paths. AirRegionCombat combines the retired air-region history and damage sources.
- LandCombatDivisionModifiers keeps ordered entries per combat, side and member. The old per-side member list retains its opaque framing as a tombstone. Older combat side operands used for CAS keep their historical anti-air and prevented lanes. Current encodings read organisation loss from the sum, using the same wrapping addition as the 24 native samples. The marked committed count distinguishes the sum layout from the historical count and sample sequence.
- EquipmentDestruction's signed net amounts retain historical positive values. Missing appended fields remain missing. The equipment-loss log variants that can rewrite rows or insert a key a second time remain state.

## Complete rows and updates

An append-only log row is complete on first publication, and its key appears once in a run. The recorder can track a count per log. A forgotten entry is carried by the log's own stream. The observed 13-year fixture separated true logs from state by writes after insertion, row changes and reinserts. The measures below retain the evidence supporting the state entries. LandEventCoverage, MapEvents and EventFeedHistory overwrite their single row each hour.

| Table | Later cell writes | Changed rows | Reinserted keys | First changed field |
| --- | ---: | ---: | ---: | --- |
| CombatLogState | 1 | 1 | 0 | `prune_observed` |
| CombatLogGroups | 8,206 | 6,454 | 14,673 | `ordinal` |
| CombatLogMonths | 184,520 | 18,046 | 17,520 | `date` |
| CombatLogEquipment | 1,579,398 | 294,471 | 574,318 | `amount_raw` |
| CombatLogEnemyEquipment | 5,913,172 | 759,492 | 328,002 | `amount_raw` |
| CombatLogManpower | 300,566 | 10,326 | 2,964 | `losses_0` |
| CombatLogTemplates | 718,227 | 8,316 | 3,239 | `wins` |
| CombatLogReferences | 9,361,480 | 37,427 | 22,052 | `battle` |
| CombatLogBattles | 44,099 | 7,760 | 14,055 | `reference_count` |
| CombatLogBattleSides | 361 | 130 | 28,110 | |
| CombatLogBattleEquipment | 7,617 | 6,312 | 1,143,638 | |
| CombatLogBattleCountries | 101 | 100 | 32,261 | |
| EquipmentDestruction | 469,520 | 1,611 | 2,026,233 | |
| CountryExperienceGains | 2,514,233 | 447 | 1 | `complete` |
| DivisionEquipmentDeletions | 0 | 0 | 40 | |

Province victory points retain their position after the three legacy owner, controller and state words. The witnesses for attack operands can be absent in current rows without losing the recorded operands that an older row held. Area labels, null areas, refused labels or operands, and a complete empty contact list remain distinct on the wire. Battle sound snapshots retain coordinate bits, repeated members, and their order so a movement can change only its affected fields. The historical six-field weather fixture independently records snow, permanent snow, two display bytes, temperature at the row's hour and raw mud in little-endian order.

A row's codec owns its field order and missing mask. The table declares top-level field gaps, and each nested value type declares its own gaps. Inspection borrows typed rows through the registry. Wire walkers check counts, masks and framing without decoding row values. Only canonical delta encodings are admitted, so re-encoding a read yields the same bytes. Validating borrowed changes checks their recorded fields before reconstruction performs typed nested operations and baseline arithmetic. The encoder validates replacement types, indices and unchanged keys before any table can change its baseline. Installation returns the displaced rows so the writer can recycle their storage. Full row enumerations share unwind cleanup across variants, which avoids expanding every pairing.

For a table known complete outside a supplied key set, the restricted encoder visits only ascending changed rows and keys, and compares equal rows exactly as the whole-table encoder does. Its notes of encoder-written spans are canonical by construction. Reconstruction may borrow those spans without reparsing them. The regression fixtures compare resulting bytes, deletions and decoded changes against the full-table path and check that an out-of-domain row is refused.

Historical goldens assemble the bytes independently of today's encoders. In the published 18-field arrival snapshot, the opaque fields at positions 5 and 14 through 16 have a byte-length envelope separate from the vector element count. The scalar sequence begins with speed modifier, base, optional organisation, optional fuel and current speed. After the terrain vector come fort, snow, river and leader terms, class, three optional rail terms, the three opaque lists and the sample hour. This fixture establishes enough framing to discard the retired rows. The division status mask cost test isolates the third mask byte across 24 synthetic hours while preserving every other encoded byte. Sparse loss fixtures exercise synthetic country counts beyond vanilla.

Readers communicate named refusals using a plain-language table or field name. A field reason includes its owning table in the same terms.

[Registry and updates](../../crates/replay-data/src/rows.rs) · [Compatibility catalogue](../../crates/replay-data/src/rows/compatibility.rs) · [Restricted encoder](../../crates/replay-data/src/rows/restricted_changes.rs)
