import { useEffect, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText, SunburstValues } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { liveAt } from '../../charts/live';
import type { WidgetProps } from '../../dashboard/definitions';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { OPTIONS } from './definition';
import type { FactoryKind } from './definition';
import { LEVELS, UNIT, formatAppliedIc, noProductionAt, productionSpendingItems, spendingDesigns, spendingEmptyLabel, spendingFinals, spendingTotal,
  spendingRecorded, validateProductionSpendingReply } from './spendingData';
import { useSpending } from './useSpending';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';

const NO_WARNINGS: string[] = [];
const ROOTS: Record<FactoryKind, { id: string; label: string }> = {
  military: { id: 'all', label: 'All military production' }, dockyards: { id: 'all', label: 'All naval production' },
};

export default function ProductionSpendingWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const kind: FactoryKind = options.factories === 'dockyards' ? 'dockyards' : 'military';
  const refits = options.refits === true;
  const headKey = useRef<(hour: number) => string>(() => '');
  const spending = useSpending(widget, runtime, active, false, validateProductionSpendingReply, hour => headKey.current(hour));
  useShownEachFrame(runtime.clock, () => spending.data && spending.track ? trackHourOf(widget, spending.track, runtime.tracks, runtime.clock.getHour()) : null);
  const { track, tag, data, error, retry, hour, from, end, headAt } = spending;
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const designs = useMemo(() => data ? spendingDesigns(data, refits, kind) : [], [data, refits, kind]);
  const empty = (at: number) => !!data && noProductionAt(data, at, refits) && spendingTotal(data, designs, { from, hour: at, end: at }, kind, refits).exact === 0n;
  const emptyLabel = (at: number) => data ? spendingEmptyLabel(data, designs, at, from, kind, refits) : 'Loading production spending…';
  headKey.current = at => JSON.stringify([empty(at), emptyLabel(at)]);
  const valuesAt = (at: number): SunburstValues => !data || end == null ? { items: [] } : {
    items: productionSpendingItems(data, { from, hour: at, end: at }, designs, spendingFinals(data, designs, from, at), kind, refits),
    authoritativeTotal: spendingTotal(data, designs, { from, hour: at, end: at }, kind, refits),
  };
  const build = useRef(valuesAt);
  build.current = valuesAt;
  const values = useMemo(() => hour == null ? { items: [] } : valuesAt(hour), [data, designs, kind, refits, from, hour, end]);
  const live = useMemo(() => headAt && data && hour != null ? liveAt(runtime.clock, position => Math.floor(headAt(position) ?? hour), at => build.current(at)) : undefined,
    [headAt, runtime.clock, data, designs, kind, refits, from, end]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);
  const label = hour == null ? 'Loading production spending…' : emptyLabel(hour);
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact) => formatAppliedIc(value, exact, decimals, digits), unit: UNIT, valueLabel: 'IC spent', countLabel: 'Designs',
    emptyLabel: label,
  }), [decimals, digits, label]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  let body: ReactNode;
  if (!track || !tag) body = <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its production spending." />;
  else if (error) body = <WidgetState kind="error" failure={error} onRetry={retry} />;
  else if (!data) body = <WidgetState kind="loading" label="Loading production spending…" />;
  else if (!spendingRecorded(data)) body = <WidgetState kind="unavailable" message="Production spending wasn’t recorded." />;
  else if (hour == null || end == null || hour < track.branch.firstHour || hour > track.branch.lastHour) body = <WidgetState kind="outside" />;
  else if (empty(hour)) body = <WidgetState kind="empty" icon="sunburst" message="No military production yet." />;
  else body = <Sunburst label={widget.title} items={values.items} root={ROOTS[kind]} levels={levels} text={text} settings={settings}
    authoritativeTotal={values.authoritativeTotal} live={live} hoursPerSecond={hoursPerSecond} active={active} />;
  return body;
}
