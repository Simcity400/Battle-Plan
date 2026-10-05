import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { linesTooltip } from '../../charts/race/lines';
import { mixCountry } from '../../charts/race/options';
import type { RaceView } from '../../charts/race/options';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { readAt } from '../../dashboard/reads';
import type { CountryNukesReply } from '../../generated/dashboard';
import { MAX_COUNTRIES, OPTIONS } from './definition';
import { formatNukes, nuclearModel, nuclearReadNote, nuclearRuns, nuclearSegmentAt, nuclearTooltip, nukesRequest, rankNukes, validateNukesReply } from './nuclearArsenalData';

const NO_WARNINGS: string[] = [];
export default function NuclearArsenalWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view as RaceView;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, MAX_COUNTRIES),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const request = track && (resolved.top != null || resolved.tags.length) ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-nukes', params: nukesRequest(track.branch.id, resolved) } : null;
  const { data, error, retry } = useProduct<CountryNukesReply>(runtime.client, request, active, validateNukesReply);
  const model = useMemo(() => data ? nuclearModel(data) : null, [data]);
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  const ranked = useMemo(() => model ? rankNukes(model, rankedCandidates(resolved, moment, model.tags), moment, slots, resolved.top != null) : [],
    [model, resolved, moment, slots]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => !model ? [] : (resolved.top != null ? model.tags : resolved.tags).flatMap(tag => {
    const runs = nuclearRuns(model, tag, resolved.membership?.get(tag) ?? null, range);
    return runs.length ? [{ tag, runs }] : [];
  }), [model, resolved, range]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const mix = useMemo(() => model && mixTag ? { tag: mixTag, bands: model.reply.types
    .map(type => nuclearRuns(model, mixTag, resolved.membership?.get(mixTag) ?? null, range, type.ordinal)) } : null,
  [model, mixTag, resolved.membership, range]);
  const segmentAt = useCallback((tag: string, hour: number) => nuclearSegmentAt(model, tag, hour), [model]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(resolved.dropped ? [`${resolved.dropped} more chosen countries are outside this widget’s current selection limit.`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, resolved.dropped, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its nuclear arsenal." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" icon="target" message={resolved.reason} />;
  if (!model || !data || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading nuclear arsenal…" />;
  if (!model.tags.length && resolved.top == null) return <WidgetState kind="unavailable" message="No nuclear stockpiles were recorded for these countries." />;
  return <CountryRaceView runtime={runtime} track={track} active={active} options={options} view={view} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={ranked} lines={lines} parts={model.parts} mix={mix} segmentAt={segmentAt}
    label="Nuclear stockpile (game counter)" empty={resolved.top != null && readAt(data.emptyReadHours, moment) === moment
      ? 'No nuclear weapons yet.' : 'No nuclear stockpile was recorded at this time.'} formatValue={formatNukes}
    tooltip={(tag, hour, name, colours) => {
      const membership = model.series.get(tag)?.membership ?? resolved.membership?.get(tag);
      return isMember(membership, hour) ? nuclearTooltip(model, tag, hour, name, colours, formatDateTime(hour)) : null;
    }} lineTooltip={(hour, tags, labelAt, country) => [{
      ...linesTooltip(hour, lines.filter(line => tags.includes(line.tag)), labelAt, country, formatDateTime(hour), 'Nuclear stockpile (game counter)', formatNukes),
      note: tags.length ? nuclearReadNote(model, hour) : undefined,
    }]} />;
}
