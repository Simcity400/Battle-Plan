import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { mixCountry } from '../../charts/race/options';
import type { RaceView } from '../../charts/race/options';
import { formatAxisPercent } from '../../charts/valueAxis';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, rankedCandidates, resolveCountries, worldRequestTags } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { OccupationTrendReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { measureName, occupationEntries, occupationLines, occupationLineTooltip, occupationModel, occupationRequest, occupationSegmentAt,
  occupationTooltip, occupationTooltipTags, rankingMeasure, validateOccupationTrendReply } from './occupationTrendData';
import type { OccupationMeasure, ShownMeasure } from './occupationTrendData';

const NO_WARNINGS: string[] = [];
export default function OccupationTrendWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view as RaceView;
  const shown = options.measure as ShownMeasure;
  const rank = rankingMeasure(shown, options.rankingMeasure as OccupationMeasure);
  const collaboration = options.collaboration === true;
  const digits = Number(options.digits);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const all = widget.countries.kind === 'all';
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const requested = useMemo(() => worldRequestTags(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const request = track && requested.ready && !requested.reason ? { sourceId: track.source.id, revision: track.source.revision, product: 'occupation-trend',
    params: occupationRequest(track.branch.id, requested.tags, resolved.top, rank, collaboration) } : null;
  const { data, error, retry } = useProduct<OccupationTrendReply>(runtime.client, request, active, validateOccupationTrendReply);
  const model = useMemo(() => data ? occupationModel(data) : null, [data]);
  const slots = resolved.top ?? Math.max(1, all ? model?.tags.length ?? 0 : resolved.tags.length);
  const ranked = useMemo(() => model ? occupationEntries(model, all ? model.tags : rankedCandidates(resolved, moment, model.tags), moment, rank, slots, digits) : [],
    [model, all, resolved, moment, rank, slots, digits]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
  const lines = useMemo(() => model ? occupationLines(model, all || resolved.top != null ? model.tags : resolved.tags,
    shown, range, resolved.membership, digits) : [], [model, all, resolved, shown, range, digits]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const mix = useMemo(() => model && mixTag ? { tag: mixTag, lines: occupationLines(model, [mixTag], shown, range, resolved.membership, digits) }
    : null, [model, mixTag, shown, range, resolved.membership, digits]);
  const segmentAt = useCallback((tag: string, hour: number) => occupationSegmentAt(model, tag, hour), [model]);
  const relatedTags = useCallback((hour: number, tags: readonly string[]) => model ? occupationTooltipTags(model, hour, tags, collaboration) : [], [model, collaboration]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, onDiagnostics]);
  const state = !track ? <WidgetState kind="empty" icon="globe" message="Load a campaign to see occupation trends." />
    : (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window ? <WidgetState kind="unavailable" message="The chosen moment is not in this campaign." />
    : error ? <WidgetState kind="error" message={error} onRetry={retry} />
    : groups.error ? <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />
    : requested.reason ? <WidgetState kind="empty" icon="globe" message={requested.reason} />
    : !model || (grouped && !groups.data) ? <WidgetState kind="loading" label="Loading occupation trends…" /> : null;
  return <CountryRaceView runtime={runtime} track={track} active={active} options={options} view={view} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={ranked} lines={lines} mix={mix} segmentAt={segmentAt} tooltipTags={relatedTags} state={state}
    presentation={{ exact: false, held: false }}
    label={`Average ${measureName(rank).toLowerCase()}`} empty="No readable held states were recorded at this moment."
    lineEmpty={{ icon: 'globe', message: 'No readable held states were recorded for these occupiers.' }}
    mixEmpty={{ icon: 'globe', message: 'Choose an occupier with readable held states.', unavailable: name => `No readable held states were recorded for ${name}.` }}
    beforeChart={shown === 'both' && view !== 'race' ? <p className="chart-line-key"><i aria-hidden="true" />Solid: resistance · <i className="is-dashed" aria-hidden="true" />dashed: compliance</p> : null}
    formatAxis={formatAxisPercent} formatValue={String}
    tooltip={(tag, hour, _name, _colours, labelAt) => {
      if (!model || !isMember(model.series.get(tag)?.membership ?? resolved.membership?.get(tag), hour)) return null;
      const tip = occupationTooltip(model, tag, hour, labelAt, shown, collaboration);
      return tip ? [tip] : null;
    }} lineTooltip={(hour, tags, labelAt, colour) => model ? [occupationLineTooltip(model, hour, tags, labelAt, colour, shown, rank, collaboration)] : []} />;
}
