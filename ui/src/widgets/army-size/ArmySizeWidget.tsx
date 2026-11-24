import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';
import CountryPicker from '../../components/CountryPicker';
import { armyDrillActors, armyDrillNumbers, pinArmyDrill } from './armyDrill';
import ArmyBreakdown from './ArmyBreakdown';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import LiveText, { liveTexts } from '../../charts/LiveText';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { countryValueAt, countryValueLines, countryValuesModel, countryValueSegmentAt, rankCountryValues } from '../../charts/race/countryValues';
import { mixCountry } from '../../charts/race/options';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { countryCapitulationBadges } from '../../dashboard/countryCapitulations';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { countryTagAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import DatedCountryChip, { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { NumericReply } from '../../dashboard/numericReply';
import { useProduct } from '../../dashboard/products';
import { useFrameWindows } from './frameWindows.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { ArmyAttribution, CountryGroupReply } from '../../generated/dashboard';
import type { ArmySizeReply } from './armySizeData';
import { armyAbsent, armyMeasure, armyMetadataRequest, armySizeRequest, armyTooltips, armyWindows,
  formatArmyValue, formatArmyRaw, measureName, mergeArmyWindows, offersMeasure, typeName, validateArmySizeReply,
  validateArmyWindow } from './armySizeData.ts';
import { normaliseArmyOptions, OPTIONS } from './definition';
import { armyCountrySelection } from './ArmyCountryChip';
import './armySize.css';
import { usePreparedArmy } from './preparedArmy';
import { overlayCountryIdentities } from '../../dashboard/countryIdentity';
import { useHeadHour } from './headHour.ts';
import { useChangedState } from '../../dashboard/changedState.ts';
import { useDrill } from '../../charts/drill/useDrill';
import type { PartsRaceDrillAction, PartsRaceDrillState } from '../../charts/race/partsRaceDrill';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget, DetailWindowHost } from '../../families/host/index.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { armySizeMixDeclaration } from './armySizeMixProjection.ts';
import { armySizeRaceDeclaration } from './armySizeRaceProjection.ts';

const sameWarnings = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((warning, at) => warning === b[at]);

const NO_WARNINGS: string[] = [];

const HOLDERS = { holders: true };

export default function ArmySizeWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, normaliseArmyOptions(props.widget.options));
  if (options.view === 'race' || options.view === 'mix') {
    return <ArmySizeBarsWidget {...props} barsView={options.view} />;
  }
  return <LegacyArmySizeWidget {...props} />;
}

function ArmySizeBarsWidget(props: WidgetProps & { readonly barsView: 'race' | 'mix' }) {
  const { widget, runtime, active, barsView } = props;
  const options = resolveOptions(OPTIONS, normaliseArmyOptions(widget.options));
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const [detailWarnings, setDetailWarnings] = useChangedState<string[]>([], sameWarnings);
  const gate = useGate(runtime, widget, SPEC, active);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const countFor = options.countFor as ArmyAttribution;
  const metadataRequest = armyMetadataRequest(track);
  const metadata = useProduct<ArmySizeReply>(runtime.client, metadataRequest, active && detail !== null, validateArmySizeReply, track?.source);
  const measure = barsView === 'mix' ? 'battalions' : armyMeasure(options.measure, metadata.data?.measures);
  const extent: [number, number] = track ? [track.branch.firstHour, track.branch.lastHour] : [0, 1];

  const onActivate = useCallback((action: InstalledAction) => {
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const meta = action.meta as { readonly tag?: string; readonly key?: string } | null | undefined;
    setDetail({ tag: meta?.tag ?? action.subject, key: meta?.key ?? action.subject, hour: gated });
  }, [gate]);

  return (
    <div ref={anchorRef} className="fh-frame">
      {barsView === 'mix' ? (
        <DeclaredFamilyWidget
          {...props}
          adapter={barsHostAdapter}
          view="mix"
          declaration={armySizeMixDeclaration}
          selectedKey={detail?.key ?? null}
          extraWarnings={detailWarnings}
          onActivate={onActivate}
        />
      ) : (
        <DeclaredFamilyWidget
          {...props}
          adapter={barsHostAdapter}
          view="race"
          declaration={armySizeRaceDeclaration}
          selectedKey={detail?.key ?? null}
          extraWarnings={detailWarnings}
          onActivate={onActivate}
        />
      )}
      <DetailWindowHost
        open={detail !== null}
        subject={detail?.tag ?? null}
        hour={detail?.hour ?? null}
        title={detail ? `${detail.tag} army breakdown` : 'Army breakdown'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => { setDetail(null); setDetailWarnings(NO_WARNINGS); }}
      >
        {detail && track ? (
          <ArmyBreakdown
            runtime={runtime}
            gate={gate}
            track={track}
            active={active}
            options={options}
            extent={extent}
            countrySlot={null}
            headerSelection={{ tag: detail.tag, hour: detail.hour }}
            playhead={detail.hour}
            tag={detail.tag}
            hour={detail.hour}
            measure={measure}
            countFor={countFor}
            manifest={metadata.data}
            originHour={detail.hour}
            following={false}
            pinned={false}
            onWarnings={setDetailWarnings}
            onPick={() => {}}
            onFollow={() => {}}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

function LegacyArmySizeWidget({ widget, runtime, active, countrySlot = null, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, normaliseArmyOptions(widget.options));
  const view = options.view === 'breakdown' ? 'mix' : 'lines';
  const pinned = options.view === 'breakdown';
  const drillControl = useDrill<PartsRaceDrillState, PartsRaceDrillAction>(runtime, useRegion(widget), null, reducePartsRaceDrill, CLOSED_PARTS_RACE, HOLDERS);
  const gate = useGate(runtime, widget, SPEC, active);
  const drill = drillControl.state, dispatchDrill = drillControl.dispatch;
  const drillFollowing = drill.breakdown !== null && drill.follow;
  const missingKey = useRef<(hour: number) => string>(() => '');
  const woken = useMemo(() => new Set<() => void>(), []);
  const playbackHour = useHeadHour(runtime.clock, active && (widget.time.mode === 'follow' || drillFollowing), hour => missingKey.current(hour), woken);
  const [drillOrigin, setDrillOrigin] = useState<{ tag: string; hour: number } | null>(null);
  const drillAnchor = useRef<HTMLDivElement>(null);
  const [drillWarnings, setDrillWarnings] = useChangedState<string[]>([], sameWarnings);
  const countFor = options.countFor as ArmyAttribution;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const savedSelection = armyCountrySelection(widget, track);
  const invalidSavedSelection = pinned && Object.hasOwn(widget.options ?? {}, 'drillOriginHour') && !savedSelection;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => widget.time.mode === 'follow' ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [widget.time.mode, runtime.clock, axisOffset]);
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const headAt = (hour: number) => {
    const at = track && resolveWindow(widget, track, runtime.tracks, hour);
    return at && following ? at.lastHour : null;
  };
  const momentAt = (hour: number) => Math.floor(headAt(hour) ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const stores = useMemo(() => ({ manifest: new NumericReply<ArmySizeReply>(), groups: new NumericReply<CountryGroupReply>(), groupWarnings: new NumericReply<string[]>(),
    numbers: new NumericReply<ArmySizeReply>(), labels: new NumericReply<Pick<ArmySizeReply, 'types' | 'warnings'>>() }), [runtime.client]);
  const sourceScope = JSON.stringify([track?.source.id, track?.branch.id]);
  const incomingGroups = useCountryGroups(runtime, track, active && (grouped || !pinned));
  const groupData = stores.groups.update(sourceScope, incomingGroups.data, groupNumbers);
  const groupWarnings = stores.groupWarnings.update(sourceScope, incomingGroups.data?.warnings, reply => reply);
  const groupReply = { ...incomingGroups, data: groupData };
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupReply.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groupReply.data, lastHour]);
  const metadataRequest = armyMetadataRequest(track);
  const incomingMetadata = useProduct<ArmySizeReply>(runtime.client, metadataRequest, active, validateArmySizeReply, track?.source);
  const manifest = stores.manifest.update(sourceScope, incomingMetadata.data, reply => reply);
  const metadata = { ...incomingMetadata, data: manifest };
  const measure = pinned && !options.measure && metadata.data?.measures.includes('battalions') ? 'battalions'
    : armyMeasure(options.measure, metadata.data?.measures);
  const byType = measure === 'battalions' && pinned;
  const content = useContentRevision(runtime.client);
  const identity = track && metadata.data && (resolved.top || resolved.tags.length)
    ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content,
      resolved.tags, resolved.top, measure, countFor, byType]) : null;
  const membershipKey = useMemo(() => JSON.stringify([...(resolved.membership ?? [])]), [resolved.membership]);
  const viewportKey = JSON.stringify([sourceScope, resolved.tags, resolved.top, measure, countFor, byType, view, extent, options.playback,
    options.windowDays, options.afterPlayhead]);
  const viewport = useRef<{ key: string; range: Span } | null>(null);
  const windowsCheck = useRef(() => {});
  const onViewChange = useCallback((x: readonly [number, number]) => {
    viewport.current = { key: viewportKey, range: { firstHour: Math.floor(x[0]), lastHour: Math.ceil(x[1]) } };
    windowsCheck.current();
  }, [viewportKey]);
  const prepared = usePreparedArmy(runtime.client, gate, track, track && metadata.data ? armySizeRequest(track.branch.id, resolved, measure, countFor, false) : null,
    metadata.data, active && view === 'lines', widget.time.mode === 'follow');
  const [reachedHour, setReachedHour] = useState<number | null>(null);
  const dense = view === 'lines' && prepared.dense;
  const windowsAt = (hour: number) => {
    const at = momentAt(hour), until = shownUntil(headAt(hour), options.playback as PlaybackMode, options.afterPlayhead === true);
    if (dense) {
      const reached = [...(prepared.lineData?.read() ?? prepared.summaries).values()].flatMap(summary => {
        const end = Math.min(summary.to, until ?? Infinity);
        for (let index = summary.buckets.length - 1; index >= 0; index--) {
          const bucket = summary.buckets[index];
          if (bucket && bucket.firstHour <= end) return [Math.min(bucket.lastHour, end)];
        }
        return [];
      });
      const required = [...new Set([at, ...(reachedHour === null ? [] : [reachedHour]), ...reached].flatMap(end =>
        armyWindows(metadata.data, { firstHour: end, lastHour: end }, end, false)))].sort((a, b) => a - b);
      return { required, wanted: [...new Set([...required, ...armyWindows(metadata.data, { firstHour: at, lastHour: at }, at, following)])].sort((a, b) => a - b) };
    }
    const seen = viewport.current?.key === viewportKey ? viewport.current.range : { firstHour: at, lastHour: at };
    const range = { firstHour: Math.max(extent[0], seen.firstHour), lastHour: Math.min(extent[1], seen.lastHour, until ?? extent[1]) };
    return { required: armyWindows(metadata.data, range, at, false), wanted: armyWindows(metadata.data, range, at, following, true) };
  };
  const { required } = windowsAt(playbackHour);
  const subscribeClock = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const history = useFrameWindows<ArmySizeReply>(runtime.client, identity, window => (productRequest({
    sourceId: track!.source.id, revision: track!.source.revision, product: 'army-size',
    params: armySizeRequest(track!.branch.id, resolved, measure, countFor, byType, window),
  })), () => windowsAt(runtime.clock.getHour()).wanted, subscribeClock, active && offersMeasure(metadata.data, measure),
  value => validateArmyWindow(value, metadata.data!), track?.source);
  windowsCheck.current = history.check;
  missingKey.current = hour => windowsAt(hour).required.filter(window => !history.holds(window)).join(',');
  const incomingData = useMemo(() => {
    if (!metadata.data) return undefined;
    const reply = mergeArmyWindows(metadata.data, history.data, [...history.data.keys()]);
    const identities = overlayCountryIdentities(prepared.identities ?? [], reply.series.flatMap(series => series.identities === undefined ? [] : [{ tag: series.tag, identities: series.identities }]));
    const series = new Map(reply.series.map(series => [series.tag, series]));
    for (const identity of identities) {
      const before = series.get(identity.tag);
      series.set(identity.tag, { ...before, tag: identity.tag, runs: before?.runs ?? [],
        segments: before?.segments ?? identity.identities.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour })),
        identities: identity.identities, membership: before?.membership ?? null });
    }
    return { ...reply, series: [...series.values()] };
  }, [metadata.data, history.data, prepared.identities]);
  const pending = required.some(index => !history.data.has(index));
  const error = prepared.error ?? required.map(index => history.errors.get(index)).find(Boolean);
  const retry = prepared.error ? prepared.retry : history.retry;
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
  const lineTags = useMemo(() => dense ? [...new Set([...(model?.tags ?? []), ...prepared.summaries.keys()])]
    : resolved.top != null || widget.countries.kind !== 'group' ? model?.tags ?? [] : resolved.tags, [resolved, model, dense, prepared.summaries]);
  const resetKey = JSON.stringify([track?.source.id, track?.branch.id,
    measure, countFor, widget.countries, widget.time, options.view, widget.options?.drillOriginHour]);
  const pickScope = JSON.stringify([resetKey, membershipKey, byType, view, extent]);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope: pickScope, model: numbers }),
    [runtime.client, pickScope, numbers]);
  const choices = drill.choices?.key === resetKey ? drill.choices : null;
  useEffect(() => { dispatchDrill({ type: 'close' }); setDrillOrigin(null); }, [resetKey]);
  const drillTag = drill.breakdown;
  const clockHour = track ? Math.floor(playbackHour) + track.axisOffset : null;
  const drillHour = choices !== null ? drill.hour : drillControl.following ? clockHour : drillControl.hour;
  const drillAt = Math.floor((drillControl.open ? drillHour : moment) ?? moment);
  const drillPlayhead = drillControl.following ? clockHour : playhead;
  const originHour = drillOrigin?.tag === drillTag ? drillOrigin.hour : drillAt;
  const pinSlots = useSlotTags(runtime.client, track, active && !!drillTag && isSlotKey(drillTag));
  const pinTag = drillTag && isSlotKey(drillTag) ? countryTagAt(pinSlots, countryFromKey(drillTag), originHour) : drillTag;
  const follow = () => { if (active && track) drillControl.follow(); };
  useEffect(() => {
    if (!pinned || invalidSavedSelection || !window || drill.breakdown !== null || drill.choices !== null || !model) return;
    const fixed = savedSelection?.tag;
    const actors = fixed ? [fixed] : armyDrillActors(model,
      widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? model.tags
        : rankedCandidates(resolved, moment, model.tags), moment, resolved.top ?? undefined);
    if (actors.length === 1) {
      setDrillOrigin({ tag: actors[0], hour: savedSelection?.hour ?? moment });
      drillControl.show(actors[0], moment);
    } else if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour: moment });
  }, [pinned, invalidSavedSelection, !!window, drill, model, resolved, moment, playhead, savedSelection?.tag, savedSelection?.hour, widget.countries, widget.time.mode, resetKey]);
  const drillOpen = choices !== null || drill.breakdown !== null;
  const open = (hour: number, tag?: string, candidates?: readonly string[]) => {
    if (!active) return;
    const actors = partsRaceDrillActors(tag, candidates, () => model ? mixCountry(widget.countries, fallback,
      rankCountryValues(model, rankedCandidates(resolved, hour, model.tags), hour, slots)) : null);
    if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour });
    else if (actors.length) {
      setDrillOrigin({ tag: actors[0], hour: Math.floor(hour) });
      drillControl.show(actors[0], hour);
    }
    else if (candidates !== undefined) dispatchDrill({ type: 'close' });
  };
  useEffect(() => {
    const warnings = [...(data?.warnings ?? metadata.data?.warnings ?? []), ...(groupWarnings ?? []),
      ...(!grouped && !pinned && incomingGroups.error ? [`Capitulation markers are unavailable: ${incomingGroups.error.message}`] : []), ...drillWarnings];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, metadata.data, groupWarnings, incomingGroups.error, grouped, pinned, drillWarnings, onDiagnostics]);

  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
  const lines = useMemo(() => !model ? [] : lineTags.flatMap(tag => {
    const runs = countryValueLines(model, tag, cutTo(tag), range);
    return runs.length || prepared.summaries.has(tag) ? [{ tag, runs, endLabel: (hour: number, name: string) => {
      const at = countryValueAt(model, tag, hour);
      return { text: name, suffix: at?.rawTotal !== undefined ? formatArmyRaw(at.rawTotal, measure) : at?.total == null ? '—' : formatArmyValue(at.total) };
    } }] : [];
  }), [model, lineTags, resolved.membership, range, measure, prepared.summaries]);
  const badges = useMemo(() => {
    if (!model) return undefined;
    const countries = lineTags.flatMap(tag => {
      const series = model.series.get(tag);
      return series ? [{ ...series, membership: membershipOf(tag) }] : [];
    });
    return [...countryCapitulationBadges(groupReply.data, countries, extent, groupReply.data?.countryIdentities).values()].flat();
  }, [model, lineTags, groupReply.data, resolved.membership, extent[0], extent[1]]);
  const segmentAt = useCallback((tag: string, hour: number) => countryValueSegmentAt(model, tag, hour), [model]);
  const parts = useMemo(() => (data?.types ?? []).map(type => ({ id: type.key, label: typeName(type) })), [data]);
  const dayLive = useMemo(() => liveTexts(runtime.clock, () => Math.floor(runtime.clock.getHour()) + axisOffset, hour => new Map([['day', dayLabel(hour)]])),
    [runtime.clock, axisOffset]);

  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its army sizes." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (invalidSavedSelection) return <WidgetState kind="unavailable" message="Saved country unavailable." />;
    if (metadata.error) return <WidgetState kind="error" failure={asFailure(metadata.error)} onRetry={metadata.retry} />;
    if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
    if (grouped && groupReply.error && !groupReply.data) return <WidgetState kind="error" failure={groupReply.error} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="shield" message={resolved.reason} />;
    if (metadata.data && !offersMeasure(metadata.data, measure)) {
      return <WidgetState kind="unavailable" message="Not in this recording" />;
    }
    if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label="Loading army sizes…" />;
    if (!metadata.data || armyAbsent(metadata.data)) return <WidgetState kind="unavailable" message="Not in this recording" />;
    return undefined;
  })();

  const title = measureName(measure);
  const hasOrigin = Object.hasOwn(widget.options ?? {}, 'drillOriginHour');
  const unchosen = !hasOrigin && drillTag === null;
  const neutralHeader = pinned && countrySlot && (state !== undefined || choices !== null || unchosen)
    ? createPortal(unchosen ? <ChooseCountryChip /> : <DatedCountryChip label={null} hour={window ? drillAt : null}
      reason={!savedSelection ? 'Saved country unavailable.'
        : 'Country unavailable at this time.'} />, countrySlot) : null;
  const breakdown = <ArmyBreakdown runtime={runtime} gate={pinned ? gate : drillControl.gate} track={track} active={active} options={options} extent={extent}
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
  const groupNotice = grouped && groupReply.error && groupReply.data ? <WidgetState kind="error" failure={groupReply.error} onRetry={groupReply.retry} /> : null;
  if (pinned) return <>{neutralHeader}{groupNotice}<div ref={drillAnchor} className="army-size-drill parts-race-drill">{state ?? choice}</div></>;
  const main = <CountryRaceView runtime={runtime} track={track} state={state} active={active && (!drillOpen || options.drill === 'popout')} onPick={open} pickOwner={pickOwner} options={options} view="lines" extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={[]} lines={lines} badges={badges} parts={parts} mix={null} segmentAt={segmentAt} countryTags={lineTags}
    label="Army size" empty={`No known army sizes on ${dayLabel(moment)}.`} formatValue={formatArmyValue}
    onViewChange={onViewChange}
    notRecorded={lines.every(line => !line.runs.length) ? prepared.unrecorded : null}
    lineSummaries={prepared.summaries.size ? prepared.summaries : undefined} lineData={prepared.lineData} onSummaryView={prepared.onSummaryView} liveHead={liveHead}
    exactLineTags={dense ? prepared.exactLineTags : undefined} onReachHour={dense ? setReachedHour : undefined}
    navigation={dense ? prepared : undefined}
    beforeChart={pending || !incomingMetadata.data || grouped && !incomingGroups.data
      ? <WidgetState kind="loading" label="Loading army data…" />
      : !lines.length ? <WidgetState kind="empty" icon="shield" message="No known army sizes in this view." /> : undefined}
    labels={{ race: hour => `Army size, ${title}, ${dayLabel(hour)}`, lines: `Army size over time, ${title}`,
      mix: name => `Army size of ${name} by unit group, Battalions`, actions: 'Army size actions' }}
    raceLegendParts={[]} partsLabel="Battalions by unit group"
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
  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const pinHeld = drillControl.gate.checkHold(pinTag ? { tag: pinTag, origin: originHour } : null);
  const pinCountry = pinHeld.held;
  return <div className="army-size-drill parts-race" ref={drillAnchor}>{groupNotice}
    <DrillHost drill={{ ...drillControl, open: drillOpen, follow }} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={drillAnchor}
      title="Army breakdown" day={drillFollowing && track ? <LiveText live={dayLive} at="day" /> : dayLabel(drillAt)} canFollow={choices === null && track !== undefined}
      returnFocus=".race-chart canvas, .time-chart canvas"
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={track && pinCountry && pinTime ? () => runtime.addWidgetBelow(widget.id, pinArmyDrill(widget, track, pinCountry, pinTime, measure)) : undefined}
      main={main}>
      {choice}
    </DrillHost>
  </div>;
}
