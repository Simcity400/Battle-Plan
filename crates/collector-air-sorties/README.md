# collector-air-sorties

Air-base registry witnesses include every extant wing's pool base and transfer base, including selected missions without a region or registration. The existing worker deduplicates the handles before reading them; selected-wing range symbols share those witnesses.

Air sortie capture: the sortie inputs and AirReferenceWitnesses that the AirMissions job reads beside the registered missions and the wing census. The host owns the AirMissions job, the air pack loan and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-air-sorties` and `cargo clippy -p collector-air-sorties --all-targets -- -D warnings` for its synthetic checks.

Air mission inputs also carry the interception alert's exact trace-count predicate. The existing registered-mission batch reads four bytes into unused packet padding and publishes only whether the retained base-to-region trace contains more than one entry. It adds no scan, allocation, hook or synchronous hourly work; a refused field leaves this predicate independently unavailable.

Sortie acquisition can offer each successfully authenticated country-region object, including a successful lookup miss, to another consumer in the same worker capture. This shares the existing lookup without coupling its efficiency result to Ground Crews flag availability.

AirMissionInputs carries current execution and effective aircraft independently of selected missions and scenario efficiency. Production forwards the existing wing worker's owned fuel execution packet, preserving its optional failures and avoiding duplicate mask/count acquisition. The ordinary reader demands the count only for a nonzero current mask.

The current execution subset carries raw normalization operands for nonzero effective aircraft. Independent field failures preserve the remaining mission and normalization fields. Zero execution/count demand no normalization reads.

AirMissionInputs.detection_stat_13 and detection_stat_14 are independent numeric raw aircraft-detection columns for Navy patrol spotting. Their positive one-bit, nonzero-count executions share normalization's selected cached record; stable values carry in wire deltas independently of changing aircraft counts or normalization packets.

AirSortieModifiers also publishes the certified console detection vector and an explicit availability witness supplied by its host observer. A failed certification drops partial override rows without invalidating ordinary sortie terms. The host reads this persistent simulation-console state only first/on-change under its sole writer.

The same registered-mission worker projects six independent numeric disruption columns. Raw disruption is read only for applicable current execution; zero disruption, the native bypass kinds and nonpositive counts demand no combat subtree. Demanded attack/defence reuse the selected normalization record; default speed and two narrowly probed raw wing combat terms remain independent acquisition leaves.

A seventh numeric patrol_night_applied_raw column supplies held sampled clocks' last-applied factor. It reuses the existing weather manager authentication and owned order scratch, deduplicating demanded region samples. The updated gate avoids all marker acquisition; a held marker fault leaves other mission facts readable. A held wire clock never interprets a missing marker as updated.

The registered-mission pass shares one loaded spotting-factor cache across all64-mission batches. Repeated kinds, including a failed observation, reuse the same original result. Known zero/out-of-range factors omit all new patrol leaves; unknown factors leave them unavailable. Ordinary Air normalization continues independently. The factor itself comes from recorded content and adds no wire column.
