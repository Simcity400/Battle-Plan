import { useMemo } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { CountryStabilityReply, HourSpan } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { usePreparedMeasure } from '../country-measure/preparedMeasure';

import { scaleSummary } from '../country-measure/preparedMeasureData';
export const percentSummary = (summary: ChartSummary) => scaleSummary(summary, 100);

/** Exact reached hours supply values and read dates; summaries supply geometry. */
export function usePreparedStability(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  tags: readonly string[], head: number, hover: number | null, active: boolean, shown: readonly string[] = ['stability', 'warSupport'], within?: ReadonlyMap<string, readonly HourSpan[]>) {
  const stability = usePreparedMeasure(client, source, branchId, 'stability', tags, head, hover, active, false, 'country-stability', shown, within);
  const warSupport = usePreparedMeasure(client, source, branchId, 'warSupport', tags, head, hover, active, false, 'country-stability', shown, within);
  const data = useMemo<CountryStabilityReply | undefined>(() => stability.states && warSupport.states ? {
    branchId: branchId!, decimals: 5, warnings: [...new Set([...stability.states.warnings, ...warSupport.states.warnings])], series: tags.map(tag => {
      const states = [...stability.states!.states, ...warSupport.states!.states].filter(state => state.tag === tag);
      const reads = new Map(states.flatMap(state => state.readHour != null && state.readHour !== state.hour
        ? [[state.hour, { firstHour: state.hour, lastHour: state.hour, readHour: state.readHour! }] as const] : []));
      return { tag, stability: stability.data?.series.find(row => row.tag === tag)?.runs ?? [], warSupport: warSupport.data?.series.find(row => row.tag === tag)?.runs ?? [],
        maxCommandPower: [], carried: [...reads.values()].sort((a, b) => a.firstHour - b.firstHour), recorded: states.some(state => state.recorded), breakdown: null };
    }),
  } : undefined, [stability.states, warSupport.states, branchId, tags, stability.data, warSupport.data]);
  return { onNavigateHour: stability.onNavigateHour, keyboardHour: stability.keyboardHour, keyboardRevision: stability.keyboardRevision,
    hourLoaded: (tag: string, hour: number) => stability.hourLoaded(tag, hour) && warSupport.hourLoaded(tag, hour),
    enabled: stability.enabled, owner: stability.owner, data, stability, warSupport,
    retry: () => { stability.retry(); warSupport.retry(); }, error: stability.error ?? warSupport.error,
    onSummaryView: (view: Parameters<typeof stability.onSummaryView>[0]) => { stability.onSummaryView(view); warSupport.onSummaryView(view); } };
}
