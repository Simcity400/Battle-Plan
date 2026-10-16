import { useEffect, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { ProductionSpendingReply } from '../../generated/dashboard';
import { useParityRows } from '../../dashboard/parityRows';
import { NumericReply } from '../../dashboard/numericReply';
import { OPTIONS } from './definition';
import { LEVELS, UNIT, formatAppliedIc, noProductionAt, productionSpendingItems, spendingEmptyLabel, spendingFinals, spendingTotal, validateProductionSpendingReply } from './spendingData';

const NO_WARNINGS: string[] = [];
const ROOT = { id: 'all', label: 'All production' };

export default function ProductionSpendingWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'production-spending',
    params: { branchId: track.branch.id, tag, refits: options.refits === true, ...(runtime.client.preparedQuery ? { from, hour, end } : {}) } } : null;
  const product = useProduct<ProductionSpendingReply>(runtime.client, request, active, validateProductionSpendingReply, track?.source);
  const retained = useMemo(() => new NumericReply<ProductionSpendingReply>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify(request), product.data, data => ({ ...data, warnings: [], rocketName: null,
    leaves: data.leaves.map(leaf => ({ ...leaf, name: null, names: [] })) }));
  const data = product.data ?? numbers, { error, retry } = product;
  const finals = useMemo(() => data && end != null ? spendingFinals(data, from, end) : [], [data, from, end]);
  const items = useMemo(() => data && hour != null && end != null ? productionSpendingItems(data, { from, hour, end }, finals) : [], [data, from, hour, end, finals]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: 'Production spending values', complete: !!product.data,
    total: items.length, rows: items.map(item => ({ path: item.path.map(part => part.id), exact: item.exact?.toString() ?? null,
      final: item.final, unknownValue: item.unknownValue, finalUnknown: item.finalUnknown, present: item.present })) }));
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);
  const emptyLabel = data && hour != null ? spendingEmptyLabel(data, hour, from) : 'Loading production spending…';
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
  else if (!data.reads.length) body = <WidgetState kind="unavailable" message="Production spending wasn’t recorded." />;
  else if (hour == null || end == null || hour < track.branch.firstHour || hour > track.branch.lastHour) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (noProductionAt(data, hour) && spendingTotal(data, { from, hour, end }).exact === 0n) body = <WidgetState kind="empty" icon="sunburst" message="No military production yet." />;
  else body = <Sunburst label="Production spending" items={items} root={ROOT} levels={levels} text={text} settings={settings}
    authoritativeTotal={spendingTotal(data, { from, hour, end })} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
    active={active} />;
  return <div ref={root} style={{ display: 'contents' }}>{body}</div>;
}
