import { useContext, useEffect, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/layout';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import TimeChart from '../../charts/TimeChart';
import Legend from '../../charts/Legend';
import { PreparedKey } from '../../charts/PreparedKey';
import { pendingLayer } from '../../charts/prepared';
import { columnAt, columnsLayer } from '../../charts/columns';
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { useChartTokens } from '../../charts/useChartTokens';
import { dayLabel } from '../../charts/draw';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { approximateAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useParityRows } from '../../dashboard/parityRows';
import { NumericReply } from '../../dashboard/numericReply';
import type { DockyardSpendingReply } from '../../generated/dashboard';
import { UNIT, formatAppliedIc, noProductionAt, preparedWindow } from '../production-spending/spendingData';
import { OPTIONS, VIEWS } from './definition';
import type { DockyardViewChoice } from './definition';
import { LEVELS, PURPOSES, REPAIR_NOTE, repairsRead, dockyardItems, exactOf, monthColumns, monthLabel, monthNote, monthTotal, formatExactIc, reasonLabel, validateDockyardSpendingReply } from './dockyardData';
import './dockyardSpending.css';

const NO_WARNINGS: string[] = [];
const ROOT = { id: 'all', label: 'All dockyard IC' };

export default function DockyardSpendingWidget({ widget, runtime, active, onDiagnostics, headerSlot, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const view: DockyardViewChoice = options.view === 'months' ? 'months' : 'circle';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'dockyard-spending',
    params: { branchId: track.branch.id, tag, view, from, hour, end } } : null;
  const product = useProduct<DockyardSpendingReply>(runtime.client, request, active, validateDockyardSpendingReply, track?.source);
  const retained = useMemo(() => new NumericReply<DockyardSpendingReply>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify(request), product.data, data => ({ ...data, warnings: [],
    leaves: data.leaves.map(leaf => ({ ...leaf, name: null, names: [] })) }));
  const data = product.data ?? numbers, { error, retry } = product;
  const shown = useMemo(() => data && hour != null && end != null ? preparedWindow(data, { from, hour, end }) : null, [data, from, hour, end]);
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  // The view switch sits beside Live, where the frame never clips it; a frame
  // without header actions (the catalogue preview) shows it beside the title.
  const actions = useContext(HeaderActionsSlot);
  const slot = actions ?? headerSlot;
  const header = slot && setOption && createPortal(<Segmented value={view} options={VIEWS}
    onChange={value => setOption('view', value)} label="Dockyard spending view" />, slot);

  let body: ReactNode;
  if (!track || !tag) body = <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its dockyard spending." />;
  else if (error) body = <WidgetState kind="error" detail={error} onRetry={retry} />;
  else if (!data) body = <WidgetState kind="loading" label="Loading dockyard spending…" />;
  else if (!data.reads.length && !repairsRead(data) && preparedPrefix(data)?.pending.length) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (!data.reads.length && !repairsRead(data)) body = <WidgetState kind="unavailable" message="Dockyard spending wasn’t recorded." />;
  else if (hour == null || end == null || hour < track.branch.firstHour || hour > track.branch.lastHour) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (!shown) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (view === 'months') body = <DockyardMonths data={data} extent={[from ?? track.branch.firstHour, range ? hour : track.branch.lastHour]}
    playhead={widget.time.mode === 'follow' ? shown.hour : null} branchStart={track.branch.firstHour} decimals={decimals} digits={digits}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} />;
  else body = <DockyardCircle data={data} shown={shown} requestedHour={hour} options={options} decimals={decimals} digits={digits}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} />;
  return <>{header}{body}</>;
}

/** New ships, refits and repairs take the first category colours in both views. */
function usePurposeColours(): string[] {
  const tokens = useChartTokens();
  return useMemo(() => PURPOSES.map((_, index) => tokens.categories[index % tokens.categories.length]), [tokens]);
}

export function DockyardCircle({ data, shown, requestedHour, options, decimals, digits, hoursPerSecond, active }: {
  data: DockyardSpendingReply; shown: { from: number | null; hour: number; end: number }; requestedHour: number;
  options: ReturnType<typeof resolveOptions>; decimals: number; digits: number; hoursPerSecond: number; active: boolean;
}) {
  const settings = sunburstSettings(options);
  const colours = usePurposeColours();
  const items = useMemo(() => dockyardItems(data, shown, colours), [data, shown, colours]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: 'Dockyard spending values', complete: true,
    total: items.length, rows: items.map(item => ({ path: item.path.map(part => part.id), exact: item.exact?.toString() ?? null,
      final: item.final, unknownValue: item.unknownValue, finalUnknown: item.finalUnknown, present: item.present })) }));
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const repairs = exactOf(data.repairs.hour);
  const empty = !items.some(item => item.present);
  const emptyLabel = repairs.exact == null ? reasonLabel(repairs.reason) : 'No dockyard IC spent in this view.';
  const footnote = [shown.hour < requestedHour ? `Spent through ${dayLabel(shown.hour)}; the rest is still being prepared.` : null,
    approximateAt(preparedPrefix(data), shown.hour) ? 'Approximate equipment types while preparation continues.' : null].filter(Boolean).join(' ') || undefined;
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact, denominator) => exact == null ? formatAppliedIc(value, null, decimals, digits)
      : formatExactIc(exact, denominator ?? 1n, decimals, digits), unit: UNIT, valueLabel: 'Dockyard IC', countLabel: 'Designs',
    emptyLabel,
  }), [decimals, digits, emptyLabel]);
  return <div ref={root} style={{ display: 'contents' }}>
    {empty && noProductionAt(data, shown.hour) && repairs.exact === 0n
      ? <WidgetState kind="empty" icon="sunburst" message="No dockyard IC spent yet." detail={footnote} />
      : <Sunburst label="Dockyard spending" items={items} root={ROOT} levels={levels} text={text} settings={settings}
        hoursPerSecond={hoursPerSecond} active={active} footnote={footnote} />}
  </div>;
}

export function DockyardMonths({ data, extent, playhead, branchStart, decimals, digits, hoursPerSecond, active }: {
  data: DockyardSpendingReply; extent: [number, number]; playhead: number | null; branchStart: number;
  decimals: number; digits: number; hoursPerSecond: number; active: boolean;
}) {
  const colours = usePurposeColours();
  const columns = useMemo(() => monthColumns(data.months, decimals, data.repairs.denominator), [data.months, decimals, data.repairs.denominator]);
  const pending = preparedPrefix(data)?.pending ?? [];
  const layers = useMemo(() => [...(pending.length ? [pendingLayer(pending)] : []), columnsLayer(columns, colours)], [pending, columns, colours]);
  const keyHours = useMemo(() => data.months.map(month => month.firstHour), [data.months]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: 'Dockyard spending by month', complete: true, total: data.months.length,
    rows: data.months.map(month => ({ path: [String(month.firstHour)], newShips: month.newShips.totalRaw, refits: month.refits.totalRaw,
      repairs: month.repairs.totalRaw, purposeUnproven: month.purposeUnproven, unresolved: month.unresolved })) }));
  const write = (exact: bigint | null, reason: string | null, denominator = 1n) => exact == null ? reasonLabel(reason)
    : `${formatExactIc(exact, denominator, decimals, digits)} ${UNIT}`;
  const repairDenominator = BigInt(data.repairs.denominator);
  return <div className="dockyard-months" ref={root}>
    <Legend label="Dockyard IC spent on" always items={PURPOSES.map((purpose, index) => ({ id: purpose.id, label: purpose.label, color: colours[index] }))} />
    <PreparedKey pending={pending} approximate={preparedPrefix(data)?.approximate ?? []} />
    <div className="dockyard-months-chart">
      <TimeChart label="Dockyard spending by month" extent={extent} playhead={playhead} playback="draw-in" branchStart={branchStart}
        hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        fitValues={shown => shown} layers={layers} keyHours={keyHours} active={active} revision={{ columns, colours, pending, digits }}
        tooltip={hour => {
          const index = columnAt(columns, Math.floor(hour));
          if (index < 0) return null;
          const month = data.months[index];
          const total = monthTotal(month, data.repairs.denominator);
          return {
            title: monthLabel(month.firstHour),
            rows: [
              ...[month.newShips, month.refits, month.repairs].map((part, band) => ({ label: PURPOSES[band].label, swatch: colours[band],
                value: write(part.totalRaw == null ? null : BigInt(part.totalRaw), part.reason, band === 2 ? repairDenominator : 1n) })),
              { label: 'Total', strong: true, divider: true, value: write(total.exact, total.reason, total.denominator) },
            ],
            note: [monthNote(month), REPAIR_NOTE].filter(Boolean).join(' '),
          };
        }} />
    </div>
  </div>;
}
