import { useEffect, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { dayLabel } from '../../charts/draw';
import { approximateAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useFollowedProduct } from '../../dashboard/products';
import type { ProductionSpendingReply } from '../../generated/dashboard';
import { useParityRows } from '../../dashboard/parityRows';
import { NumericReply } from '../../dashboard/numericReply';
import { OPTIONS } from './definition';
import type { FactoryKind } from './definition';
import { LEVELS, UNIT, formatAppliedIc, noProductionAt, preparedWindow, productionSpendingItems, spendingEmptyLabel, spendingFinals, spendingTotal, validateProductionSpendingReply } from './spendingData';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
const ROOTS: Record<FactoryKind, { id: string; label: string }> = {
  military: { id: 'all', label: 'All military production' }, dockyards: { id: 'all', label: 'All naval production' },
};

export default function ProductionSpendingWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const kind: FactoryKind = options.factories === 'dockyards' ? 'dockyards' : 'military';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'production-spending',
    params: { branchId: track.branch.id, tag, refits: options.refits === true, ...(runtime.client.preparedQuery ? { from, hour, end } : {}) } } : null;
  // The last window's spending stays shown while playback's newest is read.
  const product = useFollowedProduct<ProductionSpendingReply>(runtime.client, request, ['from', 'hour', 'end'], active, validateProductionSpendingReply, track?.source);
  // Numbers kept across a content refresh belong to the request without its window, and keep the window they answer.
  const lane = request && JSON.stringify([request.sourceId, request.revision, request.params.branchId, request.params.tag, request.params.refits]);
  const retained = useMemo(() => new NumericReply<ProductionSpendingReply>(), [runtime.client]);
  const numbers = lane ? retained.update(lane, product.data, data => ({ ...data, warnings: [], rocketName: null,
    leaves: data.leaves.map(leaf => ({ ...leaf, name: null, names: [] })) })) : undefined;
  const data = product.data ?? numbers, { error, retry } = product;
  const answered = product.data && product.moment ? JSON.parse(product.moment) as [number | null, number | null, number | null] : null;
  const held = useRef<{ lane: string; window: { from: number | null; hour: number | null | undefined; end: number | null | undefined } } | null>(null);
  if (lane && answered && answered[1] != null) held.current = { lane, window: { from: answered[0], hour: answered[1], end: answered[2] } };
  const asked = product.data ? answered && answered[1] != null ? { from: answered[0], hour: answered[1], end: answered[2] } : { from, hour, end }
    : numbers && held.current?.lane === lane ? held.current.window : { from, hour, end };
  // Before preparation reaches the dates shown, the reply answers its prepared
  // prefix: the amounts spent through the prefix's last hour.
  const shown = useMemo(() => data && asked.hour != null && asked.end != null ? preparedWindow(data, { from: asked.from, hour: asked.hour, end: asked.end }) : null,
    [data, asked.from, asked.hour, asked.end]);
  const finals = useMemo(() => data && shown ? spendingFinals(data, shown.from, shown.end, kind) : [], [data, shown, kind]);
  const items = useMemo(() => data && shown ? productionSpendingItems(data, shown, finals, kind) : [], [data, shown, finals, kind]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: 'Production spending values', complete: !!product.data,
    total: items.length, rows: items.map(item => ({ path: item.path.map(part => part.id), exact: item.exact?.toString() ?? null,
      final: item.final, unknownValue: item.unknownValue, finalUnknown: item.finalUnknown, present: item.present })) }));
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);
  const emptyLabel = data && shown ? spendingEmptyLabel(data, shown.hour, shown.from, kind) : 'Loading production spending…';
  const footnote = [shown && asked.hour != null && shown.hour < asked.hour ? `Spent through ${dayLabel(shown.hour)}; the rest is still being prepared.` : null,
    shown && approximateAt(preparedPrefix(data), shown.hour) ? 'Approximate equipment types while preparation continues.' : null].filter(Boolean).join(' ') || undefined;
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact) => formatAppliedIc(value, exact, decimals, digits), unit: UNIT, valueLabel: 'IC spent', countLabel: 'Designs',
    emptyLabel,
  }), [decimals, digits, emptyLabel]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  let body: ReactNode;
  if (!track || !tag) body = <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its production spending." />;
  else if (error) body = <WidgetState kind="error" detail={error} onRetry={retry} />;
  else if (!data) body = <WidgetState kind="loading" label="Loading production spending…" />;
  else if (!data.reads.length && preparedPrefix(data)?.pending.length) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (!data.reads.length) body = <WidgetState kind="unavailable" message="Production spending wasn’t recorded." />;
  else if (hour == null || end == null || hour < track.branch.firstHour || hour > track.branch.lastHour) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (!shown) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (noProductionAt(data, shown.hour) && spendingTotal(data, shown, kind).exact === 0n) body = <WidgetState kind="empty" icon="sunburst" message="No military production yet." detail={footnote} />;
  else body = <Sunburst label={widget.title} items={items} root={ROOTS[kind]} levels={levels} text={text} settings={settings}
    authoritativeTotal={spendingTotal(data, shown, kind)} hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
    active={active} footnote={footnote} />;
  return <div ref={root} style={{ display: 'contents' }}>{body}</div>;
}
