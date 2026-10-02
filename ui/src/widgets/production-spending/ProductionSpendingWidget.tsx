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
import type { ProductionSpendingReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { LEVELS, UNIT, formatDeliveredIc, productionSpendingItems, spendingFinals, spendingFootnote, spendingTotal, unknownLabel, validateProductionSpendingReply } from './spendingData';

const NO_WARNINGS: string[] = [];
const ROOT = { id: 'all', label: 'All production' };

export default function ProductionSpendingWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'production-spending',
    params: { branchId: track.branch.id, tag, refits: options.refits === true } } : null;
  const { data, error, retry } = useProduct<ProductionSpendingReply>(runtime.client, request, active, validateProductionSpendingReply);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const range = widget.time.mode === 'range';
  const from = range ? window?.firstHour ?? null : null;
  const end = range ? hour : track?.branch.lastHour;
  const finals = useMemo(() => data && end != null ? spendingFinals(data, from, end) : [], [data, from, end]);
  const items = useMemo(() => data && hour != null && end != null ? productionSpendingItems(data, { from, hour, end }, finals) : [], [data, from, hour, end, finals]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const decimals = data?.decimals ?? 5;
  const text = useMemo<SunburstText>(() => ({
    format: (value, exact) => formatDeliveredIc(value, exact, decimals), unit: UNIT, valueLabel: 'Delivered IC at read cost', countLabel: 'Designs', unknownLabel,
  }), [decimals]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its production spending." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading production spending…" />;
  if (!data.reads.length) return <WidgetState kind="unavailable" message="Not known: production output was not recorded in this campaign branch." />;
  if (hour == null || end == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;
  return <Sunburst label="Production spending" items={items} root={ROOT} levels={levels} text={text} settings={settings}
    authoritativeTotal={spendingTotal(data, { from, hour, end })} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
    active={active} footnote={spendingFootnote(data, { from, hour, end }, settings.growth)} />;
}
