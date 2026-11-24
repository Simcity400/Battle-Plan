import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { CLOSED_PARTS_RACE, reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';
import CountryPicker from '../../components/CountryPicker';
import { armyDrillActors, armyDrillNumbers } from './armyDrill';
import ArmyBreakdown from './ArmyBreakdown';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { countryValuesModel } from '../../charts/race/countryValues';
import DatedCountryChip, { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetProps } from '../../dashboard/definitions';
import { rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { NumericReply } from '../../dashboard/numericReply';
import { useProduct } from '../../dashboard/products';
import { useFrameWindows } from './frameWindows.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { ArmyAttribution, CountryGroupReply } from '../../generated/dashboard';
import type { ArmySizeReply } from './armySizeData';
import { armyAbsent, armyMeasure, armyMetadataRequest, armySizeRequest, armyWindows,
  mergeArmyWindows, offersMeasure, validateArmySizeReply, validateArmyWindow } from './armySizeData.ts';
import { normaliseArmyOptions, OPTIONS } from './definition';
import { armyCountrySelection } from './ArmyCountryChip';
import './armySize.css';
import { useHeadHour } from './headHour.ts';
import { useChangedState } from '../../dashboard/changedState.ts';
import { useDrill } from '../../charts/drill/useDrill';
import type { PartsRaceDrillAction, PartsRaceDrillState } from '../../charts/race/partsRaceDrill';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget, DetailWindowHost } from '../../families/host/index.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { parseChartInspect } from '../../families/time-chart/hitTest.ts';
import { timeChartHostAdapter } from '../../families/time-chart/hostAdapter.ts';
import { armyLinesDeclaration } from './armyLinesProjection.ts';
import { armySizeMixDeclaration } from './armySizeMixProjection.ts';
import { armySizeRaceDeclaration } from './armySizeRaceProjection.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';

const sameWarnings = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((warning, at) => warning === b[at]);

const NO_WARNINGS: string[] = [];

const HOLDERS = { holders: true };

export default function ArmySizeWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, normaliseArmyOptions(props.widget.options));
  if (options.view === 'breakdown') {
    return <ArmyBreakdownWidget {...props} />;
  }
  const familyView = options.view === 'lines' ? 'lines' : options.view === 'mix' ? 'mix' : 'race';
  return <ArmySizeFamilyWidget {...props} familyView={familyView} />;
}

function ArmySizeFamilyWidget(props: WidgetProps & { readonly familyView: 'race' | 'mix' | 'lines' }) {
  const { widget, runtime, active, familyView } = props;
  const options = resolveOptions(OPTIONS, normaliseArmyOptions(widget.options));
  const isLines = familyView === 'lines';
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const [detailWarnings, setDetailWarnings] = useChangedState<string[]>([], sameWarnings);
  const gate = useGate(runtime, widget, SPEC, active);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const countFor = options.countFor as ArmyAttribution;
  const metadataRequest = armyMetadataRequest(track);
  const metadata = useProduct<ArmySizeReply>(runtime.client, metadataRequest, active && detail !== null, validateArmySizeReply, track?.source);
  const measure = familyView === 'mix' ? 'battalions' : armyMeasure(options.measure, metadata.data?.measures);
  const extent: [number, number] = track ? [track.branch.firstHour, track.branch.lastHour] : [0, 1];

  const onActivate = useCallback((action: InstalledAction) => {
    if (isLines && parseChartInspect(action.subject) !== null) return;
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const meta = action.meta as { readonly tag?: string; readonly key?: string } | null | undefined;
    setDetail({ tag: meta?.tag ?? action.subject, key: meta?.key ?? action.subject, hour: gated });
  }, [gate, isLines]);

  return (
    <div ref={anchorRef} className="fh-frame">
      {isLines ? (
        <DeclaredFamilyWidget
          {...props}
          adapter={timeChartHostAdapter}
          view="lines"
          declaration={armyLinesDeclaration}
          extraWarnings={detailWarnings}
          onActivate={onActivate}
        />
      ) : familyView === 'mix' ? (
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

function ArmyBreakdownWidget({ widget, runtime, active, countrySlot = null, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, normaliseArmyOptions(widget.options));
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
  const invalidSavedSelection = Object.hasOwn(widget.options ?? {}, 'drillOriginHour') && !savedSelection;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
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
  const incomingGroups = useCountryGroups(runtime, track, active && grouped);
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
  const measure = !options.measure && metadata.data?.measures.includes('battalions') ? 'battalions'
    : armyMeasure(options.measure, metadata.data?.measures);
  const byType = measure === 'battalions';
  const content = useContentRevision(runtime.client);
  const identity = track && metadata.data && (resolved.top || resolved.tags.length)
    ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content,
      resolved.tags, resolved.top, measure, countFor, byType]) : null;
  const membershipKey = useMemo(() => JSON.stringify([...(resolved.membership ?? [])]), [resolved.membership]);
  const windowsAt = (hour: number) => {
    const at = momentAt(hour);
    const range = { firstHour: at, lastHour: at };
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
  missingKey.current = hour => windowsAt(hour).required.filter(window => !history.holds(window)).join(',');
  const incomingData = useMemo(() => {
    if (!metadata.data) return undefined;
    return mergeArmyWindows(metadata.data, history.data, [...history.data.keys()]);
  }, [metadata.data, history.data]);
  const pending = required.some(index => !history.data.has(index));
  const error = required.map(index => history.errors.get(index)).find(Boolean);
  const numericScope = JSON.stringify([sourceScope, resolved.tags, resolved.top, membershipKey, measure, countFor, byType, extent]);
  const previousNumbers = stores.numbers.update(numericScope, undefined, armyDrillNumbers);
  const numbers = stores.numbers.update(numericScope, pending && previousNumbers ? undefined : incomingData, armyDrillNumbers);
  const incomingLabels = useMemo(() => incomingData ? { types: incomingData.types, warnings: incomingData.warnings } : undefined, [incomingData]);
  const labels = stores.labels.update(numericScope, pending ? undefined : incomingLabels, reply => reply);
  const shown = numbers ?? incomingData;
  const data = useMemo(() => shown ? { ...shown, ...labels } : undefined, [shown, labels]);
  const model = useMemo(() => data ? countryValuesModel(data) : null, [data]);
  const resetKey = JSON.stringify([track?.source.id, track?.branch.id,
    measure, countFor, widget.countries, widget.time, options.view, widget.options?.drillOriginHour]);
  const choices = drill.choices?.key === resetKey ? drill.choices : null;
  useEffect(() => { dispatchDrill({ type: 'close' }); setDrillOrigin(null); }, [resetKey]);
  const drillTag = drill.breakdown;
  const clockHour = track ? Math.floor(playbackHour) + track.axisOffset : null;
  const drillHour = choices !== null ? drill.hour : drillControl.following ? clockHour : drillControl.hour;
  const drillAt = Math.floor((drillControl.open ? drillHour : moment) ?? moment);
  const drillPlayhead = drillControl.following ? clockHour : playhead;
  const originHour = drillOrigin?.tag === drillTag ? drillOrigin.hour : drillAt;
  const follow = () => { if (active && track) drillControl.follow(); };
  useEffect(() => {
    if (invalidSavedSelection || !window || drill.breakdown !== null || drill.choices !== null || !model) return;
    const fixed = savedSelection?.tag;
    const actors = fixed ? [fixed] : armyDrillActors(model,
      widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? model.tags
        : rankedCandidates(resolved, moment, model.tags), moment, resolved.top ?? undefined);
    if (actors.length === 1) {
      setDrillOrigin({ tag: actors[0], hour: savedSelection?.hour ?? moment });
      drillControl.show(actors[0], moment);
    } else if (actors.length > 1) dispatchDrill({ type: 'actors', key: resetKey, tags: actors, hour: moment });
  }, [invalidSavedSelection, !!window, drill, model, resolved, moment, playhead, savedSelection?.tag, savedSelection?.hour, widget.countries, widget.time.mode, resetKey]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? metadata.data?.warnings ?? []), ...(groupWarnings ?? []), ...drillWarnings];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, metadata.data, groupWarnings, drillWarnings, onDiagnostics]);

  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its army sizes." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (invalidSavedSelection) return <WidgetState kind="unavailable" message="Saved country unavailable." />;
    if (metadata.error) return <WidgetState kind="error" failure={asFailure(metadata.error)} onRetry={metadata.retry} />;
    if (error) return <WidgetState kind="error" failure={error} onRetry={history.retry} />;
    if (grouped && groupReply.error && !groupReply.data) return <WidgetState kind="error" failure={groupReply.error} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="shield" message={resolved.reason} />;
    if (metadata.data && !offersMeasure(metadata.data, measure)) {
      return <WidgetState kind="unavailable" message="Not in this recording" />;
    }
    if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label="Loading army sizes…" />;
    if (!metadata.data || armyAbsent(metadata.data)) return <WidgetState kind="unavailable" message="Not in this recording" />;
    return undefined;
  })();

  const hasOrigin = Object.hasOwn(widget.options ?? {}, 'drillOriginHour');
  const unchosen = !hasOrigin && drillTag === null;
  const neutralHeader = countrySlot && (state !== undefined || choices !== null || unchosen)
    ? createPortal(unchosen ? <ChooseCountryChip /> : <DatedCountryChip label={null} hour={window ? drillAt : null}
      reason={!savedSelection ? 'Saved country unavailable.'
        : 'Country unavailable at this time.'} />, countrySlot) : null;
  const breakdown = <ArmyBreakdown runtime={runtime} gate={gate} track={track} active={active} options={options} extent={extent}
    countrySlot={state === undefined && choices === null && !unchosen ? countrySlot : null}
    headerSelection={hasOrigin || drillTag === null ? savedSelection : { tag: drillTag, hour: originHour }}
    playhead={drillPlayhead} tag={drillTag} hour={drillAt} measure={measure} countFor={countFor} manifest={metadata.data}
    originHour={originHour} following={drill.follow} pinned onWarnings={setDrillWarnings}
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
  return <>{neutralHeader}{groupNotice}<div ref={drillAnchor} className="army-size-drill parts-race-drill">{state ?? choice}</div></>;
}
