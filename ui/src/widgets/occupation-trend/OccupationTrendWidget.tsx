import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import CountryPicker from '../../components/CountryPicker';
import CountryRaceView from '../../charts/race/CountryRaceView';
import type { RaceView } from '../../charts/race/options';
import { formatAxisPercent } from '../../charts/valueAxis';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter, isMember, MEMBERSHIP_PREPARING, noMembersAt, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { countryDrillTagAt, countryIdentityIndex } from '../../dashboard/countryIdentity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { usePreparedOccupation } from './preparedOccupation';
import { useParityRows } from '../../dashboard/parityRows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { OPTIONS } from './definition';
import { measureName, occupationEntries, occupationHolderMix, occupationLines, occupationLineTooltip, occupationModel, occupationRequest, occupationSegmentAt,
  occupationTooltip, occupationTooltipTags, rankingMeasure } from './occupationTrendData';
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
  // The request retains literal roots. Expanding aliases here would select
  // unrelated holders which merely reused one of those aliases.
  const requested = { tags: all ? [] : resolved.tags,
    ready: all || resolved.top != null || (!grouped || !!groups.data) && resolved.tags.length > 0,
    reason: all ? null : resolved.reason };
  const request = track && requested.ready && !requested.reason ? { sourceId: track.source.id, revision: track.source.revision, product: 'occupation-trend',
    params: occupationRequest(track.branch.id, requested.tags, resolved.top, rank, collaboration) } : null;
  const prepared = usePreparedOccupation(runtime.client, track, request?.params ?? null, shown, moment, active, resolved.membership ?? undefined);
  const { data, retry } = prepared;
  const error = useHeldFailure(prepared.error, data !== undefined, retry);
  const parityRoot = useRef<HTMLDivElement>(null);
  useParityRows(parityRoot, () => ({ kind: 'chart', label: 'Occupation trend values', complete: !!data, total: data?.series.length ?? 0,
    hour: moment, rows: data?.series ?? [] }));
  const model = useMemo(() => data ? occupationModel(data) : null, [data]);
  const identities = useMemo(() => grouped && groups.data ? countryIdentityIndex(groups.data.countryIdentities) : model?.identities ?? null,
    [grouped, groups.data, model]);
  const counted = useMemo(() => projectResolvedCountries(resolved, countryMatchFilter(widget.countries, fallback ? [fallback] : [],
    groups.data, track?.branch.lastHour ?? 0, identities, track?.branch.firstHour ?? 0)),
    [resolved, widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour, identities]);
  const slots = resolved.top ?? Math.max(1, all ? model?.tags.length ?? 0 : counted.tags.length);
  const ranked = useMemo(() => model ? occupationEntries(model, all ? model.tags : rankedCandidates(counted, moment, model.tags), moment, rank, slots, digits) : [],
    [model, all, counted, moment, rank, slots, digits]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
  const lines = useMemo(() => model ? occupationLines(model, all || resolved.top != null ? model.tags : counted.tags,
    shown, range, counted.membership, digits, prepared.lineSummaries) : [], [model, all, resolved.top, counted, shown, range, digits, prepared.lineSummaries]);
  const mixScopeKey = JSON.stringify([request, widget.countries, widget.time]);
  const mixScope = useMemo(() => ({ client: runtime.client, key: mixScopeKey }), [runtime.client, mixScopeKey]);
  const [choice, setChoice] = useState<{ scope: typeof mixScope; tag: string; hour: number } | null>(null);
  const mixCandidates = useMemo(() => !model ? [] : resolved.top != null ? ranked.map(row => row.tag)
    : (all ? model.tags : rankedCandidates(counted, moment, model.tags)).filter(tag => occupationSegmentAt(model, tag, moment) !== null),
    [model, resolved.top, ranked, all, counted, moment]);
  const selected = choice?.scope === mixScope ? choice : mixCandidates.length === 1 ? { scope: mixScope, tag: mixCandidates[0], hour: moment } : null;
  const mixTag = selected?.tag, mixOriginHour = selected?.hour;
  const mix = useMemo(() => model && mixTag && mixOriginHour !== undefined ? occupationHolderMix(model, mixTag, mixOriginHour, moment, shown, range, counted.membership, digits, prepared.lineSummaries)
    : null, [model, mixTag, mixOriginHour, moment, shown, range, counted.membership, digits, prepared.lineSummaries]);
  useEffect(()=>{prepared.onSummaryMembership(view==='mix' ? mix?.membership : resolved.membership ?? undefined);},[view,mix?.membership,resolved.membership,prepared.onSummaryMembership]);
  useEffect(() => {
    if (active && view === 'mix' && choice?.scope !== mixScope && mixCandidates.length === 1) {
      setChoice({ scope: mixScope, tag: mixCandidates[0], hour: moment });
    }
  }, [active, view, choice, mixScope, mixCandidates, moment]);
  const chooserKey = JSON.stringify([mixCandidates, moment]);
  const segmentAt = useCallback((tag: string, hour: number) => occupationSegmentAt(model, tag, hour), [model]);
  const relatedTags = useCallback((hour: number, tags: readonly string[]) => model ? occupationTooltipTags(model, hour, tags, collaboration) : [], [model, collaboration]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, onDiagnostics]);
  const none = all ? null : noMembersAt(widget.countries, resolved, groups.data, moment, track?.branch.lastHour ?? 0);
  const state = !track ? <WidgetState kind="empty" icon="globe" message="Load a campaign to see occupation trends." />
    : (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window ? <WidgetState kind="unavailable" message="The chosen moment is not in this campaign." />
    : error ? <WidgetState kind="error" detail={error} onRetry={retry} />
    : grouped && groups.error && !groups.data ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />
    : requested.reason ? <WidgetState kind="empty" icon="globe" message={requested.reason} />
    // A group with no member found yet asks for nothing, so no reply will come.
    : none ? <NoMembersState reason={none.reason} icon="globe" />
    : !model || (grouped && !groups.data) ? <WidgetState kind="loading" label="Loading occupation trends…" /> : null;
  const chooserOwner = useMemo(() => ({ scope: mixScope, key: chooserKey, model, active, available: !state }),
    [mixScope, chooserKey, model, active, !state]);
  const chooserRef = useRef(chooserOwner);
  chooserRef.current = chooserOwner;
  const mixChooser = view === 'mix' && track && model && !state ? <CountryPicker countries={track.source.metadata.countries}
    allowedTags={mixCandidates} selectedTag={mix?.available ? mix.tag : null} playerCountryId={null} source={track.source} client={runtime.client}
    field flagAt={{ branchId: track.branch.id, hour: moment }} onChange={tag => {
      if (!active || state || chooserRef.current !== chooserOwner || !tag || !mixCandidates.includes(tag) || occupationSegmentAt(model, tag, moment) === null) return;
      setChoice({ scope: mixScope, tag, hour: moment });
    }} /> : null;
  return <div ref={parityRoot}>{grouped && groups.error && groups.data ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null}<CountryRaceView pending={data?.pending} lineSummaries={prepared.lineSummaries} onSummaryView={prepared.onSummaryView}
    onReachHour={prepared.onReachHour} navigation={prepared.navigation} runtime={runtime} track={track} active={active} options={options} view={view} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={ranked} lines={lines} mix={mix} segmentAt={segmentAt} tooltipTags={relatedTags} state={state}
    presentation={{ exact: false, held: false }}
    label={`Average ${measureName(rank).toLowerCase()}`} empty={counted.known(moment) ? 'No occupied-state data at this time.' : MEMBERSHIP_PREPARING}
    lineEmpty={{ icon: 'globe', message: counted.known(extent[0], extent[1]) ? 'No occupied-state data for these countries.' : MEMBERSHIP_PREPARING }}
    mixEmpty={{ icon: 'globe', message: counted.known(moment) ? 'Choose an occupier.' : MEMBERSHIP_PREPARING, unavailable: () => 'No occupied-state data at this time.' }}
    countryTags={view === 'mix' ? [...new Set(mix?.lines.map(line => line.tag) ?? [])] : undefined}
    beforeChart={<>{data?.preparing?.length ? <p className="chart-line-key">Country identity history is still being prepared.</p> : null}{mixChooser}{view === 'mix' && mix && !mix.available ? <p className="chart-line-key">Country unavailable at this time.</p> : null}
      {shown === 'both' && view !== 'race' ? <p className="chart-line-key"><i aria-hidden="true" />Solid: resistance · <i className="is-dashed" aria-hidden="true" />dashed: compliance</p> : null}</>}
    formatAxis={formatAxisPercent} formatValue={String}
    tooltip={(tag, hour, _name, _colours, labelAt) => {
      const alias = view === 'mix' && mix ? countryDrillTagAt(mix.membership, hour) : tag;
      if (!model || !alias || !isMember(model.series.get(alias)?.membership ?? counted.membership?.get(alias), hour)) return null;
      const tip = occupationTooltip(model, alias, hour, labelAt, shown, collaboration);
      return tip ? [tip] : null;
    }} lineTooltip={(hour, tags, labelAt, colour) => model ? [occupationLineTooltip(model, hour, tags, labelAt, colour, shown, rank, collaboration)] : []} /></div>;
}
