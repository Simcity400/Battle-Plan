import { useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText, SunburstValues } from '../../charts/sunburst/Sunburst';
import type { SunburstItem, SunburstKey } from '../../charts/sunburst/layout';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import Legend from '../../charts/Legend';
import { columnAt, columnsLayer } from '../../charts/columns';
import type { ColumnPeriod } from '../../charts/columns';
import { liveAt } from '../../charts/live';
import type { Live } from '../../charts/live';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { UNIT, formatAppliedIc, noProductionAt, spendingRecorded } from '../production-spending/spendingData';
import type { SpendingWindow } from '../production-spending/spendingData';
import { useSpending } from '../production-spending/useSpending';
import type { DockyardViewChoice } from './definition';
import { LEVELS, formatExactIc, monthColumns, monthLabel, monthsOf, reasonLabel, shownMonthBands } from './dockyardData';
import type { Exact, PurposeMonth } from './dockyardData';
import './dockyardSpending.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import type { ProductionSpendingReply } from '../../generated/dashboard';
import type { SpendingData } from '../production-spending/spendingData';

type Options = Readonly<Record<string, OptionValue>>;

export interface PurposeSpendingKind {
  name: string;
  measure: string;
  root: SunburstKey;
  bands: readonly { id: string; label: string }[];
  options: readonly OptionField[];
  repairs: boolean;
  validate: (data: unknown) => ProductionSpendingReply;
  read?: (data: SpendingData) => boolean;
  items: (data: SpendingData, options: Options, window: SpendingWindow, colours: readonly string[]) => SunburstItem[];
  other?: (data: SpendingData, window: SpendingWindow) => Exact;
  month: (data: SpendingData, options: Options, first: number, last: number, branchFirst: number) => PurposeMonth;
  note?: string;
}

const NO_WARNINGS: string[] = [];
const ZERO: Exact = { exact: 0n, reason: null };
const sentence = (text: string) => text[0].toUpperCase() + text.slice(1);

export function PurposeSpendingWidget({ kind, widget, runtime, active, onDiagnostics }: WidgetProps & { kind: PurposeSpendingKind }) {
  const options = resolveOptions(kind.options, widget.options);
  const view: DockyardViewChoice = options.view === 'months' ? 'months' : 'circle';
  const headKey = useRef<(hour: number) => string>(() => '');
  const spending = useSpending(widget, runtime, active, kind.repairs, kind.validate, hour => headKey.current(hour), view === 'months');
  useShownEachFrame(runtime.clock, () => spending.data && spending.track ? trackHourOf(widget, spending.track, runtime.tracks, runtime.clock.getHour()) : null);
  const { track, tag, data, error, retry, hour, from, end, headAt } = spending;
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  headKey.current = at => view === 'months' || !data || end == null ? view : JSON.stringify(emptyAt(kind, data, { from, hour: at, end }));
  const hoursPerSecond = usePlaybackMotion(runtime.clock);

  const read = data && (spendingRecorded(data) || kind.read?.(data) === true);
  if (!track || !tag) return <WidgetState kind="empty" icon="sunburst" message={`Load a campaign and choose a country to see its ${kind.name.toLowerCase()}.`} />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label={`Loading ${kind.name.toLowerCase()}…`} />;
  if (!read) return <WidgetState kind="unavailable" message={`${kind.name} wasn’t recorded.`} />;
  if (hour == null || end == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="outside" />;
  if (view === 'months') return <PurposeMonths kind={kind} data={data} options={options}
    extent={[from ?? track.branch.firstHour, widget.time.mode === 'range' ? hour : track.branch.lastHour]}
    last={headAt ? track.branch.lastHour : hour} playhead={headAt ? hour : null} headAt={headAt} runtime={runtime}
    branchStart={track.branch.firstHour} decimals={decimals} digits={digits} hoursPerSecond={hoursPerSecond} active={active} />;
  return <PurposeCircle kind={kind} data={data} window={{ from, hour, end }} options={options} decimals={decimals} digits={digits}
    headAt={headAt} runtime={runtime} hoursPerSecond={hoursPerSecond} active={active} />;
}

function emptyAt(kind: PurposeSpendingKind, data: SpendingData, window: SpendingWindow): [boolean, string] {
  const other = kind.other?.(data, window) ?? ZERO;
  return [noProductionAt(data, window.hour) && other.exact === 0n,
    other.exact == null ? reasonLabel(other.reason) : `No ${kind.measure} spent in this view.`];
}

function usePurposeColours(bands: readonly unknown[]): string[] {
  const tokens = useChartTokens();
  return useMemo(() => bands.map((_, index) => tokens.categories[index % tokens.categories.length]), [tokens, bands]);
}

type Runtime = WidgetProps['runtime'];

export function PurposeCircle({ kind, data, window, options, decimals, digits, headAt, runtime, hoursPerSecond, active }: {
  kind: PurposeSpendingKind; data: SpendingData; window: SpendingWindow; options: Options; decimals: number; digits: number;
  headAt: ((position: number) => number | null) | null; runtime: Runtime; hoursPerSecond: number | undefined; active: boolean;
}) {
  const settings = sunburstSettings(options);
  const colours = usePurposeColours(kind.bands);
  const { from, hour, end } = window;
  const valuesAt = (at: number): SunburstValues => ({ items: kind.items(data, options, { from, hour: at, end }, colours) });
  const build = useRef(valuesAt);
  build.current = valuesAt;
  const values = useMemo(() => valuesAt(hour), [kind, data, options, from, hour, end, colours]);
  const live = useMemo(() => headAt ? liveAt(runtime.clock, position => Math.floor(headAt(position) ?? hour), at => build.current(at)) : undefined,
    [headAt, runtime.clock, kind, data, options, from, end, colours]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const [empty, emptyLabel] = emptyAt(kind, data, window);
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact, denominator) => exact == null ? formatAppliedIc(value, null, decimals, digits)
      : formatExactIc(exact, denominator ?? 1n, decimals, digits), unit: UNIT, valueLabel: sentence(kind.measure), countLabel: 'Designs',
    emptyLabel,
  }), [decimals, digits, emptyLabel, kind.measure]);
  return empty ? <WidgetState kind="empty" icon="sunburst" message={`No ${kind.measure} spent yet.`} />
    : <Sunburst label={kind.name} items={values.items} root={kind.root} levels={levels} text={text} settings={settings}
      live={live} hoursPerSecond={hoursPerSecond} active={active} />;
}

function headMonthLayer(columns: readonly ColumnPeriod[], colours: readonly string[], through: (index: number, hour: number) => ColumnPeriod): ChartLayer {
  const base = columnsLayer(columns, colours);
  let held: { hour: number; layer: ChartLayer } | null = null;
  return {
    draw(ctx, frame, tokens, state) {
      const head = state.playhead == null ? null : Math.floor(state.playhead);
      const index = head == null ? -1 : columnAt(columns, head);
      if (head == null || index < 0 || head >= columns[index].lastHour) return base.draw(ctx, frame, tokens, state);
      if (held?.hour !== head) {
        const shown = [...columns];
        shown[index] = through(index, head);
        held = { hour: head, layer: columnsLayer(shown, colours) };
      }
      held.layer.draw(ctx, frame, tokens, state);
    },
    extent: base.extent,
  };
}

export function PurposeMonths({ kind, data, options, extent, last, playhead, headAt, runtime, branchStart, decimals, digits, hoursPerSecond, active }: {
  kind: PurposeSpendingKind; data: SpendingData; options: Options; extent: [number, number]; last: number; playhead: number | null;
  headAt: ((position: number) => number | null) | null; runtime: Runtime; branchStart: number;
  decimals: number; digits: number; hoursPerSecond: number | undefined; active: boolean;
}) {
  const colours = usePurposeColours(kind.bands);
  const first = extent[0];
  const months = useMemo(() => monthsOf(first, last).map(month => kind.month(data, options, month.firstHour, month.lastHour, branchStart)),
    [kind, data, options, first, last, branchStart]);
  const columns = useMemo(() => monthColumns(months, decimals), [months, decimals]);
  const head = useMemo<Live<number> | undefined>(() => {
    if (!headAt || playhead == null) return undefined;
    const live = liveAt(runtime.clock, position => Math.floor(headAt(position) ?? playhead), at => at);
    return { read: live.hour, subscribe: live.subscribe };
  }, [headAt, runtime.clock, playhead]);
  const monthThrough = (index: number, hour: number) => kind.month(data, options, months[index].firstHour, Math.min(months[index].lastHour, hour), branchStart);
  const layers = useMemo(() => [headMonthLayer(columns, colours, (index, hour) => monthColumns([monthThrough(index, hour)], decimals)[0])],
    [columns, colours]);
  const keyHours = useMemo(() => columns.map(column => column.firstHour), [columns]);
  const label = `${kind.name} by month`;
  const write = ({ exact, reason, denominator }: Exact) => exact == null ? reasonLabel(reason)
    : `${formatExactIc(exact, denominator ?? 1n, decimals, digits)} ${UNIT}`;
  const shownBands = useMemo(() => shownMonthBands(kind.bands, months), [kind.bands, months]);
  return <div className="dockyard-months">
    <Legend label={`${sentence(kind.measure)} spent on`} always items={shownBands.map(({ band, index }) => ({ id: band.id, label: band.label, color: colours[index] }))} />
    <div className="dockyard-months-chart">
      <TimeChart label={label} extent={extent} playhead={playhead} liveHead={head} playback="draw-in" branchStart={branchStart}
        hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        fitValues={shown => shown} layers={layers} keyHours={keyHours} active={active} revision={{ columns, colours, digits }}
        tooltip={at => {
          const index = columnAt(columns, Math.floor(at));
          if (index < 0) return null;
          const headHour = head?.read();
          const month = headHour != null && headHour >= months[index].firstHour && headHour < months[index].lastHour ? monthThrough(index, headHour) : months[index];
          return {
            title: monthLabel(month.firstHour),
            rows: [
              ...shownBands.map(({ band, index: row }) => ({ label: band.label, swatch: colours[row], value: write(month.bands[row]) })),
              { label: 'Total', strong: true, divider: true, value: write(month.total) },
            ],
            note: [month.note, kind.note].filter(Boolean).join(' ') || undefined,
          };
        }} />
    </div>
  </div>;
}
