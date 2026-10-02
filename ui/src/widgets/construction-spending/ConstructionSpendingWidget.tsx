import { useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstText } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { ConstructionSpendingReply, SpendingGap } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { GAP_LABELS, LEVELS, footnote, formatSpent, spendingItems, validateSpendingReply } from './spendingData';

const NO_WARNINGS: string[] = [];
const ROOT = { id: 'all', label: 'All construction' };

/** Where one country's civilian-factory output went, as a sunburst (spec 2.4, mock-up
 * style-3): new buildings or repairs, then building type, then state. With Live on it
 * grows read by read as the queue's progress builds up; off, the whole campaign. */
export default function ConstructionSpendingWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'construction-spending',
    params: { branchId: track.branch.id, tag } } : null;
  const { data, error, retry } = useProduct<ConstructionSpendingReply>(runtime.client, request, active, validateSpendingReply);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const items = useMemo(() => data && hour != null && end != null ? spendingItems(data, { from, hour, end }) : [], [data, from, hour, end]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const scale = data?.costScale ?? 1;
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact) => formatSpent(value, exact, scale), unit: 'IC', valueLabel: 'Spent', countLabel: 'Projects',
    unknownLabel: reason => GAP_LABELS[reason as SpendingGap] ?? reason,
  }), [scale]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its construction spending." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading construction spending…" />;
  if (!data.reads.length && !data.railwayReads.length) return <WidgetState kind="unavailable" message="The construction queue was not recorded in this campaign branch." />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;
  if (!items.some(item => item.present !== false)) return <WidgetState kind="empty" icon="sunburst" message="No construction recorded"
    detail="The queue is read once a day; projects that start and finish between reads are not seen." />;
  return <Sunburst label="Construction spending" items={items} root={ROOT} levels={levels} text={text} settings={settings}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} footnote={footnote(data)} />;
}
