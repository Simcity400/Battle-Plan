import type { Gate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useRef } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { PlaybackController } from '../../dashboard/playback';
import type { ChartSummary } from '../../charts/chartSummary';
import type { LiveDrillParts } from '../../charts/drill/DrillParts';
import { ownerKeys, useKeyedSeries, useSeriesParts, type SeriesOwner } from '../../dashboard/keyedSeries';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import type { CoveredBranch } from '../../dashboard/regionTime';
import { useProducts } from '../../dashboard/products';
import { liveFollowed } from '../../dashboard/follow/liveFollowed.ts';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { measureOf, namesAt, partsAtStates, partsFromLines, validateStabilityPartNames } from './partsData';
import type { Measure } from './stabilityData';
import type { MeasureParts, StabilityPartName, StabilityPartNamesReply } from './types';
import { productRequest } from '../../dashboard/request.ts';

export const PARTS_FAMILY = { kind: 'stability-parts', measure: 'value' } as const;


export function useStabilityParts(client: DashboardClient, clock: PlaybackController, gate: Gate, source: DashboardSource | undefined, branch: (CoveredBranch & { id: string }) | undefined,
  owner: SeriesOwner | null, extent: readonly [number, number], lastHour: number, measure: Measure,
  hour: number | null, hover: number | null, following: boolean, active: boolean, root: { readonly current: Element | null }) {
  const branchId = branch?.id;
  const tag = owner?.key ?? null;
  const span = useMemo(() => gate.span(extent[0], extent[1]), [extent[0], extent[1], gate.span]);
  const listed = useSeriesParts(client, gate, source, branchId, PARTS_FAMILY, owner, span, active);
  const all = useMemo(() => (listed.data?.parts ?? []).map(part => part.part), [listed.data]);
  const namesRequests = useMemo(() => source && branchId && tag && span ? namesAt(listed.data?.parts ?? [], span.lastHour).flatMap(([at, parts]) => {
    const hour = gate.recorded(at);
    return hour === null ? [] : [productRequest({ sourceId: source.id, revision: source.revision, product: 'stability-part-names', params: { branchId, country: countryFromKey(tag), parts, hour } })];
  }) : [], [source, branchId, tag, listed.data, span, gate.recorded]);
  const namedAll = useProducts<StabilityPartNamesReply>(client, namesRequests, active, validateStabilityPartNames, source);
  const named = useMemo(() => ({ ...namedAll, data: namedAll.data && span && { tag: tag ?? '', hour: span.lastHour, parts: namedAll.data.flatMap(reply => reply.parts),
    warnings: [...new Set(namedAll.data.flatMap(reply => reply.warnings))] } }), [namedAll, tag, span]);
  const names = useMemo(() => new Map<string, StabilityPartName>((named.data?.parts ?? []).map(part => [part.part, part])), [named.data]);
  const parts = useMemo(() => all.filter(part => measureOf(part) === measure), [all, measure]);
  const { keys, within } = useMemo(() => ownerKeys(owner, parts), [owner, parts]);
  const spec = useMemo(() => source && branch && listed.data && keys.length ? { source, branch, family: PARTS_FAMILY, keys,
    partBits: listed.data.partBits, lastHour, within } : null,
  [source, branch, listed.data, keys, lastHour, within]);
  const series = useKeyedSeries(client, clock, gate, spec, following ? null : gate.recorded(hour, owner?.spans), gate.recorded(hover, owner?.spans), following, active, root);
  const lines = useMemo(() => new Map<string, ChartSummary | undefined>(keys.map(key => [key.part, series.summaryByKey.get(keyId(key))?.summary])),
    [keys, series.summaryByKey]);
  const geometry = useMemo<MeasureParts>(() => partsFromLines(parts, lines, names), [parts, lines, names]);
  const drawn = useRef<{ lines: unknown; head: number | null; parts: typeof parts; names: typeof names; made: MeasureParts } | null>(null);
  const geometryAt = useCallback((head: number | null): MeasureParts => {
    const newest = series.revision();
    const held = drawn.current;
    if (held && held.lines === newest && held.head === head && held.parts === parts && held.names === names) return held.made;
    const made = partsFromLines(parts, new Map(keys.map(key => [key.part, series.summaryOf(keyId(key))])), names, head);
    drawn.current = { lines: newest, head, parts, names, made };
    return made;
  }, [series.revision, series.summaryOf, keys, parts, names]);
  const states = series.states?.states;
  const at = useCallback((at: number): MeasureParts => partsAtStates(parts, (states ?? []).filter(state => state.tag === tag), Math.floor(at), names),
    [parts, states, tag, names]);
  const loadedAt = useCallback((at: number) => tag !== null && listed.data !== undefined
    && parts.every(part => (states ?? []).some(state => state.tag === tag && state.part === part && state.hour === Math.floor(at))), [parts, states, tag, listed.data]);
  const followed = series.followed;
  const live = useMemo(() => followed && tag ? (render: (values: MeasureParts) => ReturnType<LiveDrillParts['read']>): LiveDrillParts =>
    liveFollowed(followed.entry, entry => {
      const states = parts.flatMap(part => {
        const row = followed.row({ tag, part });
        const value = row < 0 ? null : rowAmount(entry.answer, row, 0);
        return value === null || value === undefined ? [] : [{ tag, part, hour: 0, drop: null, missing: null,
          run: { firstHour: 0, lastHour: 0, valueRaw: String(value), slot: 0, countryId: 0, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true } }];
      });
      return render(partsAtStates(parts, states, 0, names));
    }) : undefined, [followed, tag, parts, names]);
  return { enabled: !!spec, listed, named, names, parts, geometry, geometryAt, revision: series.revision, subscribe: series.subscribe,
    at, loadedAt, live, decimals: series.decimals, unrecorded: series.unrecorded,
    onSummaryView: series.onSummaryView, onNavigateHour: series.onNavigateHour, keyboardHour: series.keyboardHour, keyboardRevision: series.keyboardRevision,
    ready: !!named.data || all.length === 0, error: listed.error ?? named.error ?? series.error,
    retry: () => { listed.retry(); named.retry(); series.retry(); } };
}
