import { runAt, stretchAt, stretchStarts } from '../../charts/series.ts';
import type { Run } from '../../charts/series';
import { seriesAt } from '../../charts/race/series.ts';
import type { raceSeriesIndex } from '../../charts/race/series';
import { shareReading } from '../../charts/sunburst/share.ts';
import type { SunburstKey } from '../../charts/sunburst/layout';
import type { FactionTotalsReply, TerritoryMeasure, TerritoryReply, TerritoryRule } from '../../generated/dashboard';
import { territoryColumn } from './territoryData.ts';

/** World coverage comes from its own complete-table scan, not the selected series. */
export function territoryWorldModel(reply: TerritoryReply, measure: TerritoryMeasure, rule: TerritoryRule) {
  const column = territoryColumn(measure, rule);
  const runs: Run[] = reply.worldTotals.flatMap(run => run.values[column] == null ? []
    : [{ firstHour: run.firstHour, lastHour: run.lastHour, value: run.values[column], ...(run.startsSegment ? { breakBefore: true } : {}) }]);
  return { runs, starts: stretchStarts(runs) };
}

/** Faction proof covers even factions outside top N: an unknown total could have
 * been a leader, and fixed end/ever rosters can count one country more than once. */
export function territoryShareReading(series: ReturnType<typeof raceSeriesIndex>, world: ReturnType<typeof territoryWorldModel>,
  hour: number, count: number, look: (key: string) => SunburstKey, rest: SunburstKey,
  factions?: Pick<FactionTotalsReply, 'recorded' | 'disjoint'>, unavailableReason?: string | null, expectedKeys?: readonly string[]) {
  const at = Math.floor(hour);
  const total = runAt(world.runs, at)?.value ?? null;
  // Fixed choices must stay candidates even when their value is missing.
  const values = (expectedKeys ?? seriesAt(series, at)).map(key => ({ key, value: runAt(series.series.get(key)?.runs ?? [], at)?.value ?? null }));
  const recorded = !factions || runAt(factions.recorded, at) != null;
  const disjoint = !factions || runAt(factions.disjoint, at) != null;
  const reason = unavailableReason ?? (!recorded ? 'The whole faction share was not recorded at this hour.'
    : !disjoint ? 'Faction memberships overlap. Choose ‘At each hour’ or another view.' : null);
  const reading = shareReading(values, total, count, key => {
    const line = series.series.get(key)!;
    return { ...look(key), stretch: stretchAt(line.runs, line.starts, at) };
  }, rest, reason);
  if (unavailableReason == null && recorded && !disjoint) reading.missingLabel = 'Share unavailable';
  return { ...reading, stretch: reading.unavailableReason != null ? null
    : `${stretchAt(world.runs, world.starts, at)}:${factions ? runAt(factions.recorded, at)!.firstHour : ''}` };
}
