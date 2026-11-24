import { useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText, SunburstValues } from '../../charts/sunburst/Sunburst';
import type { SunburstItem, SunburstKey } from '../../charts/sunburst/layout';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { liveAt } from '../../charts/live';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { UNIT, formatAppliedIc, noProductionAt, spendingRecorded, spendingWidgetGate } from '../production-spending/spendingData';
import type { SpendingWindow } from '../production-spending/spendingData';
import { useSpending } from '../production-spending/useSpending';
import { LEVELS, formatExactIc, reasonLabel } from './dockyardData';
import type { Exact } from './dockyardData';
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
  read?: (data: SpendingData, at?: number) => boolean;
  items: (data: SpendingData, options: Options, window: SpendingWindow, colours: readonly string[]) => SunburstItem[];
  other?: (data: SpendingData, window: SpendingWindow) => Exact;
  note?: string;
}

const NO_WARNINGS: string[] = [];
const ZERO: Exact = { exact: 0n, reason: null };
const sentence = (text: string) => text[0].toUpperCase() + text.slice(1);

export function PurposeSpendingWidget({ kind, widget, runtime, active, onDiagnostics }: WidgetProps & { kind: PurposeSpendingKind }) {
  const options = resolveOptions(kind.options, widget.options);
  const headKey = useRef<(hour: number) => string>(() => '');
  const spending = useSpending(widget, runtime, active, kind.repairs, kind.validate, hour => headKey.current(hour), false);
  useShownEachFrame(runtime.clock, () => spending.data && spending.track ? trackHourOf(widget, spending.track, runtime.tracks, runtime.clock.getHour()) : null);
  const { track, tag, data, error, retry, hour, from, end, headAt } = spending;
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  headKey.current = at => {
    if (!data || !track || end == null) return 'circle';
    const floored = Math.floor(at);
    const read = spendingRecorded(data) || kind.read?.(data, floored) === true;
    return `${read ? 1 : 0}:${JSON.stringify(emptyAt(kind, data, { from, hour: at, end }))}`;
  };
  const hoursPerSecond = usePlaybackMotion(runtime.clock);

  const read = Boolean(data && (spendingRecorded(data) || kind.read?.(data, hour) === true));
  const gate = spendingWidgetGate({ hasTarget: Boolean(track && tag), hasError: Boolean(error), branch: track?.branch, hour, end, hasData: Boolean(data), recorded: read });
  if (gate === 'empty') return <WidgetState kind="empty" icon="sunburst" message={`Load a campaign and choose a country to see its ${kind.name.toLowerCase()}.`} />;
  if (gate === 'error') return <WidgetState kind="error" failure={error!} onRetry={retry} />;
  if (gate === 'outside') return <WidgetState kind="outside" />;
  if (gate === 'loading') return <WidgetState kind="loading" label={`Loading ${kind.name.toLowerCase()}…`} />;
  if (gate === 'unavailable') return <WidgetState kind="unavailable" message={`${kind.name} wasn’t recorded.`} />;
  return <PurposeCircle kind={kind} data={data!} window={{ from, hour: hour!, end: end! }} options={options} decimals={decimals} digits={digits}
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

function PurposeCircle({ kind, data, window, options, decimals, digits, headAt, runtime, hoursPerSecond, active }: {
  kind: PurposeSpendingKind; data: SpendingData; window: SpendingWindow; options: Options; decimals: number; digits: number;
  headAt: ((position: number) => number | null) | null; runtime: Runtime; hoursPerSecond: number | undefined; active: boolean;
}) {
  const settings = sunburstSettings(options);
  const colours = usePurposeColours(kind.bands);
  const { from, hour, end } = window;
  const valuesAt = (at: number): SunburstValues => ({
    items: kind.items(data, options, { from, hour: at, end }, colours),
  });
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
