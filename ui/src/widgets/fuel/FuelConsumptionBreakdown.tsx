import type { CountryAt, Gate } from '../../dashboard/regionGate';
import { answersKey, countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../../charts/drill/DrillParts';
import { mapLive } from '../../charts/live';
import { runAt } from '../../charts/series';
import type { Span } from '../../charts/series';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import WindowedRows from '../../charts/WindowedRows';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { FAMILY_MEASURES, FUEL_DEMAND_TOTALS, formatFuelPart, scaledSummary, summaryRuns } from './fuelData';
import { consumptionLevelAvailable, consumptionParts, demandBasis, demandGeometry, levelComplete, siteLevel, validateFuelDemand,
  validateFuelDemandHistory } from './fuelConsumption';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from './fuelConsumption';
import { detailWindows, fuelRecipientFor, joinedRuns } from './fuelDrill';
import type { FuelBasis } from './fuelDrill';
import { useDetailView } from './FuelBreakdown';
import type { DrillChart } from './FuelBreakdown';
import { useFuelAt, usePartSeries } from './fuelReads';
import type { CountryFuelReply, FuelDemandHistoryReply, FuelDemandPart, FuelDemandReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

type HistoryChart = Omit<DrillChart, 'detailHours'>;
const GAIN = 100000n;

const partRow = (part: FuelDemandPart, color?: string): DrillPart => ({ id: part.key, label: part.recordedName ?? 'Name unavailable',
  raw: part.amountRaw === null ? null : BigInt(part.amountRaw), value: part.amountRaw === null ? null : Number(part.amountRaw) / Number(GAIN), color,
  detail: part.reason ?? part.nameReason ?? undefined,
  text: part.amountRaw === null ? 'Consumption unavailable' : `${formatFuelPart(BigInt(part.amountRaw), GAIN)} per day` });

function familyRows(reply: CountryFuelReply | undefined, digits: number): DrillPart[] {
  const consumption = reply?.consumption;
  return [0, 1, 2].map(kind => {
    const raw = consumption?.nativeDailyRaw[kind] ?? null;
    return { id: String(kind), label: `${reply?.consumerNames[kind] ?? 'Consumer name unavailable'} demand per day`, value: raw === null ? null : Number(raw) / Number(GAIN),
      raw: raw === null ? null : BigInt(raw), text: raw === null ? consumption?.reasons[kind] ?? reply?.consumptionMissing ?? 'Unavailable' : formatFuelPart(BigInt(raw), GAIN, digits) };
  });
}

const familyStructure = (reply: CountryFuelReply) => JSON.stringify([reply.consumptionMissing, reply.consumption?.reasons ?? null, reply.consumerNames]);

export default function FuelConsumptionBreakdown({ runtime, gate, widget, track, tag, hour, follows, within, selection, onSelection, level, onLevel, digits, campaign,
  aliases, detailHours, history, ...chart }: {
  runtime: WidgetRuntime; gate: Gate; widget: WidgetSpec; track: BoundTrack; tag: string; hour: number | null; follows: boolean; digits: number; campaign: string;
  aliases?: ReadonlyMap<string, readonly Span[]>; history: CountryAt['history'];
  selection: FuelConsumptionSelection | null; onSelection: (selection: FuelConsumptionSelection | null) => void;
  level: FuelConsumptionLevel | null; onLevel: (level: FuelConsumptionLevel | null) => void;
} & DrillChart) {
  const tokens = useChartTokens();
  const shown = useShownHour();
  const at = useFuelAt(runtime, widget, tag, hour, follows, chart.active, familyStructure);
  const reply = at.data;
  useEffect(() => { shown(at.shownAt()); });
  const families = useMemo<LiveDrillParts>(() => mapLive(at.live, value => { shown(at.shownAt()); return { parts: familyRows(value, digits) }; }), [at, digits, shown]);
  const playing = usePlaybackMotion(runtime.clock) !== undefined;
  const demandHour = gate.recorded(follows ? (playing ? null : at.live.read()?.read?.readHour ?? null) : hour);
  const demandTag = demandHour === null ? null : gate.country?.aliasAt(demandHour) ?? null;
  const demand = useProduct<FuelDemandReply>(runtime.client, demandHour !== null && demandTag !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'country-fuel-demand', params: { branchId: track.branch.id, country: countryFromKey(demandTag), hour: demandHour } }) : null, chart.active, validateFuelDemand, track.source);
  const data = demand.data && answersKey(demand.data.asked, demandTag) ? demand.data : undefined;
  const basis = data ? demandBasis(track.branch.id, data) : null;
  const levelName = level && data && level.basis.country === data.country && level.basis.countryInstance === data.countryInstance ? level.level : level ? null : 'all';
  const available = levelName !== null && (!data || consumptionLevelAvailable(data, levelName));
  const displayed = useMemo(() => data && available && levelName ? consumptionParts(data, levelName) : [], [data, available, levelName]);
  const inventory = useMemo(() => displayed.map(part => partRow(part, tokens.categories[part.kind % tokens.categories.length])), [displayed, tokens.categories]);
  const total = useMemo(() => inventory.reduce((sum, part) => sum + (part.raw ?? 0n), 0n), [inventory]);
  const complete = !!data && !!levelName && available && levelComplete(data, levelName, displayed);
  const chooseLevel = useCallback((next: string) => {
    onSelection(null);
    if (next === 'all') onLevel(null);
    else if (basis) onLevel({ campaign, branch: track.branch.id, basis, level: next });
  }, [onSelection, onLevel, basis, campaign, track.branch.id]);
  const chosen = fuelRecipientFor(selection, campaign, track.branch.id, tag, aliases);
  const part = chosen && data && data.country === chosen.basis.country && data.countryInstance === chosen.basis.countryInstance
    ? data.parts.find(row => row.key === chosen.key) : null;
  const byBranch = usePartSeries(runtime, gate, track, FUEL_DEMAND_TOTALS, FAMILY_MEASURES, history, chart.active);
  const branchSeries = useMemo(() => FAMILY_MEASURES.map((_, kind) => {
    const summary = scaledSummary(byBranch.summaries[kind], 1);
    const series: BreakdownSeries = { id: String(kind), label: reply?.consumerNames[kind] ?? 'Consumer name unavailable', color: tokens.categories[kind % tokens.categories.length],
      runs: summaryRuns(summary), amount: when => hour !== null && Math.floor(when) === hour && reply?.consumption?.positiveDailyRaw[kind] != null
        ? formatFuelPart(BigInt(reply.consumption.positiveDailyRaw[kind]!), GAIN, digits) : 'Hover a day to read it' };
    return { summary, series };
  }), [byBranch.summaries, reply, tokens.categories, hour, digits]);

  return <div ref={at.root}>
    {at.error && <WidgetState kind="error" what="consumption" failure={at.error} onRetry={at.retry} />}
    {!reply && !at.error && <WidgetState kind="loading" label="Loading current consumption…" read={at} />}
    {reply && <DrillParts positiveBars showShares={false} active={chart.active} parts={familyRows(reply, digits)} live={follows ? families : undefined}
      format={String} onPick={key => chooseLevel(`family:${key}`)} />}
    {demand.error && <WidgetState kind="error" what="consumers" failure={demand.error} onRetry={demand.retry} />}
    {chosen ? <>
      <button type="button" className="button small quiet" onClick={() => onSelection(null)}>Back to displayed consumers</button>
      <strong>{part?.recordedName ?? chosen.name ?? 'Name unavailable'}</strong>
      {part?.reason && <p className="drill-empty">Consumption unavailable: {part.reason}</p>}
      {data && !part && <p className="drill-empty">Consumer unavailable at this time.</p>}
      <DemandHistory runtime={runtime} gate={gate} track={track} basis={chosen.basis} keys={[chosen.key]} names={new Map([[chosen.key, part?.recordedName ?? chosen.name]])}
        colors={new Map([[chosen.key, tokens.accent]])} hour={hour} detailHours={detailHours} within={within} chart={chart}
        label="Individual current consumption required per day" />
    </> : <>
      {demandHour === null ? <p className="drill-empty">{follows ? 'Individual consumers are listed for the day shown once playback stops.' : 'Individual consumption was not recorded for this moment.'}</p>
        : !data && !demand.error ? <WidgetState kind="loading" label="Loading individual consumers…" read={at} /> : null}
      {levelName !== 'all' && <button type="button" className="button small quiet" onClick={() => chooseLevel('all')}>All current consumers</button>}
      {data && !available && <p className="drill-empty">Not recorded at this moment.</p>}
      {data && available && <DrillParts virtual active={chart.active} positiveBars signedShares parts={inventory}
        total={complete ? Number(total) / Number(GAIN) : null} format={String}
        totalRow={{ label: complete ? 'Daily demand' : 'Daily demand subtotal',
          value: !complete && !inventory.some(row => row.raw != null) ? 'Unavailable' : formatFuelPart(total, GAIN, digits) }}
        onPick={key => { const row = data.parts.find(item => item.key === key); if (row && basis) onSelection({ campaign, branch: track.branch.id, basis, key: row.key, name: row.recordedName }); }} />}
      {data && <ConsumptionTopology data={data} active={chart.active} onLevel={chooseLevel} />}
      {basis && data && available && displayed.length > 0 && <DemandHistory runtime={runtime} gate={gate} track={track} basis={basis} keys={displayed.map(row => row.key)}
        names={new Map(displayed.map(row => [row.key, row.recordedName]))} colors={new Map(displayed.map(row => [row.key, tokens.categories[row.kind % tokens.categories.length]]))}
        hour={hour} detailHours={detailHours} within={within} chart={chart} label="Displayed individual current consumption required per day" />}
      <details><summary>By military branch</summary>
        {byBranch.error && <WidgetState kind="error" what="consumption history" failure={byBranch.error} onRetry={byBranch.retry} />}
        <BreakdownChart {...chart} within={within} onSummaryView={byBranch.onSummaryView} series={branchSeries.map(item => item.series)}
          summaries={{ parts: branchSeries.filter(item => item.summary).length === branchSeries.length ? branchSeries.map(item => item.summary!) : [] }}
          selectedHour={hour} label={`${tag}: current consumption families`} axes={{ xFrom: null, xTo: null, yMin: 0, yMax: null, log: false }} />
      </details>
    </>}
  </div>;
}

function ConsumptionTopology({ data, active, onLevel }: { data: FuelDemandReply; active: boolean; onLevel: (level: string) => void }) {
  const topology = useMemo(() => [
    ...data.commands.map(group => ({ id: `command:${group.reference}`, label: group.recordedName ?? 'Name unavailable', value: null, text: group.armyGroup ? 'Army group' : 'Army' })),
    ...data.fleets.map(fleet => ({ id: `fleet:${fleet.reference}`, label: fleet.recordedName ?? 'Name unavailable', value: null, text: 'Fleet' })),
  ], [data.commands, data.fleets]);
  const sites = useMemo(() => data.sites.map((site, index) => ({ key: siteLevel(site, index), item: site })), [data.sites]);
  return <details><summary>Armies, fleets and air bases</summary>
    {data.topologyReason && <p className="drill-empty">Some groups may be missing: {data.topologyReason}</p>}
    <DrillParts virtual active={active} showShares={false} format={String} parts={topology} onPick={onLevel} />
    <WindowedRows items={active ? sites : []} rowHeight={64} label="Air bases">{({ key, item: site }) => <button type="button" tabIndex={0} className="button small quiet"
      onClick={() => onLevel(key)}>Air base · {site.wings.length} wings{site.reason ? ` (${site.reason})` : ''}</button>}</WindowedRows>
  </details>;
}

function DemandHistory({ runtime, gate, track, basis, keys, names, colors, hour, detailHours, within, chart, label }: {
  runtime: WidgetRuntime; gate: Gate; track: BoundTrack; basis: FuelBasis; keys: readonly string[]; names: ReadonlyMap<string, string | null>;
  colors: ReadonlyMap<string, string>; hour: number | null; detailHours: number; within: readonly Span[] | null | undefined; chart: HistoryChart; label: string;
}) {
  const height = useViewportHeight();
  const end = Math.max(chart.extent[0], Math.min(chart.extent[1], hour ?? chart.extent[1]));
  const start = Math.max(chart.extent[0], end - detailHours + 1);
  const [visible, onViewChange] = useDetailView(start, end, detailHours);
  const wanted = detailWindows(chart.extent, visible, hour, detailHours);
  const basisAt = gate.recorded(basis.hour);
  const windows = useProductWindows<FuelDemandHistoryReply>(runtime.client, basisAt === null ? null : JSON.stringify([track.source.id, track.source.revision, basis, keys]),
    window => (productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'country-fuel-demand-history',
      params: { branchId: basis.branchId, country: countryFromKey(basis.tag), hour: basisAt!, window, keys } })),
    wanted, chart.active, validateFuelDemandHistory, track.source);
  const runs = useMemo(() => new Map(keys.map(key => [key, joinedRuns([...windows.data.values()].map(reply => reply.parts.find(part => part.key === key)?.runs ?? []))])),
    [windows.data, keys]);
  const keyHours = useMemo(() => [...new Set([...runs.values()].flatMap(list => list.map(run => run.firstHour)))].sort((a, b) => a - b), [runs]);
  const series = useMemo<BreakdownSeries[]>(() => keys.map(key => {
    const allowed = (when: number) => !within || !!runAt(within, when);
    const list = runs.get(key) ?? [];
    const amount = (when: number) => { const run = allowed(when) ? runAt(list, when) : null;
      return run?.amountRaw != null ? `${formatFuelPart(BigInt(run.amountRaw), GAIN)} per day` : run?.reason ?? 'History unavailable'; };
    return { id: key, label: names.get(key) ?? 'Name unavailable', color: colors.get(key) ?? '', runs: demandGeometry(list, Number(GAIN)), amount, missingAt: amount };
  }), [keys, runs, names, colors, within]);
  return <>
    {chart.active && wanted.some(window => !windows.data.has(window) && !windows.errors.has(window)) && <p className="drill-empty">Loading consumption history…</p>}
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" what="consumption history" failure={reason} onRetry={windows.retry} />)}
    <BreakdownChart {...chart} within={within} series={series} selectedHour={hour} tooltipHeight={height} keyHours={keyHours} onViewChange={onViewChange}
      label={label} axes={{ xFrom: start - chart.branchStart, xTo: end - chart.branchStart, yMin: 0, yMax: null, log: false }} />
  </>;
}
