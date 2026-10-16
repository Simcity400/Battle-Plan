import { useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart } from '../../charts/drill/DrillParts';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import WindowedRows from '../../charts/WindowedRows';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import { clipToSpans, runAt } from '../../charts/series';
import type { Span } from '../../charts/series';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { isMember } from '../../dashboard/groups';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { NumericReply } from '../../dashboard/numericReply';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useProduct, useProducts } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import type { FuelDemandHistoryReply, FuelDemandPart, FuelDemandReply, FuelConsumptionReply, FuelReply } from '../../generated/dashboard';
import { formatFuelPart } from './fuelData';
import { usePreparedFuel, fuelSummaryRuns, useFuelSeriesParity } from './preparedFuel';
import FuelConsumptionStack from './FuelConsumptionStack';
import { useDetailView } from './FuelBreakdown';
import { consumptionAt, consumptionBasis, consumptionCoverage, consumptionForActor, consumptionGeometry, consumptionHistoryNumbers,
  consumptionLevelAvailable, consumptionParts, consumptionRuns, consumptionSelection, demandNumbers, mergeConsumptionWindows, siteLevel, validateFuelConsumption, validateFuelDemand, validateFuelDemandHistory } from './fuelConsumption';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from './fuelConsumption';

type HistoryChart = Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>;
const partRow = (part: FuelDemandPart, scale: number, color?: string): DrillPart => ({ id: part.key,
  label: part.recordedName ?? 'Name unavailable', raw: part.amountRaw === null ? null : BigInt(part.amountRaw),
  value: part.amountRaw === null ? null : Number(part.amountRaw) / scale, color,
  text: part.amountRaw === null ? 'Consumption unavailable' : `${formatFuelPart(BigInt(part.amountRaw), BigInt(scale))} per day` });
/** Family and displayed totals are written to `digits` decimals; each army, fleet or
 * air wing's own rate stays exact, since a fraction of one fuel would read as 0 or 1. */
export default function FuelConsumptionBreakdown({ reply, actorSpans, identitySpans, runtime, track, hour, selection, onSelection, level, onLevel, within, digits, ...chart }: {
  reply: FuelReply; runtime: WidgetRuntime; track: BoundTrack; hour: number | null; digits: number;
  actorSpans?: ReadonlyMap<string, readonly Span[]>; identitySpans?: ReadonlyMap<string, readonly Span[]>;
  selection: FuelConsumptionSelection | null; onSelection: (selection: FuelConsumptionSelection | null) => void;
  level: FuelConsumptionLevel | null; onLevel: (level: FuelConsumptionLevel | null) => void;
} & HistoryChart) {
  const tokens = useChartTokens();
  const levelName = level?.level ?? 'all';
  const scope = JSON.stringify([track.source.id, track.branch.id, reply.tag]);
  const requests = [...((identitySpans ?? actorSpans)?.keys() ?? [reply.tag])].map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'country-fuel-consumption', params: { branchId: track.branch.id, tag } }));
  const prepared = usePreparedFuel(runtime.client, track, requests.map(request=>request.params.tag), Math.floor(hour ?? chart.extent[1]), chart.active, ['consumption-0','consumption-1','consumption-2'], actorSpans, selection?.basis.hour);
  const legacyProduct = useProducts<FuelConsumptionReply>(runtime.client, requests, chart.active && !prepared.enabled, value => {
    const result = validateFuelConsumption(value);
    if (result.branchId !== track.branch.id || !requests.some(request => request.params.tag === result.tag))
      throw new Error('The selected consumption history belongs to another country or branch.');
    return result;
  });
  const product = prepared.enabled ? { ...prepared, data: prepared.consumption } : legacyProduct;
  const summaryStore = useMemo(() => new NumericReply<FuelConsumptionReply[]>(), [runtime.client]);
  const numbers = summaryStore.update(JSON.stringify(requests), product.data, values => values.map(value => ({ ...value, warnings: [] })));
  const summaries = product.data ?? numbers;
  const summary = summaries?.find(value => value.tag === reply.tag);
  const current = summary && hour !== null && isMember(within ?? undefined, hour) ? runAt(summary.reads, hour) : null;
  const matchesLevel = !level || level.campaign === track.source.metadata.campaignKey && level.branch === track.branch.id
    && level.tag === reply.tag && current?.country === level.country && current.countryInstance === level.countryInstance && current.segmentStartHour === level.segmentStartHour;
  const basis = summary ? consumptionBasis(summary, current) : null;
  const detail = useProduct<FuelDemandReply>(runtime.client, chart.active && basis ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-fuel-demand', params: basis } : null, chart.active, value => {
    const result = validateFuelDemand(value);
    if (!basis || result.branchId !== basis.branchId || result.tag !== basis.tag || result.hour !== basis.hour
      || result.read.country !== basis.country || result.read.countryInstance !== basis.countryInstance) throw new Error('The fuel consumption detail belongs to another actor or supplying read.');
    return result;
  });
  const store = useMemo(() => new NumericReply<FuelDemandReply>(), [runtime.client]);
  const detailNumbers = store.update(JSON.stringify([scope, basis]), detail.data, demandNumbers);
  const data = detail.data ?? detailNumbers;
  const availableLevel = matchesLevel && (!data || consumptionLevelAvailable(data, levelName));
  const displayed = useMemo(() => data && availableLevel ? consumptionParts(data, levelName) : [], [data, availableLevel, levelName]);
  const inventory = useMemo(() => displayed.map(part => partRow(part, reply.gainScale, tokens.categories[part.kind % tokens.categories.length])), [displayed, reply.gainScale, tokens]);
  const chooseLevel = (next: string) => { onSelection(null); if (next === 'all') onLevel(null);
    else if (current && current.countryInstance !== null) onLevel({ campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag: reply.tag,
      country: current.country, countryInstance: current.countryInstance, segmentStartHour: current.segmentStartHour, level: next }); };
  const chosen = consumptionForActor(selection, track.source.metadata.campaignKey, track.branch.id, reply.tag, identitySpans ?? actorSpans);
  const selectionScope = chosen && JSON.stringify(chosen);
  const coverage = useMemo(() => {
    if (!chosen) return [];
    if (!prepared.enabled) return consumptionCoverage(summaries ?? [], chosen, actorSpans);
    const point = prepared.states?.states.find(state=>state.tag===chosen.basis.tag && state.hour===chosen.basis.hour);
    if (!point?.consumptionSegment || point.consumption?.country!==chosen.basis.country || point.consumption.countryInstance!==chosen.basis.countryInstance
      || point.consumption.segmentStartHour!==chosen.segmentStartHour) return [];
    const spans = actorSpans ? [...actorSpans.values()].flat().sort((a,b)=>a.firstHour-b.firstHour) : undefined;
    return spans ? clipToSpans([point.consumptionSegment],spans) : [point.consumptionSegment];
  }, [summaries, actorSpans, selectionScope, prepared.states]); // eslint-disable-line react-hooks/exhaustive-deps
  const matching = chosen && hour !== null && !!runAt(coverage, hour);
  const part = matching ? data?.parts.find(row => row.key === chosen.key) : null;
  const owner = useMemo(() => ({ parent: chart.pickOwner, numbers: prepared.enabled ? prepared.owner : numbers, key: chosen?.key, levelName }), [chart.pickOwner, prepared.enabled ? prepared.owner : numbers, chosen?.key, levelName]);
  const series = useMemo<BreakdownSeries[]>(() => [0, 1, 2].map(kind => ({ id: String(kind), label: reply.consumerNames[kind] ?? 'Consumer name unavailable',
    color: tokens.categories[kind % tokens.categories.length], runs: (summaries ?? []).flatMap(source => {
      const runs = prepared.enabled ? fuelSummaryRuns(prepared.summary(source.tag, `consumption-${kind}`), reply.gainScale) : source.reads.flatMap((read, i, reads) => read.reasons[kind] !== null ? [] : [{
      firstHour: read.firstHour, lastHour: read.lastHour, value: Number(read.positiveDailyRaw[kind]) / reply.gainScale,
      breakBefore: read.startsSegment || i === 0 || reads[i - 1].reasons[kind] !== null || reads[i - 1].lastHour + 1 !== read.firstHour }]);
      return actorSpans ? clipToSpans(runs, actorSpans.get(source.tag) ?? []) : runs;
    }).sort((a, b) => a.firstHour - b.firstHour),
    amount: at => { const tag = actorSpans ? countryDrillTagAt(actorSpans, at) : reply.tag;
      const source = summaries?.find(value => value.tag === tag), read = source && runAt(source.reads, at); return read && read.reasons[kind] === null
      ? formatFuelPart(BigInt(read.knownDailyRaw[kind]), BigInt(reply.gainScale), digits) : 'Unavailable'; },
  })), [summaries, actorSpans, reply.tag, reply.gainScale, reply.consumerNames, tokens, digits, prepared.summaries]);
  const parity = useRef<HTMLDivElement>(null);
  useFuelSeriesParity(parity, runtime.client, track, requests.map(request=>request.params.tag), chart.extent, actorSpans, 'Fuel consumption values', summaries, true);
  const total = useMemo(() => inventory.reduce((sum, part) => sum + (part.raw ?? 0n), 0n), [inventory]);
  const complete = useMemo(() => {
    if (!data || !current || !availableLevel || inventory.some(part => part.raw === null)) return false;
    if (levelName.startsWith('site:')) {
      const site = data.sites.find((site, index) => siteLevel(site, index) === levelName);
      return site?.reason === null && site.wings.length === displayed.length;
    }
    const families = /^family:[012]$/.test(levelName) ? [Number(levelName.slice(7))]
      : levelName.startsWith('command:') ? [0] : levelName.startsWith('fleet:') ? [2] : [0, 1, 2];
    return families.every(kind => current.reasons[kind] === null) && (!levelName.startsWith('command:') || data.topologyReason === null);
  }, [inventory, current, data, availableLevel, levelName, displayed.length]);
  return <div ref={parity}>
    {product.error && <WidgetState kind="error" what="consumption" detail={product.error} onRetry={product.retry} />}
    {!summary && !product.error && <WidgetState kind="loading" label="Loading current consumption..." />}
    {detail.error && <WidgetState kind="error" what="consumers" detail={detail.error} onRetry={detail.retry} />}
    {current && <DrillParts positiveBars showShares={false} active={chart.active} parts={current.nativeDailyRaw.map((raw, kind) => ({ id: String(kind),
      label: `${reply.consumerNames[kind] ?? 'Consumer name unavailable'} demand per day`, value: raw === null ? null : Number(raw) / reply.gainScale,
      raw: raw === null ? null : BigInt(raw), text: raw === null ? 'Unavailable' : formatFuelPart(BigInt(raw), BigInt(reply.gainScale), digits) }))} format={String}
      onPick={key => chooseLevel(`family:${key}`)} />}
    {chosen ? <>
      <button type="button" className="button small quiet" onClick={() => onSelection(null)}>Back to displayed consumers</button>
      {!matching && <p className="drill-empty">Not recorded at this moment.</p>}
      {matching && !part && !detail.error && <p className="drill-empty">{data ? 'Consumer unavailable at this time.' : 'Loading consumer…'}</p>}
      {part && data && <ConsumptionFacts part={part} />}
      <ConsumptionHistory runtime={runtime} track={track} reply={reply} selection={chosen} coverage={coverage} hour={hour} within={within} {...chart} pickOwner={owner} />
    </> : <>
      {!basis && <p className="drill-empty">Individual consumption was not recorded for this moment.</p>}
      {basis && !data && !detail.error && <WidgetState kind="loading" label="Loading individual consumers..." />}
      {levelName !== 'all' && <button type="button" className="button small quiet" onClick={() => chooseLevel('all')}>All current consumers</button>}
      {!matchesLevel && <p className="drill-empty">Not recorded at this moment.</p>}
      {matchesLevel && !availableLevel && <p className="drill-empty">Not recorded at this moment.</p>}
      {data && <DrillParts virtual active={chart.active} positiveBars signedShares parts={inventory}
        total={complete ? Number(total) / reply.gainScale : null} format={String}
        totalRow={{ label: complete ? 'Daily demand' : 'Daily demand subtotal',
          value: !availableLevel ? 'Not recorded' : !complete && !inventory.some(part => part.raw !== null)
            ? 'Unavailable' : formatFuelPart(total, BigInt(reply.gainScale), digits) }}
        onPick={key => { const part = data.parts.find(part => part.key === key); if (part) onSelection(consumptionSelection(data, part, track.source.metadata.campaignKey)); }} />}
      {data && <ConsumptionTopology data={data} active={chart.active} onLevel={chooseLevel} />}
      {basis && data && availableLevel && <FuelConsumptionStack {...chart} within={within} pickOwner={owner} runtime={runtime} track={track}
        basis={basis} segmentStartHour={data.read.segmentStartHour} parts={displayed} hour={hour} detailHours={reply.detailHours} scale={reply.gainScale} />}
      {summary && <details><summary>By military branch</summary><BreakdownChart {...chart} onSummaryView={prepared.enabled ? prepared.onSummaryView : undefined} onHover={prepared.enabled ? prepared.onHover : undefined} onNavigateHour={prepared.enabled ? prepared.onNavigateHour : undefined} keyboardHour={prepared.keyboardHour} keyboardRevision={prepared.keyboardRevision} within={within} pickOwner={owner} series={series} selectedHour={hour}
        label={`${reply.tag}: current consumption families`} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} /></details>}
    </>}
  </div>;
}

function ConsumptionFacts({ part }: { part: FuelDemandPart }) {
  return <>
    <strong>{part.recordedName ?? 'Name unavailable'}</strong>
    {part.reason && <p className="drill-empty">Consumption unavailable.</p>}
  </>;
}
function ConsumptionTopology({ data, active, onLevel }: { data: FuelDemandReply; active: boolean; onLevel: (level: string) => void }) {
  const topology = useMemo(() => [
    ...data.commands.map(group => ({ id: `command:${group.reference}`, label: group.recordedName ?? 'Name unavailable', value: null,
      text: group.armyGroup ? 'Army group' : 'Army' })),
    ...data.fleets.map(fleet => ({ id: `fleet:${fleet.reference}`, label: fleet.recordedName ?? 'Name unavailable', value: null,
      text: 'Fleet' })),
  ], [data.commands, data.fleets]);
  const sites = useMemo(() => data.sites.map((site, index) => ({ key: siteLevel(site, index), item: site })), [data.sites]);
  return <details><summary>Armies, fleets and air bases</summary>
    {data.topologyReason && <p className="drill-empty">Some groups may be missing.</p>}
    <DrillParts virtual active={active} showShares={false} format={String} parts={topology} onPick={onLevel} />
    <WindowedRows items={active ? sites : []}
      rowHeight={64} label="Air bases">{({ key, item: site }) => <button type="button" tabIndex={0} className="button small quiet"
        onClick={() => onLevel(key)}>Air base · {site.wings.length} wings</button>}</WindowedRows>
  </details>;
}

function ConsumptionHistory({ runtime, track, reply, selection, coverage, hour, within, ...chart }: {
  runtime: WidgetRuntime; track: BoundTrack; reply: FuelReply; selection: FuelConsumptionSelection; coverage: readonly Span[]; hour: number | null;
} & HistoryChart) {
  const end = Math.max(chart.extent[0], Math.min(chart.extent[1], hour ?? chart.extent[1]));
  const start = Math.max(chart.extent[0], end - reply.detailHours + 1);
  const [visible, onViewChange] = useDetailView(start, end, reply.detailHours);
  const from = Math.max(chart.extent[0], visible[0]), to = Math.min(chart.extent[1], visible[1]);
  const wanted: number[] = [];
  for (let window = Math.floor(from / reply.detailHours); window <= Math.floor(to / reply.detailHours); window++) wanted.push(window);
  if (hour !== null && hour >= chart.extent[0] && hour <= chart.extent[1]) wanted.push(Math.floor(hour / reply.detailHours));
  const identity = JSON.stringify([track.source.id, track.branch.id, selection]);
  const content = useContentRevision(runtime.client);
  const requestIdentity = JSON.stringify([identity, track.source.revision, content]);
  const windows = useProductWindows<FuelDemandHistoryReply>(runtime.client, requestIdentity, window => ({ sourceId: track.source.id,
    revision: track.source.revision, product: 'country-fuel-demand-history', params: { ...selection.basis, window, key: selection.key } }),
    [...new Set(wanted)], chart.active, value => {
      const result = validateFuelDemandHistory(value);
      if (result.branchId !== selection.basis.branchId || result.tag !== selection.basis.tag || result.hour !== selection.basis.hour
        || result.country !== selection.basis.country || result.countryInstance !== selection.basis.countryInstance
        || result.segmentStartHour !== selection.segmentStartHour || result.key !== selection.key
        || result.firstHour !== result.window * reply.detailHours || result.lastHour !== (result.window + 1) * reply.detailHours - 1) throw new Error('The current consumption history belongs to another native occurrence.');
      return result;
    });
  const values = useMemo(() => [...windows.data.values()], [windows.data]);
  const store = useMemo(() => new NumericReply<FuelDemandHistoryReply[]>(), [runtime.client]);
  const previous = store.update(identity, undefined, consumptionHistoryNumbers);
  const wantedKey = wanted.join(',');
  const merged = useMemo(() => mergeConsumptionWindows(previous ?? [], values, wanted), [previous, values, wantedKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const numbers = store.update(identity, merged.length ? merged : undefined, consumptionHistoryNumbers);
  const numericRuns = useMemo(() => consumptionRuns(numbers ?? []), [numbers]);
  const names = useMemo(() => consumptionRuns(values), [values]);
  const allowed = useMemo(() => within ? clipToSpans(coverage, within) : coverage, [coverage, within]);
  const current = hour !== null && runAt(allowed, hour) ? consumptionAt(numericRuns, hour) : null;
  const currentName = hour !== null && runAt(allowed, hour) ? consumptionAt(names, hour) : null;
  const format = (at: number) => { if (!runAt(allowed, at)) return 'Consumer unavailable at this time.';
    const run = consumptionAt(numericRuns, at); return run?.amountRaw !== null && run
    ? `${formatFuelPart(BigInt(run.amountRaw), BigInt(reply.gainScale))} per day` : 'History unavailable'; };
  const owner = useMemo(() => ({ parent: chart.pickOwner, numbers, identity, allowed }), [chart.pickOwner, numbers, identity, allowed]);
  const series: BreakdownSeries[] = [{ id: selection.key, label: 'Individual current consumption', color: useChartTokens().accent,
    runs: consumptionGeometry(numericRuns, reply.gainScale), amount: format, missingAt: format,
    labelAt: at => runAt(allowed, at) ? consumptionAt(names, at)?.recordedName ?? 'Name unavailable' : 'Name unavailable' }];
  return <>
    <DrillParts positiveBars showShares={false} format={String} parts={[{ id: selection.key,
      label: currentName?.recordedName ?? 'Name unavailable', value: current?.amountRaw !== null && current ? Number(current.amountRaw) / reply.gainScale : null,
      raw: current?.amountRaw !== null && current ? BigInt(current.amountRaw) : null, text: hour === null ? 'Not recorded' : format(hour) }]} />
    {chart.active && wanted.some(window => !windows.data.has(window) && !windows.errors.has(window)) && <p className="drill-empty">Loading consumption history…</p>}
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" what="consumption history" detail={reason} onRetry={windows.retry} />)}
    <BreakdownChart {...chart} within={allowed} pickOwner={owner} series={series} selectedHour={hour} keyHours={numericRuns.map(run => run.firstHour)}
      onViewChange={onViewChange}
      label="Individual current consumption required per day" axes={{ xFrom: start - chart.branchStart, xTo: end - chart.branchStart, yMin: 0, yMax: null, log: false }} />
  </>;
}
