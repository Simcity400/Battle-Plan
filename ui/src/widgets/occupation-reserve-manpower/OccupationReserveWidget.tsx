import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { useCountryColours } from '../../charts/race/countryColours';
import { dayLabel } from '../../charts/draw';
import type { LiveHead } from '../../charts/TimeChart';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { SPEC } from '../../dashboard/regionTime';
import { contentKey, resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request.ts';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime';
import type { ReserveHistoryReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { holderBands, holderOf, historySources, persons, segmentAt, segmentParts, sourceTagAt, validateReserveHistory } from './occupationReserveData';
import './occupationReserve.css';

const NO_TAGS: readonly string[] = [];

export default function OccupationReserveWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const extent: [number, number] = track ? [track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)] : [0, 1];
  const liveHead = useMemo<LiveHead | undefined>(() => following && track ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, track, runtime.clock]);
  const regionAt = useRegionHour(runtime, useRegion(widget, SPEC), active);
  const pinnedHolder = options.holder ? holderOf(contentKey(String(options.holder))) : null;
  const moment = regionAt ?? track?.branch.lastHour ?? 0;
  const playhead = following || widget.time.mode === 'pin' ? moment : null;
  if (!track) return <WidgetState kind="empty" icon="flag" message="Load a campaign to see occupation manpower." />;
  return <HolderSources {...props} holder={pinnedHolder} hour={moment} extent={extent} playhead={playhead} liveHead={liveHead} />;
}

function HolderSources({ widget, runtime, active, holder, hour, extent, playhead, liveHead }: WidgetProps & {
  holder: { tag: string; id: number } | null; hour: number; extent: [number, number]; playhead: number | null; liveHead: LiveHead | undefined;
}) {
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track && holder ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'occupation-reserve-history',
    params: { branchId: track.branch.id, holder: holder.id } }) : null;
  const { data, error, retry } = useProduct<ReserveHistoryReply>(runtime.client, request, active, validateReserveHistory, track?.source);
  const sources = useMemo(() => data ? historySources(data) : [], [data]);
  const bands = useMemo(() => data ? holderBands(data, sources) : [], [data, sources]);
  const tagOf = useCallback((source: number) => data ? sourceTagAt(data, source, hour)
    ?? [...data.sources.find(item => item.id === source)?.tags ?? []].reverse().find(tag => tag.tag !== null)?.tag ?? null : null, [data, hour]);
  const sourceTags = useMemo(() => [...new Set(sources.flatMap(source => { const tag = tagOf(source); return tag === null ? [] : [tag]; }))], [sources, tagOf]);
  const looks = useCountryLooks(runtime.client, track, useMemo(() => holder ? [holder.tag, ...sourceTags] : NO_TAGS, [holder, sourceTags]), hour, active);
  const colour = useCountryColours(sourceTags, looks.color, tokens, looks);
  const segment = data ? segmentAt(data, Math.floor(hour)) : null;
  const values = segment ? segmentParts(segment, sources) : sources.map(() => undefined);
  const mix = holder ? { tag: holder.tag, bands, parts: sources.map((source, index) => {
    const tag = tagOf(source);
    const value = values[index];
    return { id: String(source), label: tag === null ? 'Source country unavailable' : looks.name(tag, hour), color: tag === null ? tokens.muted : colour(tag, hour),
      value: value === undefined ? '—' : persons(value) };
  }) } : null;
  const tooltip = (_tag: string, at: number, name: string): TooltipContent[] | null => {
    const shown = data ? segmentAt(data, Math.floor(at)) : null;
    if (!data || !shown) return null;
    if (!shown.entries) return [{ title: name, rows: [{ label: 'Reserve manpower', value: `Not recorded (${shown.unknown})` }, { label: 'As of', value: dayLabel(shown.firstHour) }] }];
    const named = (source: number) => { const tag = sourceTagAt(data, source, Math.floor(at)); return tag === null ? 'Source country unavailable' : looks.name(tag, at); };
    return [{ title: name, rows: [
      { label: 'Reserve manpower held', value: persons(shown.entries.reduce((sum, entry) => sum + Math.max(0, entry.amount), 0)) },
      ...shown.entries.map(entry => ({ label: named(entry.source), value: persons(entry.amount), indent: true })),
    ] }];
  };
  if (!track) return null;
  if (!holder) return <WidgetState kind="empty" icon="flag" message="Choose a holder: click its bar in the race." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading this holder's reserve…" />;
  const negative = segment?.entries?.filter(entry => entry.amount < 0) ?? [];
  const list = negative.length > 0 && <dl className="occupation-negative" aria-label="Negative entries">
    <dt>Negative entries</dt>
    {negative.map((entry, index) => { const tag = sourceTagAt(data, entry.source, Math.floor(hour)); return <dd key={index}><span>{tag === null ? 'Source country unavailable' : looks.name(tag, hour)}</span><span>{persons(entry.amount)}</span></dd>; })}
  </dl>;
  if (segment?.admitted && !sources.length) return <div className="occupation-holder">
    <WidgetState kind="empty" icon="flag" message={`${looks.name(holder.tag, hour)} holds no reserve manpower at this date.`} />{list}
  </div>;
  return <div className="occupation-holder">
    <CountryRaceView runtime={runtime} track={track} active={active} options={widget.options ?? {}} view="mix" extent={extent} playhead={playhead} liveHead={liveHead}
      moment={hour} slots={1} ranked={[]} lines={[]} parts={mix?.parts ?? []} mix={mix} countryTags={[holder.tag]}
      label="Reserve manpower by source country" empty="No reserve manpower recorded." formatValue={value => Math.round(value).toLocaleString('en-US')}
      labels={{ mix: name => `Reserve manpower ${name} holds by source country` }} partsLabel="Source countries"
      presentation={{ exact: true, held: true, windowedLegend: true }}
      mixEmpty={{ icon: 'flag', message: 'Choose a holder: click its bar in the race.', unavailable: name => `Reserve manpower was not recorded for ${name} at this date.` }}
      tooltip={tooltip} />
    {list}
  </div>;
}
