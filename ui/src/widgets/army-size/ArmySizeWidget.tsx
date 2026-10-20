import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DrillHost from '../../charts/drill/DrillHost';
import { drillHour } from '../../charts/drill/drillState';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';
import CountryPicker from '../../components/CountryPicker';
import { armyDrillActors, armyDrillNumbers, pinArmyDrill } from './armyDrill';
import ArmyBreakdown from './ArmyBreakdown';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { countryValueAt, countryValueEntries, countryValueLines, countryValueMix, countryValuesModel, countryValueSegmentAt, rankCountryValues } from '../../charts/race/countryValues';
import { mixCountry } from '../../charts/race/options';
import type { RaceView } from '../../charts/race/options';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { countryCapitulationBadges } from '../../dashboard/countryCapitulations';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import DatedCountryChip, { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, MEMBERSHIP_PREPARING, noMembersAt, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { groupNumbers } from '../../dashboard/groupsReply';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { NumericReply } from '../../dashboard/numericReply';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { ArmyAttribution, ArmySizeReply, CountryGroupReply } from '../../generated/dashboard';
import { armyMeasure, armyMetadataRequest, armySizeRequest, armyTooltips, armyWindows,
  formatArmyValue, formatArmyRaw, formatArmyPart, measureName, mergeArmyWindows, typeName, validateArmySizeReply,
  validateArmyWindow } from './armySizeData';
import { normaliseArmyOptions, OPTIONS } from './definition';
import { armyCountrySelection } from './ArmyCountryChip';
import './armySize.css';
import { useParityRows } from '../../dashboard/parityRows';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { usePreparedArmy } from './preparedArmy';
import { overlayCountryIdentities } from '../../dashboard/countryIdentity';

const NO_WARNINGS: string[] = [];

/** The shared country race owns presentation; Army size supplies land values. */
export default function ArmySizeWidget({ widget, runtime, active, countrySlot = null, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, normaliseArmyOptions(widget.options));
  const view = (options.view === 'breakdown' ? 'mix' : options.view) as RaceView;
  const pinned = options.view === 'breakdown';
  const [drill, dispatchDrill] = useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE);
  usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const [drillOrigin, setDrillOrigin] = useState<{ tag: string; hour: number } | null>(null);
  const drillAnchor = useRef<HTMLDivElement>(null);
  const [drillWarnings, setDrillWarnings] = useState<string[]>([]);
  const countFor = options.countFor as ArmyAttribution;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const savedSelection = armyCountrySelection(widget, track);
  const invalidSavedSelection = pinned && Object.hasOwn(widget.options ?? {}, 'drillOriginHour') && !savedSelection;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const stores = useMemo(() => ({ manifest: new NumericReply<ArmySizeReply>(), groups: new NumericReply<CountryGroupReply>(), groupWarnings: new NumericReply<string[]>(),
    numbers: new NumericReply<ArmySizeReply>(), labels: new NumericReply<Pick<ArmySizeReply, 'types' | 'warnings'>>() }), [runtime.client]);
  const sourceScope = JSON.stringify([track?.source.id, track?.branch.id]);
  const incomingGroups = useCountryGroups(runtime, track, active && (grouped || !pinned && view !== 'race'));
  const groupData = stores.groups.update(sourceScope, incomingGroups.data, groupNumbers);
  const groupWarnings = stores.groupWarnings.update(sourceScope, incomingGroups.data?.warnings, reply => reply);
  const groupReply = { ...incomingGroups, data: groupData };
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupReply.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groupReply.data, lastHour]);
  const metadataRequest = armyMetadataRequest(track);
  const incomingMetadata = useProduct<ArmySizeReply>(runtime.client, metadataRequest, active, validateArmySizeReply, track?.source);
  const manifest = stores.manifest.update(sourceScope, incomingMetadata.data, reply => reply);
  const metadata = { ...incomingMetadata, data: manifest };
  const measure = !pinned && view === 'mix' ? 'battalions' : armyMeasure(options.measure, metadata.data?.measures);
  const byType = measure === 'battalions' && (pinned || view === 'mix' || (view === 'race' && options.byType === true));
  const content = useContentRevision(runtime.client);
  const identity = track && metadata.data && (resolved.top || resolved.tags.length)
    ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content,
      resolved.tags, resolved.top, measure, countFor, byType]) : null;
  const membershipKey = useMemo(() => JSON.stringify([...(resolved.membership ?? [])]), [resolved.membership]);
  const viewportKey = JSON.stringify([sourceScope, resolved.tags, resolved.top, measure, countFor, byType, view, extent, options.playback,
    options.windowDays, options.afterPlayhead]);
  const [viewport, setViewport] = useState<{ key: string; range: Span } | null>(null);
  const onViewChange = useCallback((x: readonly [number, number]) => {
    const range = { firstHour: Math.floor(x[0]), lastHour: Math.ceil(x[1]) };
    setViewport(before => before?.key === viewportKey && before.range.firstHour === range.firstHour
      && before.range.lastHour === range.lastHour ? before : { key: viewportKey, range });
  }, [viewportKey]);
  // Bootstrap the moment only; the mounted chart reports its actual initial view.
  const visible = view !== 'race' && viewport?.key === viewportKey ? viewport.range
    : { firstHour: moment, lastHour: moment };
  const until = shownUntil(playhead, options.playback as PlaybackMode, options.afterPlayhead === true);
  const visibleRange = { firstHour: Math.max(extent[0], visible.firstHour),
    lastHour: Math.min(extent[1], visible.lastHour, until ?? extent[1]) };
  const prepared = usePreparedArmy(runtime.client, track, track && metadata.data ? armySizeRequest(track.branch.id, resolved, measure, countFor, false) : null,
    metadata.data, active && view === 'lines');
  const [reachedHour, setReachedHour] = useState<number | null>(null);
  const dense = view === 'lines' && prepared.dense;
  const reached = [...prepared.summaries.values()].flatMap(summary => {
    for (let index = summary.buckets.length - 1; index >= 0; index--) if (summary.buckets[index]) return [summary.buckets[index]!.lastHour];
    return [];
  });
  const required = dense ? [...new Set([moment, ...(reachedHour === null ? [] : [reachedHour]), ...reached].flatMap(hour =>
    armyWindows(metadata.data, { firstHour: hour, lastHour: hour }, hour, false)))].sort((a, b) => a - b)
    : armyWindows(metadata.data, visibleRange, moment, false);
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const wantedWindows = parity?.requested ? metadata.data?.windows.map((_, index) => index) ?? []
    : dense ? required : armyWindows(metadata.data, visibleRange, moment, following, view !== 'race');
  const history = useProductWindows<ArmySizeReply>(runtime.client, identity, window => ({
    sourceId: track!.source.id, revision: track!.source.revision, product: 'army-size',
    params: armySizeRequest(track!.branch.id, resolved, measure, countFor, byType, window),
  }), wantedWindows, active && metadata.data?.measures.includes(measure) === true,
  value => validateArmyWindow(value, metadata.data!), track?.source);
  const incomingData = useMemo(() => {
    if (!metadata.data) return undefined;
    const reply = mergeArmyWindows(metadata.data, history.data, wantedWindows);
    const identities = overlayCountryIdentities(prepared.identities ?? [], reply.series.flatMap(series => series.identities === undefined ? [] : [{ tag: series.tag, identities: series.identities }]));
    const series = new Map(reply.series.map(series => [series.tag, series]));
    for (const identity of identities) {
      const before = series.get(identity.tag);
      series.set(identity.tag, { ...before, tag: identity.tag, runs: before?.runs ?? [],
        segments: before?.segments ?? identity.identities.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour })),
        identities: identity.identities, membership: before?.membership ?? null });
    }
    return { ...reply, series: [...series.values()] };
  }, [metadata.data, history.data, wantedWindows.join(','), prepared.identities]); // eslint-disable-line react-hooks/exhaustive-deps
  const pending = required.some(index => !history.data.has(index));
  const parityRows = (incomingData?.series ?? []).flatMap(series => series.runs.map(run => ({ tag: series.tag, run, identities: series.identities, membership: series.membership })));
  useParityRows(drillAnchor, () => ({ kind: 'windowed', label: 'Army size exact values',
    complete: !!parity?.requested && !!metadata.data && wantedWindows.every(index => history.data.has(index)), total: parityRows.length,
    rows: parityRows.map((row, index) => ({ index, key: `${row.tag}:${row.run[0]}:${row.run[2] ?? row.run[0]}`, ...row })), warnings: incomingData?.warnings }),
    { prepare: parity?.prepare, ready: () => !!parity?.requested && !!metadata.data && wantedWindows.every(index => history.data.has(index)), error: () => required.map(index => history.errors.get(index)).find(Boolean) ?? null });
  useParityRows(drillAnchor.current?.parentElement ?? null, () => {
    const rows = (incomingData?.series ?? []).flatMap(series => (series.mix ?? []).map(run => ({ tag: series.tag, ...run })));
    return { kind: 'windowed', label: 'Army size battalion composition', complete: !!parity?.requested && wantedWindows.every(index => history.data.has(index)),
      total: rows.length, rows: rows.map((row, index) => ({ index, key: `${row.tag}:${row.firstHour}:${row.lastHour}`, ...row })), types: incomingData?.types };
  }, { prepare: parity?.prepare, ready: () => !!parity?.requested && !!metadata.data && wantedWindows.every(index => history.data.has(index)) });
  const error = prepared.error ?? required.map(index => history.errors.get(index)).find(Boolean);
  const numericScope = JSON.stringify([sourceScope, resolved.tags, resolved.top, membershipKey, measure, countFor, byType, extent]);
  const previousNumbers = stores.numbers.update(numericScope, undefined, armyDrillNumbers);
  const numbers = stores.numbers.update(numericScope, pending && previousNumbers ? undefined : incomingData, armyDrillNumbers);
  const incomingLabels = useMemo(() => incomingData ? { types: incomingData.types, warnings: incomingData.warnings } : undefined, [incomingData]);
  const labels = stores.labels.update(numericScope, pending ? undefined : incomingLabels, reply => reply);
  const shown = numbers ?? incomingData;
  const data = useMemo(() => shown ? { ...shown, ...labels } : undefined, [shown, labels]);
  const model = useMemo(() => data ? countryValuesModel(data) : null, [data]);
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  const membershipOf = (tag: string): readonly Span[] | null => model?.series.get(tag)?.membership ?? resolved.membership?.get(tag) ?? null;
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  const ranked = useMemo(() => model ? rankCountryValues(model, widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? model.tags : rankedCandidates(resolved, moment, model.tags), moment, slots) : [],
    [model, resolved, moment, slots]);
  const lineTags = useMemo(() => dense ? [...new Set([...(model?.tags ?? []), ...prepared.summaries.keys()])]
    : resolved.top != null || widget.countries.kind !== 'group' ? model?.tags ?? [] : resolved.tags, [resolved, model, dense, prepared.summaries]);
  const chosenMix = mixCountry(widget.countries, fallback, ranked);
  const mixTag = chosenMix && model?.series.has(chosenMix) ? chosenMix : ranked[0]?.tag ?? chosenMix;
  const resetKey = JSON.stringify([track?.source.id, track?.branch.id,
    measure, countFor, widget.countries, widget.time, options.view, widget.options?.drillOriginHour]);
  const pickScope = JSON.stringify([resetKey, membershipKey, byType, view, extent]);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope: pickScope, model: numbers }),
    [runtime.client, pickScope, numbers]);
  const choices = drill.choices?.key === resetKey ? drill.choices : null;
  useEffect(() => { dispatchDrill({ type: 'close' }); setDrillOrigin(null); }, [resetKey]);
  const drillTag = drill.breakdown;
  const clockHour = track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : null;
  const drillAt = Math.floor((drill.breakdown === null && choices === null ? moment : drillHour(drill, clockHour)) ?? moment);
  const drillPlayhead = drill.breakdown !== null && drill.follow ? clockHour : playhead;
  const originHour = drillOrigin?.tag === drillTag ? drillOrigin.hour : drillAt;
  const follow = () => { if (active && track) dispatchDrill({ type: 'follow' }); };
  useEffect(() => {
    if (!pinned || invalidSavedSelection || !window || drill.breakdown !== null || drill.choices !== null || !model) return;
    const fixed = savedSelection?.tag;
    const actors = fixed ? [fixed] : armyDrillActors(model,
      widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? model.tags
        : rankedCandidates(resolved, moment, model.tags), moment, resolved.top ?? undefined);
    if (actors.length === 1) {
      setDrillOrigin({ tag: actors[0], hour: savedSelection?.hour ?? moment });
      dispatchDrill({ type: 'open', breakdown: actors[0], hour: moment, playing: false,
        playhead: widget.time.mode === 'follow' ? playhead : null });
    } else if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour: moment });
  }, [pinned, invalidSavedSelection, !!window, drill, model, resolved, moment, playhead, savedSelection?.tag, savedSelection?.hour, widget.countries, widget.time.mode, resetKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const drillOpen = choices !== null || drill.breakdown !== null;
  const open = (hour: number, tag?: string, candidates?: readonly string[]) => {
    if (!active) return;
    const actors = partsRaceDrillActors(tag, candidates, () => model ? mixCountry(widget.countries, fallback,
      rankCountryValues(model, rankedCandidates(resolved, hour, model.tags), hour, slots)) : null);
    if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour });
    else if (actors.length) {
      setDrillOrigin({ tag: actors[0], hour: Math.floor(hour) });
      dispatchDrill({ type: 'open', breakdown: actors[0], hour, playing: runtime.clock.getSnapshot().playing,
        playhead: widget.time.mode === 'follow' ? playhead : null });
    }
    else if (candidates !== undefined) dispatchDrill({ type: 'close' });
  };
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
  useEffect(() => {
    const warnings = [...(data?.warnings ?? metadata.data?.warnings ?? []), ...(groupWarnings ?? []),
      ...(!grouped && !pinned && view !== 'race' && incomingGroups.error ? [`Capitulation markers are unavailable: ${incomingGroups.error}`] : []), ...drillWarnings];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, metadata.data, groupWarnings, incomingGroups.error, grouped, pinned, view, drillWarnings, onDiagnostics]);

  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = countryValueLines(model, tag, cutTo(tag), range);
    return runs.length || prepared.summaries.has(tag) ? [{ tag, runs, endLabel: (hour: number, name: string) => {
      const at = countryValueAt(model, tag, hour);
      return `${name} ${at?.rawTotal !== undefined ? formatArmyRaw(at.rawTotal, measure) : at?.total == null ? '—' : formatArmyValue(at.total)}`;
    } }] : [];
  }), [model, view, lineTags, resolved.membership, range, measure, prepared.summaries]); // eslint-disable-line react-hooks/exhaustive-deps
  const badges = useMemo(() => {
    if (!model || view === 'race') return undefined;
    const tags = view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
    const countries = tags.flatMap(tag => {
      const series = model.series.get(tag);
      return series ? [{ ...series, membership: membershipOf(tag) }] : [];
    });
    return [...countryCapitulationBadges(groupReply.data, countries, extent).values()].flat();
  }, [model, view, lineTags, mixTag, groupReply.data, resolved.membership, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const segmentAt = useCallback((tag: string, hour: number) => countryValueSegmentAt(model, tag, hour), [model]);
  const parts = useMemo(() => (data?.types ?? []).map(type => ({ id: type.key, label: typeName(type) })), [data]);
  const entries = useMemo(() => model ? countryValueEntries(model, ranked, byType, formatArmyValue).map((entry, index) => ({ ...entry, label: ranked[index].at.rawTotal === undefined ? entry.label : formatArmyRaw(ranked[index].at.rawTotal!, measure) })) : [], [model, ranked, byType, measure]);
  const mixCut = mixTag ? cutTo(mixTag) : null;
  const mixMembership = mixTag ? membershipOf(mixTag) : null;
  const bands = useMemo(() => model && view === 'mix' && mixTag ? countryValueMix(model, mixTag, mixCut, range) : null,
    [model, view, mixTag, mixCut, range]);
  const mix = useMemo(() => {
    if (!model || !mixTag || !bands) return null;
    const at = isMember(mixMembership ?? undefined, moment) ? countryValueAt(model, mixTag, moment) : null;
    return { tag: mixTag, compactStack: true, bands, parts: parts.map((part, index) => ({ ...part,
      value: at?.values[index] == null ? '—' : formatArmyPart(at, index) })) };
  }, [model, mixTag, bands, mixMembership, moment, parts]);

  const none = noMembersAt(widget.countries, resolved, groupReply.data, moment, lastHour);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its army sizes." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (invalidSavedSelection) return <WidgetState kind="unavailable" message="Saved country unavailable." />;
    if (metadata.error) return <WidgetState kind="error" detail={metadata.error} onRetry={metadata.retry} />;
    if (error) return <WidgetState kind="error" detail={error} onRetry={history.retry} />;
    if (grouped && groupReply.error) return <WidgetState kind="error" detail={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="shield" message={resolved.reason} />;
    if (metadata.data && !metadata.data.measures.length) return <WidgetState kind="unavailable" message="Not in this recording" />;
    if (metadata.data && !metadata.data.measures.includes(measure)) {
      return <WidgetState kind="unavailable" message="Not in this recording" />;
    }
    // A group with no member found yet asks for nothing, so no reply will come.
    if (none) return <NoMembersState reason={none.reason} icon="shield" />;
    if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label="Loading army sizes…" />;
    if (pending && view === 'race' && !numbers?.available) {
      return <WidgetState kind="loading" label="Loading army data…" />;
    }
    if (!metadata.data?.available) return <WidgetState kind="unavailable" message="Not in this recording" />;
    // A group's members at hours still being prepared are not known yet: no series is not proof of none.
    if (!model.tags.length && view === 'race') return resolved.known(track.branch.firstHour, track.branch.lastHour)
      ? <WidgetState kind="unavailable" message="No known army sizes for these countries." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
    return undefined;
  })();

  const title = measureName(measure);
  // Unpinned (a catalogue card), the header names the country the body opened or was given, once it has one.
  const hasOrigin = Object.hasOwn(widget.options ?? {}, 'drillOriginHour');
  const unchosen = !hasOrigin && drillTag === null;
  const neutralHeader = pinned && countrySlot && (state !== undefined || choices !== null || unchosen)
    ? createPortal(unchosen ? <ChooseCountryChip /> : <DatedCountryChip label={null} hour={window ? drillAt : null}
      reason={!savedSelection ? 'Saved country unavailable.'
        : 'Country unavailable at this time.'} />, countrySlot) : null;
  const breakdown = <ArmyBreakdown runtime={runtime} track={track} active={active} options={options} extent={extent}
    countrySlot={pinned && state === undefined && choices === null && !unchosen ? countrySlot : null}
    headerSelection={hasOrigin || drillTag === null ? savedSelection : { tag: drillTag, hour: originHour }}
    playhead={drillPlayhead} tag={drillTag} hour={drillAt} measure={measure} countFor={countFor} manifest={metadata.data}
    originHour={originHour} following={drill.follow} pinned={pinned} onWarnings={setDrillWarnings}
    onPick={hour => active && dispatchDrill({ type: 'pick', hour })} onFollow={follow} />;
  const choice = choices && track ? <div className="parts-race-details">
    <p className="drill-empty">Choose a country for {dayLabel(drillAt)}.</p>
    <CountryPicker countries={track.source.metadata.countries} allowedTags={choices.tags} selectedTag={null} playerCountryId={null}
      source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: drillAt }}
      onChange={tag => {
        if (!active || tag === null || !choices.tags.includes(tag)) return;
        setDrillOrigin({ tag, hour: drillAt });
        dispatchDrill({ type: 'choose', key: choices.key, tag, hour: drillAt });
      }} />
  </div> : breakdown;
  if (pinned) return <>{neutralHeader}<div ref={drillAnchor} className="army-size-drill parts-race-drill">{state ?? choice}</div></>;
  const main = <CountryRaceView runtime={runtime} track={track} state={state} active={active && (!drillOpen || options.drill === 'popout')} onPick={open} pickOwner={pickOwner} options={options} view={view} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={entries} lines={lines} badges={badges} parts={parts} mix={mix} segmentAt={segmentAt} countryTags={shownTags}
    label="Army size" empty={resolved.known(moment) ? `No known army sizes on ${dayLabel(moment)}.` : MEMBERSHIP_PREPARING} formatValue={formatArmyValue}
    onViewChange={view === 'race' ? undefined : onViewChange}
    lineSummaries={prepared.summaries.size ? prepared.summaries : undefined} onSummaryView={view === 'lines' ? prepared.onSummaryView : undefined}
    exactLineTags={dense ? prepared.exactLineTags : undefined} onReachHour={dense ? setReachedHour : undefined}
    navigation={dense ? prepared : undefined}
    beforeChart={pending || !incomingMetadata.data || grouped && !incomingGroups.data
      ? <WidgetState kind="loading" label="Loading army data…" />
      : view === 'race' ? undefined : view === 'mix' && model && !parts.length && mixTag && countryValueAt(model, mixTag, moment)?.total === 0
        ? <WidgetState kind="empty" icon="shield" message={`0 battalions recorded on ${dayLabel(moment)}.`} />
        : !lines.length && view === 'lines' ? resolved.known(extent[0], extent[1])
          ? <WidgetState kind="empty" icon="shield" message="No known army sizes in this view." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />
          : view === 'mix' && !mixTag && !resolved.known(moment) ? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />
          : view === 'mix' && !bands?.some(runs => runs.length)
            ? <WidgetState kind="empty" icon="shield" message="No battalion mix recorded in this view." /> : undefined}
    labels={{ race: `Army size, ${title}, ${dayLabel(moment)}`, lines: `Army size over time, ${title}`,
      mix: name => `Army size of ${name} by unit group, Battalions`, actions: 'Army size actions' }}
    raceLegendParts={byType ? parts : []} partsLabel="Battalions by unit group"

    resetKey={`${track?.source.id}:${track?.branch.id}:${measure}:${countFor}:${byType}`}
    presentation={{ exact: true, held: false, lineColourHours: 'labels', windowedLegend: true,
      tooltipKeep: parts.length > 1 ? 1 : 0, linePadding: CHART_LAYOUT.countryEndLabels, strongMixName: true, mixKeyHours: 'starts', raceEmptyState: false, resetTimeView: true }}
    tooltip={(tag, hour, name, colours) => {
      if (!model) return null;
      const at = isMember(membershipOf(tag) ?? undefined, hour) ? countryValueAt(model, tag, hour) : null;
      return at ? armyTooltips(model, name, at, hour, measure, colours) : null;
    }} lineTooltip={(hour, tags, labelAt, country) => [{ title: dayLabel(hour), subtitle: title,
      rows: model ? rankCountryValues(model, tags, hour, tags.length).map(({ tag, at }) => {
        const label = labelAt(hour)(tag);
        return { label: label.name, value: at.rawTotal === undefined ? formatArmyValue(at.total) : formatArmyRaw(at.rawTotal, measure),
          swatch: country(tag), short: true, countryFlag: label };
      }) : [{ label: 'Not recorded', tone: 'muted' as const }],
    }]} />;
  return <div className="army-size-drill parts-race" ref={drillAnchor}>
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={drillAnchor}
      title="Army breakdown" day={dayLabel(drillAt)} following={drill.follow} canFollow={choices === null && track !== undefined}
      returnFocus=".race-chart canvas, .time-chart canvas" onClose={() => dispatchDrill({ type: 'close' })} onFollow={follow}
      onPin={track && drillTag ? () => runtime.addWidgetBelow(widget.id, pinArmyDrill(
        widget, track, drillTag, drillAt, originHour, measure, drill.follow)) : undefined}
      main={main}>
      {choice}
    </DrillHost>
  </div>;
}
