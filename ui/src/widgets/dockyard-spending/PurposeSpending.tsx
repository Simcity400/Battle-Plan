import { useContext, useEffect, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstItem, SunburstKey, SunburstText } from '../../charts/sunburst/layout';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import TimeChart from '../../charts/TimeChart';
import Legend from '../../charts/Legend';
import { PreparedKey } from '../../charts/PreparedKey';
import { pendingLayer } from '../../charts/prepared';
import { columnAt, columnsLayer } from '../../charts/columns';
import type { ColumnPeriod } from '../../charts/columns';
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { useChartTokens } from '../../charts/useChartTokens';
import { dayLabel } from '../../charts/draw';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { approximateAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useParityRows } from '../../dashboard/parityRows';
import { NumericReply } from '../../dashboard/numericReply';
import type { ProductionSpendingReply } from '../../generated/dashboard';
import { UNIT, formatAppliedIc, noProductionAt, preparedWindow } from '../production-spending/spendingData';
import type { SpendingWindow } from '../production-spending/spendingData';
import { VIEWS } from './definition';
import type { DockyardViewChoice } from './definition';
import { LEVELS, formatExactIc, monthLabel, reasonLabel } from './dockyardData';
import type { Exact } from './dockyardData';
import './dockyardSpending.css';

/** One month's band amounts and what its tooltip adds. */
export interface PurposeMonth { firstHour: number; lastHour: number; bands: readonly Exact[]; total: Exact; note?: string }

/** A widget of IC spent by purpose: Dockyard spending or Military factory
 * spending. Both read a production spending reply split by purpose, with
 * one column of purpose amounts per month. */
export interface PurposeSpendingKind<R extends ProductionSpendingReply & { months: readonly unknown[] }> {
  product: string;
  /** The widget's name, as its charts and view switch are labelled. */
  name: string;
  /** What the amounts are, such as "dockyard IC". */
  measure: string;
  root: SunburstKey;
  /** The months' bands, in column order; the ring's purposes take their
   * colours too. */
  bands: readonly { id: string; label: string }[];
  options: readonly OptionField[];
  /** Request parameters the widget's settings add. */
  params?: (options: Readonly<Record<string, OptionValue>>) => Record<string, unknown>;
  validate: (data: unknown) => R;
  /** Whether the reply holds amounts besides production reads. */
  read?: (data: R) => boolean;
  items: (data: R, window: SpendingWindow, colours: readonly string[]) => SunburstItem[];
  /** The reply's own amount shown beside production, known zero or not: an
   * empty chart says why. */
  other?: (data: R) => Exact;
  columns: (data: R, decimals: number) => ColumnPeriod[];
  month: (data: R, index: number) => PurposeMonth;
  /** Each month's values for the parity rows. */
  monthRow: (data: R, index: number) => Record<string, unknown>;
  note?: string;
}

const NO_WARNINGS: string[] = [];
const sentence = (text: string) => text[0].toUpperCase() + text.slice(1);

export function PurposeSpendingWidget<R extends ProductionSpendingReply & { months: readonly unknown[] }>(
  { kind, widget, runtime, active, onDiagnostics, headerSlot, setOption }: WidgetProps & { kind: PurposeSpendingKind<R> }) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(kind.options, widget.options);
  const view: DockyardViewChoice = options.view === 'months' ? 'months' : 'circle';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: kind.product,
    params: { branchId: track.branch.id, tag, view, ...kind.params?.(options), from, hour, end } } : null;
  const product = useProduct<R>(runtime.client, request, active, kind.validate, track?.source);
  const retained = useMemo(() => new NumericReply<R>(), [runtime.client]);
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
    onChange={value => setOption('view', value)} label={`${kind.name} view`} />, slot);

  const read = data && (data.reads.length > 0 || kind.read?.(data) === true);
  let body: ReactNode;
  if (!track || !tag) body = <WidgetState kind="empty" icon="sunburst" message={`Load a campaign and choose a country to see its ${kind.name.toLowerCase()}.`} />;
  else if (error) body = <WidgetState kind="error" detail={error} onRetry={retry} />;
  else if (!data) body = <WidgetState kind="loading" label={`Loading ${kind.name.toLowerCase()}…`} />;
  else if (!read && preparedPrefix(data)?.pending.length) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (!read) body = <WidgetState kind="unavailable" message={`${kind.name} wasn’t recorded.`} />;
  else if (hour == null || end == null || hour < track.branch.firstHour || hour > track.branch.lastHour) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (!shown) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (view === 'months') body = <PurposeMonths kind={kind} data={data} extent={[from ?? track.branch.firstHour, range ? hour : track.branch.lastHour]}
    playhead={widget.time.mode === 'follow' ? shown.hour : null} branchStart={track.branch.firstHour} decimals={decimals} digits={digits}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} />;
  else body = <PurposeCircle kind={kind} data={data} shown={shown} requestedHour={hour} options={options} decimals={decimals} digits={digits}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} />;
  return <>{header}{body}</>;
}

/** Each band takes a category colour by its place, in both views. */
function usePurposeColours(bands: readonly unknown[]): string[] {
  const tokens = useChartTokens();
  return useMemo(() => bands.map((_, index) => tokens.categories[index % tokens.categories.length]), [tokens, bands]);
}

export function PurposeCircle<R extends ProductionSpendingReply & { months: readonly unknown[] }>({ kind, data, shown, requestedHour, options, decimals, digits, hoursPerSecond, active }: {
  kind: PurposeSpendingKind<R>; data: R; shown: SpendingWindow; requestedHour: number;
  options: ReturnType<typeof resolveOptions>; decimals: number; digits: number; hoursPerSecond: number; active: boolean;
}) {
  const settings = sunburstSettings(options);
  const colours = usePurposeColours(kind.bands);
  const items = useMemo(() => kind.items(data, shown, colours), [kind, data, shown, colours]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: `${kind.name} values`, complete: true,
    total: items.length, rows: items.map(item => ({ path: item.path.map(part => part.id), exact: item.exact?.toString() ?? null,
      final: item.final, unknownValue: item.unknownValue, finalUnknown: item.finalUnknown, present: item.present })) }));
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const other = kind.other?.(data) ?? { exact: 0n, reason: null };
  const empty = !items.some(item => item.present);
  const emptyLabel = other.exact == null ? reasonLabel(other.reason) : `No ${kind.measure} spent in this view.`;
  const footnote = [shown.hour < requestedHour ? `Spent through ${dayLabel(shown.hour)}; the rest is still being prepared.` : null,
    approximateAt(preparedPrefix(data), shown.hour) ? 'Approximate equipment types while preparation continues.' : null].filter(Boolean).join(' ') || undefined;
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact, denominator) => exact == null ? formatAppliedIc(value, null, decimals, digits)
      : formatExactIc(exact, denominator ?? 1n, decimals, digits), unit: UNIT, valueLabel: sentence(kind.measure), countLabel: 'Designs',
    emptyLabel,
  }), [decimals, digits, emptyLabel, kind.measure]);
  return <div ref={root} style={{ display: 'contents' }}>
    {empty && noProductionAt(data, shown.hour) && other.exact === 0n
      ? <WidgetState kind="empty" icon="sunburst" message={`No ${kind.measure} spent yet.`} detail={footnote} />
      : <Sunburst label={kind.name} items={items} root={kind.root} levels={levels} text={text} settings={settings}
        hoursPerSecond={hoursPerSecond} active={active} footnote={footnote} />}
  </div>;
}

export function PurposeMonths<R extends ProductionSpendingReply & { months: readonly unknown[] }>({ kind, data, extent, playhead, branchStart, decimals, digits, hoursPerSecond, active }: {
  kind: PurposeSpendingKind<R>; data: R; extent: [number, number]; playhead: number | null; branchStart: number;
  decimals: number; digits: number; hoursPerSecond: number; active: boolean;
}) {
  const colours = usePurposeColours(kind.bands);
  const columns = useMemo(() => kind.columns(data, decimals), [kind, data, decimals]);
  const pending = preparedPrefix(data)?.pending ?? [];
  const layers = useMemo(() => [...(pending.length ? [pendingLayer(pending)] : []), columnsLayer(columns, colours)], [pending, columns, colours]);
  const keyHours = useMemo(() => columns.map(column => column.firstHour), [columns]);
  const root = useRef<HTMLDivElement>(null);
  const label = `${kind.name} by month`;
  useParityRows(root, () => ({ kind: 'chart', label, complete: true, total: data.months.length,
    rows: data.months.map((_, index) => kind.monthRow(data, index)) }));
  const write = ({ exact, reason, denominator }: Exact) => exact == null ? reasonLabel(reason)
    : `${formatExactIc(exact, denominator ?? 1n, decimals, digits)} ${UNIT}`;
  return <div className="dockyard-months" ref={root}>
    <Legend label={`${sentence(kind.measure)} spent on`} always items={kind.bands.map((band, index) => ({ id: band.id, label: band.label, color: colours[index] }))} />
    <PreparedKey pending={pending} approximate={preparedPrefix(data)?.approximate ?? []} />
    <div className="dockyard-months-chart">
      <TimeChart label={label} extent={extent} playhead={playhead} playback="draw-in" branchStart={branchStart}
        hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        fitValues={shown => shown} layers={layers} keyHours={keyHours} active={active} revision={{ columns, colours, pending, digits }}
        tooltip={hour => {
          const index = columnAt(columns, Math.floor(hour));
          if (index < 0) return null;
          const month = kind.month(data, index);
          return {
            title: monthLabel(month.firstHour),
            rows: [
              ...month.bands.map((band, index) => ({ label: kind.bands[index].label, swatch: colours[index], value: write(band) })),
              { label: 'Total', strong: true, divider: true, value: write(month.total) },
            ],
            note: [month.note, kind.note].filter(Boolean).join(' ') || undefined,
          };
        }} />
    </div>
  </div>;
}
