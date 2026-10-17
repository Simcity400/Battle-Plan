import { useEffect, useMemo, useRef } from 'react';
import { useParityRows } from '../../dashboard/parityRows';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { stateNameRequests } from '../../dashboard/construction';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { dayLabel } from '../../charts/draw';
import { approximateAt, preparedPrefix, preparedThrough } from '../../dashboard/preparedPrefix';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { icDecimals } from '../../dashboard/icDecimals';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import type { ConstructionSpendingReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { LEVELS, footnote, formatSpent, hasConstruction, scaleDigits, spendingItems, validateSpendingReply } from './spendingData';

const NO_WARNINGS: string[] = [];
const ROOT = { id: 'all', label: 'All construction' };

/** Where one country's civilian-factory output went, as a sunburst:
 * new buildings or repairs, then building type, then state. With Live on it
 * grows at recorded factory-output reads; off, the whole campaign. */
export default function ConstructionSpendingWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
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
  const prepared = !!runtime.client.preparedQuery;
  const hours = [...new Set([from == null ? null : from - 1, hour, end].filter((value): value is number => value != null))].sort((a, b) => a - b);
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'construction-spending',
    params: { branchId: track.branch.id, tag, ...(prepared ? { hours } : {}) } } : null;
  const { data, error, retry } = useProduct<ConstructionSpendingReply>(runtime.client, request, active, validateSpendingReply, prepared ? track?.source : undefined);
  const namingActive = active && data != null && hour != null && track != null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const wantedStates = useMemo(() => stateNameRequests(data?.leaves ?? [], namingActive ? hour ?? null : null), [data, hour, namingActive]);
  const stateAt = useStateNamesAt(runtime.client, track, wantedStates, namingActive);
  const items = useMemo(() => data && hour != null && end != null ? spendingItems(data, { from, hour, end }, stateAt) : [], [data, from, hour, end, stateAt]);
  const parityRoot = useRef<HTMLDivElement>(null);
  useParityRows(parityRoot, () => ({ kind: 'chart', label: 'Construction spending values', complete: data != null, total: items.length,
    rows: items.map(item => ({ path: item.path, exact: item.exact?.toString() ?? null, final: item.final, present: item.present, unknown: item.unknown, counted: item.counted })),
  }));
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  // Before preparation reaches the hour shown, the reply's totals are those of
  // its prepared prefix; later output is still being prepared.
  const through = data && hour != null ? preparedThrough([data], hour) : hour;
  const preparedNote = [through != null && hour != null && through < hour ? `Spent through ${dayLabel(through)}; the rest is still being prepared.` : null,
    through != null && approximateAt(preparedPrefix(data), through) ? 'Approximate work intervals and building names while preparation continues.' : null].filter(Boolean).join(' ') || undefined;
  const scale = data?.costScale ?? 1;
  const digits = icDecimals(options, scaleDigits(scale));
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact) => formatSpent(value, exact, scale, digits), unit: 'IC', valueLabel: 'Spent', countLabel: 'Projects',
  }), [scale, digits]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its construction spending." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading construction spending…" />;
  if (!data.reads.length && !data.railwayReads.length && preparedPrefix(data)?.pending.length) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!data.reads.length && !data.railwayReads.length) return <WidgetState kind="unavailable" message="Construction was not recorded." />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (from != null && through != null && from > through) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!hasConstruction(items)) return <WidgetState kind="empty" icon="sunburst" message="No construction recorded" detail={footnote(data, items, preparedNote)} />;
  return <div ref={parityRoot} style={{ height: '100%' }}><Sunburst label="Construction spending" items={items} root={ROOT} levels={levels} text={text} settings={settings}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} footnote={footnote(data, items, preparedNote)} /></div>;
}
