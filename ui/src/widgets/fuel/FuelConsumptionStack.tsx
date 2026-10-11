import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import BreakdownChart from '../../charts/drill/BreakdownChart';
import type { BreakdownSeries } from '../../charts/drill/BreakdownChart';
import { runAt } from '../../charts/series';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { NumericReply } from '../../dashboard/numericReply';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useProductWindows } from '../../dashboard/productWindows';
import type { FuelAwardsParams, FuelDemandPart, FuelDemandStackReply } from '../../generated/dashboard';
import { formatFuelPart } from './fuelData';
import { useDetailView } from './FuelBreakdown';
import { consumptionGeometry, consumptionStackNumbers, mergeConsumptionWindows, stackedConsumptionRuns, validateFuelDemandStack } from './fuelConsumption';

type Chart = Omit<Parameters<typeof BreakdownChart>[0], 'series' | 'axes' | 'label'>;
/** The displayed native cohort shares one visible-window request and decoder. */
export default function FuelConsumptionStack({ runtime, track, basis, segmentStartHour, parts, detailHours, scale, hour, within, ...chart }: {
  runtime: WidgetRuntime; track: BoundTrack; basis: FuelAwardsParams; segmentStartHour: number; parts: readonly FuelDemandPart[];
  detailHours: number; scale: number; hour: number | null;
} & Chart) {
  const tokens = useChartTokens();
  const height = useViewportHeight();
  const end = Math.max(chart.extent[0], Math.min(chart.extent[1], hour ?? chart.extent[1]));
  const start = Math.max(chart.extent[0], end - detailHours + 1);
  const [visible, onViewChange] = useDetailView(start, end, detailHours);
  const from = Math.max(chart.extent[0], visible[0]), to = Math.min(chart.extent[1], visible[1]);
  const wanted = new Set<number>();
  for (let window = Math.floor(from / detailHours); window <= Math.floor(to / detailHours); window++) wanted.add(window);
  if (hour !== null && hour >= chart.extent[0] && hour <= chart.extent[1]) wanted.add(Math.floor(hour / detailHours));
  const keys = useMemo(() => parts.map(part => part.key), [parts]);
  const identity = useMemo(() => JSON.stringify([track.source.id, track.branch.id, basis.tag, basis.country, basis.countryInstance, segmentStartHour, keys]),
    [track.source.id, track.branch.id, basis.tag, basis.country, basis.countryInstance, segmentStartHour, keys]);
  const content = useContentRevision(runtime.client);
  const requestIdentity = useMemo(() => JSON.stringify([identity, basis.hour, track.source.revision, content]), [identity, basis.hour, track.source.revision, content]);
  const windows = useProductWindows<FuelDemandStackReply>(runtime.client, requestIdentity, window => ({ sourceId: track.source.id,
    revision: track.source.revision, product: 'country-fuel-demand-stack', params: { ...basis, window, keys } }), [...wanted], chart.active, value => {
    const result = validateFuelDemandStack(value);
    if (result.branchId !== basis.branchId || result.tag !== basis.tag || result.hour !== basis.hour || result.country !== basis.country
      || result.countryInstance !== basis.countryInstance || result.segmentStartHour !== segmentStartHour
      || result.firstHour !== result.window * detailHours || result.lastHour !== (result.window + 1) * detailHours - 1
      || result.parts.length !== keys.length || result.parts.some((part, i) => part.key !== keys[i])) throw new Error('The fuel stack belongs to another supplying cohort.');
    return result;
  });
  const values = useMemo(() => [...windows.data.values()], [windows.data]);
  const incoming = useMemo(() => consumptionStackNumbers(values), [values]);
  const store = useMemo(() => new NumericReply<ReturnType<typeof consumptionStackNumbers>>(), [runtime.client]);
  const previous = store.update(identity, undefined, value => value);
  const wantedKey = [...wanted].join(',');
  const merged = useMemo(() => mergeConsumptionWindows(previous ?? [], incoming, [...wanted]), [previous, incoming, wantedKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const numbers = store.update(identity, merged.length ? merged : undefined, value => value);
  const numeric = useMemo(() => stackedConsumptionRuns(numbers ?? []), [numbers]);
  const names = useMemo(() => stackedConsumptionRuns(values), [values]);
  const keyHours = useMemo(() => {
    const hours = new Set<number>();
    for (const runs of numeric.values()) for (const run of runs) hours.add(run.firstHour);
    return [...hours].sort((a, b) => a - b);
  }, [numeric]);
  const allowed = (at: number) => !within || !!runAt(within, at);
  const series = useMemo<BreakdownSeries[]>(() => parts.map(part => {
    const runs = numeric.get(part.key) ?? [];
    const amount = (at: number) => { const read = allowed(at) ? runAt(runs, at) : null;
      return read?.amountRaw != null ? `${formatFuelPart(BigInt(read.amountRaw), BigInt(scale))} per day` : read?.reason ?? 'Not recorded'; };
    return { id: part.key, label: part.recordedName ?? 'Name not recorded', color: tokens.categories[part.kind % tokens.categories.length],
      runs: consumptionGeometry(runs, scale), amount, missingAt: amount,
      labelAt: at => allowed(at) ? runAt(names.get(part.key) ?? [], at)?.recordedName ?? 'Name not recorded' : 'Name not recorded' };
  }), [parts, numeric, names, scale, tokens, within]); // eslint-disable-line react-hooks/exhaustive-deps
  const owner = useMemo(() => ({ parent: chart.pickOwner, identity, numbers, within }), [chart.pickOwner, identity, numbers, within]);
  return <>
    <p className="drill-empty">The same displayed native occurrences over time. Absent or changed occurrences stay unknown; positive rates draw the stack and hover retains signed amounts and supplying-read names.</p>
    {chart.active && [...wanted].some(window => !windows.data.has(window) && !windows.errors.has(window)) && <p className="drill-empty">Individual stacked history is loading.</p>}
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" message={reason} onRetry={windows.retry} />)}
    {values.flatMap(value => value.warnings).map((reason, i) => <p key={i} className="drill-empty">{reason}</p>)}
    <BreakdownChart {...chart} within={within} pickOwner={owner} series={series} selectedHour={hour} tooltipHeight={height} keyHours={keyHours}
      onViewChange={onViewChange}
      label="Displayed individual current consumption required per day" axes={{ xFrom: start - chart.branchStart, xTo: end - chart.branchStart, yMin: 0, yMax: null, log: false }} />
  </>;
}
