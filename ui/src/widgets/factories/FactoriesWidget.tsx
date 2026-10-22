import { useCallback, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { dayLabel } from '../../charts/draw';
import CountryRaceView from '../../charts/race/CountryRaceView';
import CountryPartsDrill from '../../charts/race/CountryPartsDrill';
import type { CountryRacePart } from '../../charts/race/countryRace';
import type { RaceView } from '../../charts/race/options';
import { mixCountry } from '../../charts/race/options';
import { linesTooltip } from '../../charts/race/lines';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, MEMBERSHIP_PREPARING, noMembersAt, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { usePreparedRace } from '../../charts/race/PartsRaceView';
import { mergeFactories, factoriesGeometry } from './preparedFactories';
import { useParityRows } from '../../dashboard/parityRows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { NumericReply } from '../../dashboard/numericReply';
import { groupNumbers } from '../../dashboard/groupsReply';
import type { CountryFactoriesReply, CountryGroupReply } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition';
import { chosenGroups, factoriesAt, factoriesRequest, factoryModel, factorySegmentAt, factoryTooltips, formatCount, groupName, lineRuns, mixBands,
  raceEntries, rankFactories, validateFactoriesReply } from './factoriesData';
import type { CountMode } from './factoriesData';
import { factoryDrillData, factoryDrillNumbers } from './factoryDrill';

/** Factory counts retain their formulas and wording; the shared race owns their views. */
export default function FactoriesWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics } = props;
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const view = (options.view as RaceView) ?? 'race';
  const mode = (options.count as CountMode) ?? 'free';
  const chosen = useMemo(() => contentSet(options.types), [options.types]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groupReply = useCountryGroups(runtime, track, active && grouped);
  const retainedGroups = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const groupScope = JSON.stringify([track?.source.id, track?.branch.id, widget.countries]);
  const numericGroups = retainedGroups.update(groupScope, groupReply.data, groupNumbers);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], numericGroups, lastHour, definition.countries.max),
    [widget.countries, fallback, numericGroups, lastHour]);
  const params = track ? factoriesRequest(track.branch.id, resolved, mode, chosen) : null;
  const request = track && params && (params.top || params.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-factories', params: runtime.client.preparedQuery ? { ...params, mode, groups: chosen.filter(key => ['military', 'naval', 'civilian'].includes(key)) } : params } : null;
  const prepared = usePreparedRace(runtime.client, track, request, moment, active, validateFactoriesReply, mergeFactories, factoriesGeometry, view !== 'race', resolved.membership ?? undefined);
  const { data, retry } = prepared;
  const error = useHeldFailure(prepared.error, data !== undefined, retry);
  const numeric = useMemo(() => new NumericReply<CountryFactoriesReply>(), [runtime.client]);
  const pickScope = JSON.stringify([track?.source.id, track?.branch.id, view, widget.countries, params]);
  const pickNumbers = numeric.update(pickScope, data, factoryDrillNumbers);
  const shownReply = data ?? pickNumbers;
  const model = useMemo(() => shownReply ? factoryModel(shownReply) : null, [shownReply]);
  const pickGroups = grouped ? numericGroups : undefined;
  const pickOwner = useMemo(() => ({ scope: pickScope, numbers: pickNumbers, membership: pickGroups }), [pickScope, pickNumbers, pickGroups]);
  const groups = useMemo(() => model ? chosenGroups(model, chosen) : [], [model, chosen.join(' ')]); // eslint-disable-line react-hooks/exhaustive-deps
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  const membershipOf = (tag: string): readonly Span[] | null => model?.series.get(tag)?.membership ?? resolved.membership?.get(tag) ?? null;
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  const ranked = useMemo(() => model ? rankFactories(model, rankedCandidates(resolved, moment, model.tags), moment, groups, mode, slots) : [],
    [model, resolved, moment, groups, mode, slots]);
  const lineTags = useMemo(() => resolved.top != null ? model?.tags ?? [] : resolved.tags, [resolved, model]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
  const warnings = useMemo(() => [...(data?.warnings ?? []), ...(groupReply.data?.warnings ?? [])], [data, groupReply.data]);
  const drillParams = useCallback((tag: string) => ({ ...factoriesRequest(track!.branch.id, { tags: [tag] }, mode, chosen),
    ...(runtime.client.preparedQuery ? { mode, groups: chosen.filter(key => ['military', 'naval', 'civilian'].includes(key)) } : {}) }), [track, mode, chosen, runtime.client]);
  const decodeDrill = useCallback((reply: CountryFactoriesReply) => {
    const native = reply === data && model ? model : factoryModel(reply);
    return factoryDrillData(native, chosenGroups(native, chosen), mode);
  }, [data, model, chosen, mode]);
  const pickDrillTag = (hour: number) => model ? mixCountry(widget.countries, fallback,
    rankFactories(model, rankedCandidates(resolved, hour, model.tags), hour, groups, mode, slots)) : null;

  const split = options.split === true;
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = lineRuns(model, tag, groups, mode, cutTo(tag), range);
    return runs.length || prepared.summaries.has(tag) ? [{ tag, runs }] : [];
  }), [model, view, lineTags, groups, mode, resolved.membership, range, prepared.summaries]); // eslint-disable-line react-hooks/exhaustive-deps
  const segmentAt = useCallback((tag: string, hour: number) => factorySegmentAt(model, tag, hour), [model]);
  const parts = useMemo<CountryRacePart[]>(() => (model?.reply.groups ?? []).map(group => ({ id: group.key, label: groupName(group) })), [model]);
  const chosenParts = useMemo(() => parts.filter(part => groups.includes(part.id as typeof groups[number])), [parts, groups]);
  const entries = useMemo(() => raceEntries(ranked, { mode, split }), [ranked, mode, split]);
  const mixCut = mixTag ? cutTo(mixTag) : null;
  const mixMembership = mixTag ? membershipOf(mixTag) : null;
  const geometry = useMemo(() => prepared.geometry ? factoryModel(prepared.geometry) : model, [prepared.geometry, model]);
  const bands = useMemo(() => geometry && view === 'mix' && mixTag ? mixBands(geometry, mixTag, groups, mode, mixCut, range) : null,
    [geometry, view, mixTag, groups, mode, mixCut, range]);
  const mix = useMemo(() => {
    if (!model || !mixTag || !bands) return null;
    const at = isMember(mixMembership ?? undefined, moment) ? factoriesAt(model, mixTag, moment, groups, mode) : null;
    return { tag: mixTag, bands, parts: chosenParts.map(part => {
      const count = at?.groups.find(group => group.key === part.id)?.count;
      return { ...part, value: count == null ? '—' : formatCount(count) };
    }) };
  }, [model, mixTag, bands, groups, mode, mixMembership, moment, chosenParts]);

  const none = noMembersAt(widget.countries, resolved, numericGroups, moment, lastHour);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="factory" message="Load a campaign to see its factories." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
    if (grouped && groupReply.error && !groupReply.data) return <WidgetState kind="error" detail={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="factory" message={resolved.reason} />;
    // A group with no member found yet asks for nothing, so no reply will come.
    if (none) return <NoMembersState reason={none.reason} icon="factory" />;
    if (!shownReply || !model || (grouped && !numericGroups)) return <WidgetState kind="loading" label="Loading factories…" />;
    // A group's members at hours still being prepared are not known yet: no series is not proof of none.
    if (!model.tags.length) return resolved.known(track.branch.firstHour, track.branch.lastHour)
      ? <WidgetState kind="unavailable" message="No factories were recorded for these countries." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
    return undefined;
  })();

  const parityAnchor = useRef<HTMLDivElement>(null);
  useParityRows(parityAnchor, () => {
    const page = prepared.parity?.data;
    const rows = page?.series.flatMap(series => series.runs.map(run => ({ tag: series.tag, run, reads: series.reads,
      identities: series.identities, membership: series.membership }))) ?? [];
    return { kind: 'chart', label: 'Factories exact values', complete: !!prepared.parity?.complete, total: rows.length, rows,
      fields: page?.fields, groups: page?.groups, warnings: page?.warnings };
  }, { prepare: prepared.parity?.prepare, ready: () => !!prepared.parity?.complete, error: () => prepared.error ?? null });
  const modeLabel = OPTIONS.find(field => field.key === 'count')!;
  const countName = modeLabel.kind === 'choice' ? modeLabel.choices.find(choice => choice.value === mode)?.label ?? mode : mode;
  return <div ref={parityAnchor} className="parts-race">{grouped && groupReply.error && groupReply.data ? <WidgetState kind="error" detail={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} /> : null}<CountryPartsDrill {...props} widget={widget} runtime={runtime} active={active} onDiagnostics={onDiagnostics} track={track}
    options={options} extent={extent} playhead={playhead} moment={moment} product="country-factories" params={drillParams}
    validate={validateFactoriesReply} decode={decodeDrill} numbers={factoryDrillNumbers} defaultTag={mixTag} pickTag={pickDrillTag} title="Factory types" measure={countName}
    warnings={warnings} state={state} resetKey={JSON.stringify([track?.source.id, track?.branch.id, mode, chosen, widget.countries])}
    main={(onPick, visible) => <CountryRaceView runtime={runtime} track={track} state={state} active={active && visible} options={options} view={view} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={entries} lines={lines} parts={parts} mix={mix && {...mix,approximate:prepared.summaries.get(mix.tag)?.approximate}} segmentAt={segmentAt} countryTags={shownTags}
    label="Factories" empty={resolved.known(moment) ? `No factories recorded on ${dayLabel(moment)}.` : MEMBERSHIP_PREPARING} formatValue={formatCount}
    labels={{ race: `Factories, ${countName}, ${dayLabel(moment)}`, lines: `Factories over time, ${countName}`,
      mix: name => `Factories of ${name} by type, ${countName}`, actions: 'Factory actions' }}
    raceLegendParts={split && groups.length > 1 ? chosenParts : []} partsLabel="Factory types"
    presentation={{ exact: false, held: false, lineColourHours: 'labels',
      tooltipKeep: groups.length > 1 ? 1 : 0, linePadding: CHART_LAYOUT.countryEndLabels, strongMixName: true,
      mixKeyHours: 'starts', raceEmptyState: false, resetTimeView: true }}
    lineSummaries={prepared.summaries} onSummaryView={prepared.onSummaryView}
    onReachHour={prepared.onReachHour} navigation={prepared.navigation} exactLineTags={prepared.exactLineTags}
    pickOwner={pickOwner} lineEmpty={{ icon: 'factory', message: resolved.known(extent[0], extent[1]) ? 'No countries to draw. Choose countries in the widget settings.' : MEMBERSHIP_PREPARING }}
    mixEmpty={{ icon: 'factory', message: !resolved.known(moment) ? MEMBERSHIP_PREPARING : widget.countries.kind === 'group'
      ? `No country of this group was recorded on ${dayLabel(moment)}.` : 'Choose a country in the widget settings.',
      unavailable: name => `No factories were recorded for ${name}.` }}
    tooltip={(tag, hour, name, colours) => {
      if (!model) return null;
      const at = isMember(membershipOf(tag) ?? undefined, hour) ? factoriesAt(model, tag, hour, groups, mode) : null;
      return at ? factoryTooltips(model, name, at, dayLabel(hour), mode, colours) : null;
    }} lineTooltip={(hour, _tags, labelAt, country) => [linesTooltip(hour, lines, labelAt, country, dayLabel(hour), countName, formatCount)]} onPick={onPick} />} /></div>;
}
