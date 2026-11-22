import type { CountryAt, Gate } from '../../dashboard/regionGate';
import type { SeriesGate } from '../../dashboard/keyedSeries';
import { useMemo, useState } from 'react';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../../charts/drill/DrillParts';
import { exactRuns } from '../../charts/chartSummary';
import type { ChartSummary } from '../../charts/chartSummary';
import { mapLive } from '../../charts/live';
import type { Live, LiveValues } from '../../charts/live';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import type { PlaybackMode } from '../../charts/view';
import type { ChartTokens } from '../../charts/tokens';
import { fixedNumber, formatAmount, formatSigned } from '../../charts/valueAxis';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { keyId } from '../../dashboard/keyedSeriesData';
import type { ReachedKeyLine } from '../../dashboard/keyedSeriesData';
import { ownerKeys, useKeyedSeries } from '../../dashboard/keyedSeries';
import { mostNamed } from '../../dashboard/tiles/view.ts';
import type { ResourceBalanceReply, ResourceInfo } from '../../generated/dashboard';
import ResourceDeals from './ResourceDealsView';
import { RESOURCE_BALANCE, RESOURCE_PARTS, balanceOf, poolsAt, poolsMissing, resourceName, resourcePartGroups, resourceParts } from './resourceData';

const family = (measure: string) => ({ kind: RESOURCE_BALANCE, measure });
const PART_FAMILIES = RESOURCE_PARTS.map(part => family(part.measure));
const GROUPS = ['Supply', 'Demand'] as const;
const GROUP_PARTS: readonly (readonly string[])[] = [['produced', 'subjects', 'imported'], ['projects', 'production', 'overlord', 'exported']];

function partsAt(values: LiveValues<number>, id: number, decimals: number, colours: ReadonlyMap<string, string>) {
  const pools = poolsAt(values, id);
  if (!pools) return null;
  const balance = balanceOf(pools), other = resourceParts(balance).at(-1)!.raw;
  const groups = resourcePartGroups(balance).map(group => ({ title: group.title, total: group.total,
    parts: group.values.map((part): DrillPart => ({ id: part.id, label: part.label, color: colours.get(part.id),
      value: fixedNumber(part.raw, decimals), raw: part.raw, text: formatAmount(part.raw, decimals, decimals) })) }));
  return { balance, groups, other };
}

export default function ResourceBreakdown({ runtime, gate, track, active, history, resource, data, live, hour, following, tokens, extent, playhead,
  liveHead, playback, afterPlayhead, windowHours, hoursPerSecond, onPick, onContextMenu }: {
  runtime: WidgetRuntime; gate: SeriesGate & Pick<Gate, 'recorded'>; track: BoundTrack; active: boolean; history: CountryAt['history']; resource: ResourceInfo; data: ResourceBalanceReply;
  live: Live<LiveValues<number>>; hour: number | null; following: boolean; tokens: ChartTokens; extent: [number, number]; playhead: number | null;
  liveHead?: { read(): number; subscribe(changed: () => void): () => void }; playback: PlaybackMode; afterPlayhead: boolean; windowHours: number;
  hoursPerSecond: number | undefined; onPick: (hour: number) => void; onContextMenu: (hour: number, x: number, y: number) => void;
}) {
  const decimals = data.decimals;
  const colours = useMemo(() => new Map(RESOURCE_PARTS.map((part, index) => [part.id as string, tokens.categories[index % tokens.categories.length]])), [tokens.categories]);
  const at = useMemo(() => partsAt(live.read(), resource.id, decimals, colours), [live, resource.id, decimals, colours, data]);
  const liveGroups = useMemo(() => GROUPS.map((_, index): LiveDrillParts => mapLive(live, values => {
    const now = partsAt(values, resource.id, decimals, colours)?.groups[index];
    return now ? { parts: now.parts, total: fixedNumber(now.total, decimals), totalText: formatAmount(now.total, decimals, decimals) }
      : { parts: [], total: null, totalText: poolsMissing(values, resource.id) };
  })), [live, resource.id, decimals, colours]);
  const texts = useMemo<LiveTexts>(() => {
    const read = () => {
      const now = partsAt(live.read(), resource.id, decimals, colours);
      return new Map([['net', now ? `Net: ${formatSigned(now.balance.net, decimals, decimals)} per day.` : poolsMissing(live.read(), resource.id)],
        ['other', now && now.other !== 0n ? `Other (the game's rounding): ${formatSigned(now.other, decimals, decimals)} per day.` : '']]);
    };
    return { text: key => read().get(key) ?? '', subscribe: changed => live.subscribe(changed) };
  }, [live, resource.id, decimals, colours]);
  const deals = useMemo(() => data.dealsMissing !== null ? null : {
    import: data.deals.filter(deal => deal.resource === resource.id && deal.direction === 'import'),
    export: data.deals.filter(deal => deal.resource === resource.id && deal.direction === 'export'),
  }, [data, resource.id]);

  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [root, setRoot] = useState<HTMLDivElement | null>(null);
  const element = useMemo(() => ({ current: root }), [root]);
  const { keys, within } = useMemo(() => ownerKeys(history, [String(resource.id)]), [history, resource.id]);
  const spec = (index: number) => ({ source: track.source, branch: track.branch, family: PART_FAMILIES[index], keys, within, partBits: 32, lastHour: track.branch.lastHour });
  const head = following ? null : gate.recorded(hour);
  const hovered = gate.recorded(hoverHour);
  const series = [
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(0), head, hovered, following, active, element),
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(1), head, hovered, following, active, element),
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(2), head, hovered, following, active, element),
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(3), head, hovered, following, active, element),
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(4), head, hovered, following, active, element),
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(5), head, hovered, following, active, element),
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(6), head, hovered, following, active, element),
    useKeyedSeries(runtime.client, runtime.clock, gate, spec(7), head, hovered, following, active, element),
  ];
  const id = keys[0] ? keyId(keys[0]) : '';
  const summaries = series.map(item => item.summaryByKey.get(id)?.summary);
  const lines = series.map(item => item.lines[0] as ReachedKeyLine | undefined);
  const chartSeries = useMemo<BreakdownSeries[]>(() => RESOURCE_PARTS.map((part, index) => ({
    id: part.id, label: part.label, color: colours.get(part.id)!, runs: (summaries[index] && exactRuns(summaries[index]!)) ?? [],
    amount: at => {
      const run = lines[index]?.runs.find(item => item.firstHour <= at && at <= item.lastHour);
      return run ? formatSigned(run.valueRaw, decimals, decimals) : lines[index]?.missing.get(at) ?? 'Not recorded';
    },
  })), [...summaries, ...lines, colours, decimals]);
  const drawn = summaries.every(Boolean) ? { parts: summaries as ChartSummary[] } : undefined;

  return <div ref={setRoot}>
    {GROUPS.map((title, index) => {
      const group = at?.groups[index];
      return <DrillParts key={title} live={liveGroups[index]} format={String}
        parts={group?.parts ?? RESOURCE_PARTS.filter(part => GROUP_PARTS[index].includes(part.id))
          .map(part => ({ id: part.id, label: part.label, value: null, color: colours.get(part.id) }))}
        total={group ? fixedNumber(group.total, decimals) : null}
        totalRow={{ label: title, value: group ? formatAmount(group.total, decimals, decimals) : '—' }} />;
    })}
    <p className="drill-empty"><LiveText live={texts} at="other" /></p>
    <p className="drill-empty"><LiveText live={texts} at="net" /></p>
    {deals && hour !== null ? (['imports', 'exports'] as const).map(direction =>
      <ResourceDeals key={direction} deals={deals[direction === 'imports' ? 'import' : 'export']} live={live} resource={resource.id}
        decimals={decimals} hour={hour} direction={direction} runtime={runtime} track={track} active={active} />)
      : <p className="drill-empty">{data.dealsMissing ?? 'Trade partners were not recorded at this hour.'}</p>}
    <BreakdownChart series={chartSeries} summaries={drawn} selectedHour={hour} label={`${resourceName(resource)} parts over time`}
      extent={extent} playhead={playhead} liveHead={liveHead} playback={playback} afterPlayhead={afterPlayhead} windowHours={windowHours}
      hoursPerSecond={hoursPerSecond} branchStart={track.branch.firstHour} active={active} pickOwner={series[0].owner}
      notRecorded={series.every(item => item.unrecorded) ? mostNamed(series.map(item => item.unrecorded!)) : null}
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      onSummaryView={view => { for (const item of series) item.onSummaryView(view); }} onHover={setHoverHour}
      onNavigateHour={series[0].onNavigateHour} keyboardHour={series[0].keyboardHour} keyboardRevision={series[0].keyboardRevision}
      onPick={at => onPick(Math.floor(at))} onContextMenu={onContextMenu} />
  </div>;
}
