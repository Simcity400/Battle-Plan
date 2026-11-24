import { useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import TimeChart from '../../charts/TimeChart';
import Legend from '../../charts/Legend';
import { columnAt } from '../../charts/columns';
import { liveAt } from '../../charts/live';
import type { Live } from '../../charts/live';
import { useChartTokens } from '../../charts/useChartTokens';
import { firstHourOfMonth, gameDate, monthIndex } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { UNIT, spendingRecorded, spendingWidgetGate } from '../production-spending/spendingData';
import type { SpendingData } from '../production-spending/spendingData';
import { useSpending } from '../production-spending/useSpending';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import type { Domain, ProductionSpendingReply } from '../../generated/dashboard';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import { dockyardSpendingProjection, militaryFactorySpendingProjection, type PurposeSpendingProjectionOptions } from './purposeSpendingProjection.ts';
import {
  LEVELS,
  createLiveMonthCache,
  formatExactIc,
  headMonthLayer,
  monthColumns,
  monthLabel,
  monthSpansFor,
  monthlyHeadKey,
  reasonLabel,
  shownMonthBands,
  visibleMonths,
  type Exact,
  type PurposeMonth,
} from './dockyardData';
import './dockyardSpending.css';

type Options = Readonly<Record<string, OptionValue>>;

export interface PurposeSpendingKind {
  name: string;
  measure: string;
  mode: 'dockyard' | 'military';
  domain: (options: Options) => Domain;
  bands: readonly { id: string; label: string }[];
  options: readonly OptionField[];
  repairs: boolean;
  validate: (data: unknown) => ProductionSpendingReply;
  read?: (data: SpendingData, at?: number) => boolean;
  month: (data: SpendingData, options: Options, first: number, last: number, branchFirst: number) => PurposeMonth;
  note?: string;
}

const NO_WARNINGS: string[] = [];
const EMPTY_SUBS = [] as const;
const sentence = (text: string) => text[0].toUpperCase() + text.slice(1);

export function PurposeSpendingWidget({ kind, widget, runtime, active, onDiagnostics, setOption }: WidgetProps & { kind: PurposeSpendingKind }) {
  const options = resolveOptions(kind.options, widget.options);
  const optionsKey = JSON.stringify(options);
  const isMonths = options.view === 'months';
  const breakdownView = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const headKey = useRef<(hour: number) => string>(() => '');
  const liveMonthCache = useMemo(() => createLiveMonthCache(kind.month), [kind]);
  const spending = useSpending(widget, runtime, active, kind.repairs, kind.validate, hour => headKey.current(hour), isMonths);
  useShownEachFrame(runtime.clock, () => isMonths && spending.data && spending.track ? trackHourOf(widget, spending.track, runtime.tracks, runtime.clock.getHour()) : null);
  const { track, tag, data, error, retry, hour, from, end, headAt } = spending;
  const following = widget.time.mode === 'follow';
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);

  useEffect(() => {
    if (!isMonths) return;
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [isMonths, data, onDiagnostics]);

  headKey.current = at => {
    if (!data || !track || end == null) return isMonths ? 'months' : breakdownView;
    const floored = Math.floor(at);
    const read = spendingRecorded(data) || kind.read?.(data, floored) === true;
    if (isMonths) {
      const date = gameDate(floored);
      const bucket = date ? monthIndex(date) : floored;
      const monthStart = date ? Math.max(from ?? track.branch.firstHour, firstHourOfMonth(bucket)) : floored;
      const current = liveMonthCache(data, options, optionsKey, monthStart, floored, track.branch.firstHour);
      return monthlyHeadKey(bucket, read, kind.bands, current);
    }
    return read ? '1' : '0';
  };
  const hoursPerSecond = usePlaybackMotion(runtime.clock);

  const read = Boolean(data && (spendingRecorded(data) || kind.read?.(data, hour) === true));
  const gate = spendingWidgetGate({
    hasTarget: Boolean(track && tag),
    hasError: Boolean(error),
    branch: track?.branch,
    hour,
    end,
    hasData: Boolean(data),
    recorded: read,
  });

  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const domain = kind.domain(options);

  const projectionOptions = useMemo<PurposeSpendingProjectionOptions>(() => ({
    mode: kind.mode,
    domain,
    from,
    firstHour: track?.branch.firstHour ?? 0,
    digits,
    levels,
  }), [kind.mode, domain, from, track?.branch.firstHour, digits, levels]);

  const staticSources = useMemo(() => {
    if (!data || !track) return null;
    return {
      panels: { spending: data },
      hour: track.branch.firstHour,
      endHour: track.branch.lastHour,
    };
  }, [data, track]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  if (isMonths) {
    if (gate === 'empty') return <WidgetState kind="empty" icon="sunburst" message={`Load a campaign and choose a country to see its ${kind.name.toLowerCase()}.`} />;
    if (gate === 'error') return <WidgetState kind="error" failure={error!} onRetry={retry} />;
    if (gate === 'outside') return <WidgetState kind="outside" />;
    if (gate === 'loading') return <WidgetState kind="loading" label={`Loading ${kind.name.toLowerCase()}…`} />;
    if (gate === 'unavailable') return <WidgetState kind="unavailable" message={`${kind.name} was not recorded.`} />;
    return (
      <PurposeMonths
        kind={kind}
        data={data!}
        options={options}
        optionsKey={optionsKey}
        liveMonth={liveMonthCache}
        extent={[from ?? track!.branch.firstHour, widget.time.mode === 'range' ? hour! : track!.branch.lastHour]}
        last={hour!}
        playhead={headAt ? hour! : null}
        headAt={headAt}
        runtime={runtime}
        branchStart={track!.branch.firstHour}
        decimals={decimals}
        digits={digits}
        hoursPerSecond={hoursPerSecond}
        active={active}
      />
    );
  }

  const blocked = gate === 'empty'
    ? <WidgetState kind="empty" icon="sunburst" message={`Load a campaign and choose a country to see its ${kind.name.toLowerCase()}.`} />
    : gate === 'error'
      ? <WidgetState kind="error" failure={error!} onRetry={retry} />
      : gate === 'outside'
        ? <WidgetState kind="outside" />
        : gate === 'loading'
          ? <WidgetState kind="loading" label={`Loading ${kind.name.toLowerCase()}…`} />
          : null;

  return (
    <FamilyBreakdownHost
      declaration={kind.mode === 'dockyard' ? dockyardSpendingProjection : militaryFactorySpendingProjection}
      options={projectionOptions}
      view={breakdownView}
      namesInRing={namesInRing}
      label={kind.name}
      subject={kind.name.toLowerCase()}
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={EMPTY_SUBS}
      staticSources={staticSources}
      offset={track?.axisOffset ?? 0}
      pin={!following && hour != null ? Math.floor(hour) : null}
      bounds={bounds}
      onSetOption={setOption}
      onDiagnostics={onDiagnostics}
      blockedState={blocked}
      loadingLabel={`Loading ${kind.name.toLowerCase()}…`}
    />
  );
}

function usePurposeColours(bands: readonly unknown[]): string[] {
  const tokens = useChartTokens();
  return useMemo(() => bands.map((_, index) => tokens.categories[index % tokens.categories.length]), [tokens, bands]);
}

type Runtime = WidgetProps['runtime'];

export function PurposeMonths({ kind, data, options, optionsKey, liveMonth, extent, last, playhead, headAt, runtime, branchStart, decimals, digits, hoursPerSecond, active }: {
  kind: PurposeSpendingKind; data: SpendingData; options: Options; optionsKey?: string;
  liveMonth?: (data: SpendingData, options: Options, optionsKey: string, first: number, last: number, branchFirst: number) => PurposeMonth;
  extent: [number, number]; last: number; playhead: number | null;
  headAt: ((position: number) => number | null) | null; runtime: Runtime; branchStart: number;
  decimals: number; digits: number; hoursPerSecond: number | undefined; active: boolean;
}) {
  const colours = usePurposeColours(kind.bands);
  const first = extent[0];
  const resolvedOptionsKey = optionsKey ?? JSON.stringify(options);
  const spans = useMemo(() => monthSpansFor(first, last, extent[1], playhead != null), [first, last, extent[1], playhead]);
  const months = useMemo(() => spans.map(span => {
    const to = Math.min(span.lastHour, last);
    const computed = kind.month(data, options, span.firstHour, to, branchStart);
    return to < span.lastHour ? { ...computed, lastHour: span.lastHour } : computed;
  }), [kind, data, options, spans, last, branchStart]);
  const columns = useMemo(() => monthColumns(months, decimals), [months, decimals]);
  const head = useMemo<Live<number> | undefined>(() => {
    if (!headAt || playhead == null) return undefined;
    const live = liveAt(runtime.clock, position => Math.floor(headAt(position) ?? playhead), at => at);
    return { read: live.hour, subscribe: live.subscribe };
  }, [headAt, runtime.clock, playhead]);
  const monthThrough = (index: number, hour: number) => {
    const span = spans[index];
    const to = Math.min(span.lastHour, hour);
    const computed = liveMonth
      ? liveMonth(data, options, resolvedOptionsKey, span.firstHour, to, branchStart)
      : kind.month(data, options, span.firstHour, to, branchStart);
    return { ...computed, lastHour: span.lastHour };
  };
  const layers = useMemo(() => [headMonthLayer(columns, colours, (index, hour) => monthColumns([monthThrough(index, hour)], decimals)[0],
    () => head?.read() ?? playhead)],
    [columns, colours, spans, data, options, resolvedOptionsKey, liveMonth, branchStart, decimals, head, playhead]);
  const keyHours = useMemo(() => columns.map(column => column.firstHour), [columns]);
  const label = `${kind.name} by month`;
  const write = ({ exact, reason, denominator }: Exact) => exact == null ? reasonLabel(reason)
    : `${formatExactIc(exact, denominator ?? 1n, decimals, digits)} ${UNIT}`;
  const shownBands = useMemo(() => shownMonthBands(kind.bands, visibleMonths(months, playhead ?? last, monthThrough)),
    [kind.bands, months, playhead, last]);
  return (
    <div className="dockyard-months">
      <Legend label={`${sentence(kind.measure)} spent on`} always items={shownBands.map(({ band, index }) => ({ id: band.id, label: band.label, color: colours[index] }))} />
      <div className="dockyard-months-chart">
        <TimeChart
          label={label}
          extent={extent}
          playhead={playhead}
          liveHead={head}
          playback="draw-in"
          branchStart={branchStart}
          hoursPerSecond={hoursPerSecond}
          axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
          fitValues={shown => shown}
          layers={layers}
          keyHours={keyHours}
          active={active}
          revision={{ columns, colours, digits }}
          tooltip={at => {
            const index = columnAt(columns, Math.floor(at));
            if (index < 0) return null;
            const headHour = head?.read() ?? playhead;
            const flooredHead = headHour == null ? null : Math.floor(headHour);
            if (flooredHead != null && flooredHead < spans[index].firstHour) return null;
            const tipBands = shownMonthBands(kind.bands, visibleMonths(months, flooredHead ?? last, monthThrough));
            const month = flooredHead != null && flooredHead >= spans[index].firstHour && flooredHead <= spans[index].lastHour
              ? monthThrough(index, flooredHead) : months[index];
            return {
              title: monthLabel(month.firstHour),
              rows: [
                ...tipBands.map(({ band, index: row }) => ({ label: band.label, swatch: colours[row], value: write(month.bands[row]) })),
                { label: 'Total', strong: true, divider: true, value: write(month.total) },
              ],
              note: [month.note, kind.note].filter(Boolean).join(' ') || undefined,
            };
          }}
        />
      </div>
    </div>
  );
}
