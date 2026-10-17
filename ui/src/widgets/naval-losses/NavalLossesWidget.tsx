import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import DrillHost from '../../charts/drill/DrillHost';
import DrillParts from '../../charts/drill/DrillParts';
import { drillHour, pinnedBreakdown } from '../../charts/drill/drillState';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill } from '../../charts/race/partsRaceDrill';
import CountryPicker from '../../components/CountryPicker';
import { dayLabel } from '../../charts/draw';
import { runAt } from '../../charts/series';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import WidgetState from '../../components/WidgetState';
import DatedCountryChip, { ChooseCountryChip } from '../../dashboard/DatedCountryChip';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { partColours } from '../../charts/race/countryRace';
import { useChartTokens } from '../../charts/useChartTokens';
import { CHART_LAYOUT } from '../../charts/tokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { countryDrillSpans, countryDrillTagAt, countryIdentityIndex } from '../../dashboard/countryIdentity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { NumericReply } from '../../dashboard/numericReply';
import { usePreparedNaval } from './preparedNaval';
import { pendingReplyAt } from '../../dashboard/preparedPrefix';
import { useParityRows } from '../../dashboard/parityRows';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { NavalLossesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { navalPinOrigin } from './navalPinOrigin';
import { NAVAL_PARTS, navalChoiceReads, navalDrillActors, navalHolderBands, formatNavalCount, navalKindLabel, navalLines, navalLineTooltip, navalLookHour, navalModel, navalNumbers, navalPeriodLabel, navalReadHour, navalRequest, navalSegmentAt,
  navalTooltip, rankNaval } from './navalLossesData';

const NO_WARNINGS: string[] = [];

export default function NavalLossesWidget({ widget, runtime, active, countrySlot, onDiagnostics }: WidgetProps) {
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const tokens = useChartTokens();
  const partColors = useMemo(() => partColours(NAVAL_PARTS, tokens.categories), [tokens.categories]);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'lines' ? 'lines' : 'race';
  const kind = options.kind === 'ships' || options.kind === 'convoys' ? options.kind : 'both';
  const attribution = options.attribution === 'sunkBy' ? 'sunkBy' : 'lost';
  const period = options.period === 'month' ? 'month' : 'running';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const params = track ? navalRequest(track.branch.id, resolved, kind, attribution, period) : null;
  const request = track && params && (params.top !== null || params.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'naval-losses', params } : null;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const extent: [number, number] = track ? widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const playhead = (widget.time.mode === 'follow' || widget.time.mode === 'pin') && window ? window.lastHour : null;
  const wantedMoment = Math.floor(playhead ?? extent[1]);
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const prepared = usePreparedNaval(runtime.client,track,params,wantedMoment,extent,view,active && (!grouped || !!groups.data),parity?.requested);
  const {data,error,retry} = prepared;
  const moment = prepared.hour;
  const model = useMemo(() => data ? navalModel(data) : null, [data]);
  const identities = useMemo(() => grouped && groups.data ? countryIdentityIndex(groups.data.countryIdentities) : model?.identities ?? null,
    [grouped, groups.data, model]);
  const counted = useMemo(() => projectResolvedCountries(resolved, countryMatchFilter(widget.countries, fallback ? [fallback] : [],
    groups.data, track?.branch.lastHour ?? 0, identities, track?.branch.firstHour ?? 0)),
    [resolved, widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour, identities]);
  const slots = resolved.top ?? Math.max(1, counted.tags.length);
  const ranked = useMemo(() => model ? rankNaval(model, rankedCandidates(counted, moment, model.tags), moment, kind, slots) : [],
    [model, counted, moment, kind, slots]);
  const lineTags = useMemo(() => prepared.dense ? [...prepared.summaries.keys()] : resolved.top == null ? counted.tags
    : model?.reply.series.filter(series=>series.runs.length).map(series=>series.tag)??[], [resolved.top,counted,model,prepared.dense,prepared.summaries]);
  const lines = useMemo(() => {
    if(!model || view!=='lines') return [];
    const lines = navalLines(model,lineTags,kind,{firstHour:extent[0],lastHour:extent[1]},counted.membership);
    if(prepared.dense) for(const tag of prepared.summaries.keys()) if(!lines.some(line=>line.tag===tag)) lines.push({tag,runs:[]});
    return lines;
  },[model,view,lineTags,kind,extent[0],extent[1],counted.membership,prepared.dense,prepared.summaries]);
  const segmentAt = useCallback((tag: string, hour: number) => navalSegmentAt(model, tag, hour), [model]);
  const countryLookHour = useCallback((tag: string | null, hour: number) => navalLookHour(model, tag, hour, kind), [model, kind]);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its naval losses." />;
    if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
    if (grouped && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="unavailable" message={resolved.reason} />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
    if (moment < track.branch.firstHour || moment > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
    if (!data || !model || (grouped && !groups.data)) return <WidgetState kind="loading" label="This range is still being prepared." />;
    if (pendingReplyAt(prepared.manifest, wantedMoment)) return <WidgetState kind="loading" label="This range is still being prepared." />;
    if (!data.available) return <WidgetState kind="unavailable" message="Naval losses were not recorded at this time." />;
    return undefined;
  })();
  const scopeKey = JSON.stringify([request, widget.countries, widget.time, view, options.show, widget.options?.drillOriginOffset, widget.options?.drillOriginTag]);
  const scope = useMemo(() => ({ client: runtime.client, key: scopeKey }), [runtime.client, scopeKey]);
  const stores = useMemo(() => ({ main: new NumericReply<NavalLossesReply>(), drill: new NumericReply<NavalLossesReply>() }), [runtime.client]);
  const numbers = stores.main.update(scopeKey, data, navalNumbers);
  const interaction = useMemo(() => ({ scope, active, available: !state }), [scope, active, !state]);
  const membershipKey = useMemo(() => JSON.stringify([...(counted.membership ?? [])]), [counted.membership]);
  const geometryKey = JSON.stringify([extent, membershipKey, options.playback, options.windowDays, options.afterPlayhead, options.log, options.yMin, options.yMax]);
  const pickOwner = useMemo(() => ({ interaction, numbers, geometryKey }), [interaction, numbers, geometryKey]);
  const [storedDrill, dispatch] = useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE);
  const [drillOwner, setDrillOwner] = useState(scope);
  const [chooserOwner, setChooserOwner] = useState<typeof pickOwner | null>(null);
  const [drillOrigin, setDrillOrigin] = useState<{ scope: typeof scope; tag: string; hour: number } | null>(null);
  const drill = drillOwner === scope && (!storedDrill.choices || chooserOwner === pickOwner) ? storedDrill : CLOSED_PARTS_RACE;
  usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const anchor = useRef<HTMLDivElement>(null);
  const pinned = options.show === 'parts';
  const { tag: restoreTag, hour: restoreHour } = navalPinOrigin(widget, track);
  const restoreOrigin = restoreTag !== null;
  const restoreKnown = !!track && restoreTag !== null && restoreHour !== null && restoreHour >= track.branch.firstHour
    && restoreHour <= track.branch.lastHour && !!model?.identities?.keyAt(restoreTag, restoreHour);
  const clockHour = track ? Math.floor(runtime.clock.getHour()) + (track.axisOffset ?? 0) : null;
  const liveHour = clockHour !== null && track && clockHour >= track.branch.firstHour && clockHour <= track.branch.lastHour ? clockHour : null;
  const selectedHour = Math.floor(drill.breakdown === null && !drill.choices ? moment : drillHour(drill, clockHour) ?? moment);
  const detailExtent: [number, number] = track ? [track.branch.firstHour, track.branch.lastHour] : extent;
  const drillOpen = kind === 'both' && (pinned || drill.breakdown !== null || !!drill.choices);
  const drillCandidates = useMemo(() => model && drillOpen ? rankedCandidates(counted, selectedHour, model.tags) : [],
    [model, drillOpen, counted, selectedHour]);
  const drillTag = drill.breakdown;
  // The race reply is clipped to Top N; a chosen actor needs its full cached history.
  const drillRequest = track && drillOpen && drillTag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'naval-losses', params: navalRequest(track.branch.id, { tags: [drillTag] }, kind, attribution, period) } : null;
  const drillReply = usePreparedNaval(runtime.client,track,drillRequest?.params??null,selectedHour,detailExtent,'mix',active && drillOpen,parity?.requested);
  const drillNumbers = stores.drill.update(JSON.stringify([scopeKey, drillRequest]), drillReply.data, navalNumbers);
  const detailAvailable = active && !!drillReply.data?.available && !drillReply.error && !!track
    && selectedHour >= track.branch.firstHour && selectedHour <= track.branch.lastHour;
  const detailGeometry = JSON.stringify([geometryKey, detailExtent]);
  const detailOwner = useMemo(() => ({ scope, drillNumbers, drillTag, detailGeometry, detailAvailable }), [scope, drillNumbers, drillTag, detailGeometry, detailAvailable]);
  const pickRef = useRef({ main: pickOwner, detail: detailOwner, choices: drill.choices, mainAllowed: false, detailAllowed: false, chooserAllowed: false });
  pickRef.current = { main: pickOwner, detail: detailOwner,
    choices: drill.choices, chooserAllowed: active && !state && !!drill.choices,
    mainAllowed: active && !state && !pinned && !(drillOpen && options.drill !== 'popout'),
    detailAllowed: active && drillOpen && !drill.choices && !!drillTag };
  const drillModel = useMemo(() => drillReply.data ? navalModel(drillReply.data) : null, [drillReply.data]);
  const originHour = drillOrigin?.scope === scope && drillOrigin.tag === drillTag ? drillOrigin.hour : selectedHour;
  const holderSpans = useMemo(() => drillModel && drillTag && (!restoreOrigin || drillModel.identities?.keyAt(drillTag, originHour))
    ? countryDrillSpans(drillModel.reply.series, drillTag, originHour) : new Map<string, readonly {firstHour:number;lastHour:number}[]>(), [drillModel, drillTag, originHour, restoreOrigin]);
  const shownDrillTag = countryDrillTagAt(holderSpans, selectedHour);
  const drillTooltipTags = useCallback((hour: number) => {
    const tag = countryDrillTagAt(holderSpans, hour);
    return tag ? [tag] : [];
  }, [holderSpans]);
  const drillSegmentAt = useCallback((tag: string, hour: number) => navalSegmentAt(drillModel, tag, hour), [drillModel]);
  const drillLookHour = useCallback((tag: string | null, hour: number) => navalLookHour(drillModel, tag, hour, kind), [drillModel, kind]);
  const selectorTags = useMemo(() => [...new Set([...(shownDrillTag ? [shownDrillTag] : []), ...drillCandidates])], [shownDrillTag, drillCandidates]);
  const selectorReads = useMemo(() => {
    const reads = new Map(navalChoiceReads(model, selectorTags, selectedHour, kind, track?.branch.id ?? ''));
    for (const [tag, read] of navalChoiceReads(drillModel, shownDrillTag ? [shownDrillTag] : [], selectedHour, kind, track?.branch.id ?? '')) {
      reads.set(tag, read);
    }
    return reads;
  }, [selectorTags, shownDrillTag, selectedHour, drillModel, model, kind, track?.branch.id]);
  const selectedRead = shownDrillTag ? selectorReads.get(shownDrillTag) : null;
  const wanted = useMemo(() => selectedRead ? [{ tag: selectedRead.tag, hour: selectedRead.hour }] : [], [selectedRead]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const drillName = selectedRead ? labelAt(selectedRead.hour)(selectedRead.tag).name : shownDrillTag ?? drillTag ?? 'Country';
  useEffect(() => { dispatch({ type: 'close' }); }, [scope]);
  useEffect(() => {
    if (!active || !pinned || kind !== 'both' || drill.breakdown !== null || drill.choices || !model || state) return;
    if (restoreOrigin) {
      if (!restoreKnown || restoreTag === null || restoreHour === null) return;
      setDrillOwner(scope);
      setDrillOrigin({ scope, tag: restoreTag, hour: restoreHour });
      dispatch({ type: 'open', breakdown: restoreTag, hour: moment,
        playing: false, playhead: widget.time.mode === 'follow' ? liveHour : null });
      return;
    }
    const actors = rankNaval(model, drillCandidates, moment, kind, slots).map(row => row.tag);
    setDrillOwner(scope);
    if (actors.length === 1) {
      setDrillOrigin({ scope, tag: actors[0], hour: moment });
      dispatch({ type: 'open', breakdown: actors[0], hour: moment,
        playing: false, playhead: widget.time.mode === 'follow' ? liveHour : null });
    }
    else if (actors.length) {
      setChooserOwner(pickOwner); dispatch({ type: 'actors', key: scopeKey, tags: actors, hour: moment });
    }
  }, [active, pinned, kind, drill.breakdown, drill.choices, model, state, restoreOrigin, restoreKnown, restoreTag, restoreHour,
    drillCandidates, moment, slots, scope, scopeKey, pickOwner, widget.time.mode, liveHour]);
  useEffect(() => {
    const warnings = [...new Set([...(data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...(drillReply.data?.warnings ?? [])])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, drillReply.data, onDiagnostics]);
  const parityRoot = import.meta.env.MODE === 'parity' ? useRef<HTMLDivElement>(null) : null;
  if (import.meta.env.MODE === 'parity') {
    const complete = parity!.requested && prepared.complete && !!data;
    useParityRows(anchor,()=>({kind:'windowed',label:'Naval losses exact values',complete,total:data?.series.reduce((n,series)=>n+series.runs.length,0)??0,
      rows:data?.series.flatMap(series=>series.runs.map(run=>({tag:series.tag,run,identities:series.identities,membership:series.membership})))??[]}),
      {prepare:parity!.prepare,ready:()=>complete,error:()=>error??null});
  }
  const label = `${navalKindLabel(kind)} ${attribution === 'lost' ? 'lost' : 'sunk by'}`;
  const open = (hour: number, tag?: string, choices?: readonly string[]) => {
    if (!mounted.current || !pickRef.current.mainAllowed || pickRef.current.main !== pickOwner || !model) return;
    hour = Math.floor(hour);
    if (hour < extent[0] || hour > extent[1]) return;
    const actors = navalDrillActors(model, partsRaceDrillActors(tag, choices, () => null), hour, kind);
    setDrillOwner(scope);
    if (actors.length > 1) {
      setChooserOwner(pickOwner); dispatch({ type: 'actors', key: scopeKey, tags: actors, hour });
    } else if (actors.length) {
      setDrillOrigin({ scope, tag: actors[0], hour });
      dispatch({ type: 'open', breakdown: actors[0], hour,
        playing: runtime.clock.getSnapshot().playing, playhead: liveHour });
    }
    else dispatch({ type: 'close' });
  };
  const follow = () => {
    if (!mounted.current || !active || !drillOpen || !drillTag || !detailAvailable || pickRef.current.detail !== detailOwner || liveHour === null) return;
    setDrillOwner(scope); dispatch({ type: 'follow' });
  };
  const main = <CountryRaceView runtime={runtime} track={track} state={state} active={active && !(drillOpen && options.drill !== 'popout')} options={options} view={view}
    extent={extent} playhead={playhead} moment={moment}
    lineSummaries={prepared.dense ? prepared.summaries : undefined} onSummaryView={next=>prepared.onSummaryView(next,(prepared.manifest?.series??[]).flatMap(series=>{
      const membership=counted.membership?.get(series.tag);
      return [{id:series.tag,spans:(series.identities??series.segments).flatMap(identity=>{
        const ranges=counted.membership?membership??[]:[identity];
        return ranges.flatMap(range=>{const firstHour=Math.max(identity.firstHour,range.firstHour),lastHour=Math.min(identity.lastHour,range.lastHour);
          return firstHour<=lastHour?[{tag:series.tag,firstHour,lastHour,startsSegment:true}]:[];});
      })}];
    }))}
    onReachHour={prepared.onReachHour} exactLineTags={prepared.exactLineTags}
    navigation={{onNavigateHour:prepared.onNavigateHour,keyboardHour:prepared.keyboardHour,keyboardRevision:prepared.keyboardRevision}} slots={slots} ranked={ranked} lines={lines} segmentAt={segmentAt}
    parts={NAVAL_PARTS} raceLegendParts={kind === 'both' ? NAVAL_PARTS : []} partsLabel="Naval losses"
    labels={{ race: `${label}, ${dayLabel(moment)}`, lines: `${label} over time`, actions: 'Naval loss actions' }}
    onPick={kind === 'both' ? open : undefined} pickOwner={pickOwner} resetKey={scopeKey}
    label={label} empty="Naval losses unknown at this time." formatValue={formatNavalCount}
    countryTags={view === 'race' ? ranked.map(entry => entry.tag) : lineTags} countryLookHour={countryLookHour}
    beforeChart={data && <p className="chart-line-key">{navalPeriodLabel(data, period)}</p>}
    presentation={{ exact: false, held: false, lineColourHours: 'labels', linePadding: CHART_LAYOUT.countryEndLabels, raceEmptyState: false, resetTimeView: true }}
    lineEmpty={{ icon: 'shield', message: 'No known naval counts for these countries in this time range.' }}
    tooltip={(tag, hour, name) => model ? [navalTooltip(model, tag, hour, name, kind, attribution)] : null}
    lineTooltip={(hour, tags, labelAt, country) => model ? [navalLineTooltip(model, hour, tags, labelAt, country, kind, attribution)] : []} />;

  const run = drillOpen && drillModel && shownDrillTag ? runAt(drillModel.series.get(shownDrillTag)?.runs ?? [], selectedHour) : null;
  const bandModel = useMemo(() => drillReply.dense && drillReply.sampled ? navalModel(drillReply.sampled) : drillModel,[drillReply.dense,drillReply.sampled,drillModel]);
  const bands = useMemo(() => drillOpen && bandModel && drillTag
    ? navalHolderBands(bandModel, holderSpans, { firstHour: detailExtent[0], lastHour: detailExtent[1] }) : [],
    [drillOpen, bandModel, drillTag, holderSpans, detailExtent[0], detailExtent[1]]);
  const parts = NAVAL_PARTS.map(part => ({ ...part, color: partColors.get(part.id), value: run?.[part.id === 'ships' ? 'ships' : 'convoys'] ?? 0,
    text: run?.[part.id === 'ships' ? 'ships' : 'convoys'] == null ? 'Unknown' : undefined }));
  if (import.meta.env.MODE === 'parity') {
    const complete = parity!.requested && drillReply.complete && (!drillOpen || !!drillReply.data);
    useParityRows(parityRoot!,()=>({kind:'table',label:'Naval loss breakdown',complete,total:parts.length,
      rows:parts.map(part=>({id:part.id,label:part.label,value:part.text?null:part.value})),hour:selectedHour,tag:shownDrillTag}),
      {prepare:parity!.prepare,ready:()=>complete,error:()=>drillReply.error??null});
  }
  const drillState = (() => {
    if (!track) return state;
    if (restoreOrigin && ((model && !restoreKnown) || (drillTag && drillModel && !drillModel.identities?.keyAt(drillTag, originHour)))) {
      return <WidgetState kind="unavailable" message="Saved country selection unavailable. Choose it again." />;
    }
    if (!drillTag) return state;
    if (((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window)
      || selectedHour < track.branch.firstHour || selectedHour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
    if (drillReply.error) return <WidgetState kind="error" detail={drillReply.error} onRetry={drillReply.retry} />;
    if (drillTag && (!drillReply.data || !drillModel)) return <WidgetState kind="loading" label="This range is still being prepared." />;
    if (drillReply.data && !drillReply.data.available) return <WidgetState kind="unavailable" message={`Naval losses not recorded for ${drillName}.`} />;
    if (drillTag && drillModel && shownDrillTag === null) return <WidgetState kind="unavailable" message="Country unavailable at this time." />;
    return undefined;
  })();
  pickRef.current.detailAllowed &&= !drillState;
  const originKey = drillTag ? drillModel?.identities?.keyAt(drillTag, originHour) : null;
  const headerKnown = !!originKey && shownDrillTag !== null && drillModel?.identities?.keyAt(shownDrillTag, selectedHour) === originKey;
  const headerAvailable = !!countrySlot && pinned && kind === 'both' && active && !drillState && headerKnown;
  const headerWanted = useMemo(() => headerAvailable && shownDrillTag ? [{ tag: shownDrillTag, hour: selectedHour }] : [],
    [headerAvailable, shownDrillTag, selectedHour]);
  const { labelAt: headerLabelAt } = useCountryLabelsAt(runtime.client, track, headerWanted, headerWanted.length > 0);
  const headerLabel = headerWanted.length ? headerLabelAt(selectedHour)(headerWanted[0].tag) : null;
  const headerReason = drillState ? 'This breakdown is unavailable at this hour.'
    : !headerKnown ? 'Country unavailable at this time.' : null;
  // Unpinned (a catalogue card), the header asks for a country until the body opens one.
  const header = countrySlot && pinned && kind === 'both' && createPortal(!restoreOrigin && !drillTag ? <ChooseCountryChip />
    : <DatedCountryChip label={headerLabel} hour={track ? selectedHour : null} reason={headerReason} />, countrySlot);
  const chooserRead = drill.choices && data ? navalReadHour(data, selectedHour, kind) : null;
  const chooserReads = useMemo(() => navalChoiceReads(model, drill.choices?.tags ?? [], selectedHour, kind, track?.branch.id ?? ''),
    [model, drill.choices?.tags, selectedHour, kind, track?.branch.id]);
  const chooser = drill.choices && track ? <div>
    <p className="drill-empty">Choose a country for {dayLabel(selectedHour)}.</p>
    <CountryPicker countries={track.source.metadata.countries} allowedTags={drill.choices.tags} selectedTag={null} playerCountryId={null}
      source={track.source} client={runtime.client} field flagAt={chooserRead !== null ? { branchId: track.branch.id, hour: chooserRead } : null}
      choiceReads={chooserReads}
      onChange={tag => {
        if (!mounted.current || !pickRef.current.chooserAllowed || pickRef.current.main !== chooserOwner || pickRef.current.choices !== drill.choices || !drill.choices || !tag) return;
        if (!model || !navalDrillActors(model, [tag], selectedHour, kind).length) return;
        setDrillOrigin({ scope, tag, hour: selectedHour });
        dispatch({ type: 'choose', key: scopeKey, tag, hour: selectedHour });
      }} />
  </div> : null;
  const breakdown = <CountryRaceView runtime={runtime} track={track} active={active && drillOpen} options={options} view="mix" state={drillState}
    extent={detailExtent} onSummaryView={next=>drillReply.onSummaryView(next,(drillReply.manifest?.series??[]).map(series=>({id:series.tag,
      spans:(holderSpans.get(series.tag)??[]).map(span=>({tag:series.tag,firstHour:span.firstHour,lastHour:span.lastHour,startsSegment:true}))})))} onReachHour={drillReply.onReachHour}
    navigation={{onNavigateHour:drillReply.onNavigateHour,keyboardHour:drillReply.keyboardHour,keyboardRevision:drillReply.keyboardRevision}}
    playhead={selectedHour} selectedHour={selectedHour} moment={selectedHour} slots={1} ranked={[]} lines={[]} parts={NAVAL_PARTS}
    mix={drillTag ? { tag: shownDrillTag ?? drillTag, bands, parts: NAVAL_PARTS } : null} segmentAt={drillSegmentAt} countryLookHour={drillLookHour} label={label}
    labels={{ mix: name => `${label} of ${name}, ships and convoys`, actions: 'Naval loss actions' }}
    empty="Naval losses unknown at this time." formatValue={formatNavalCount}
    beforeChart={<div ref={parityRoot}>{pinned && !drill.follow && liveHour !== null && <button type="button" className="button small" onClick={follow}>Follow playback</button>}
      {track && <CountryPicker countries={track.source.metadata.countries} allowedTags={selectorTags} selectedTag={shownDrillTag} playerCountryId={null}
        source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: selectedRead?.hour ?? selectedHour }}
        choiceReads={selectorReads} onChange={tag => {
        if (!mounted.current || !pickRef.current.detailAllowed || pickRef.current.detail !== detailOwner || !tag || !selectorTags.includes(tag)) return;
        const choiceModel = tag === shownDrillTag ? drillModel : model;
        if (!choiceModel || !navalDrillActors(choiceModel, [tag], selectedHour, kind).length) return;
        setDrillOrigin({ scope, tag, hour: selectedHour });
        dispatch({ type: 'open', breakdown: tag, hour: selectedHour, playing: drill.follow, playhead: drill.follow ? liveHour : null });
      }} />}
      <DrillParts parts={parts} format={formatNavalCount} positiveBars signedShares total={run?.ships != null && run?.convoys != null ? run.ships + run.convoys : 0}
        totalRow={{ label: 'Total', value: run?.ships != null && run?.convoys != null ? formatNavalCount(run.ships + run.convoys) : 'Unknown' }} /></div>}
    mixEmpty={{ icon: 'shield', message: 'No country with known naval losses at this time.', unavailable: name => `Naval losses unknown for ${name}.` }}
    pickOwner={detailOwner} resetKey={scopeKey} onPick={drillTag ? hour => {
      if (!mounted.current || !pickRef.current.detailAllowed || pickRef.current.detail !== detailOwner) return;
      hour = Math.floor(hour);
      if (hour >= detailExtent[0] && hour <= detailExtent[1]) dispatch({ type: 'open', breakdown: drillTag, hour, playing: false, playhead: null });
    } : undefined}
    tooltipTags={drillTooltipTags} tooltip={(_tag, hour, _name, _colours, labelAt) => {
      const tag = countryDrillTagAt(holderSpans, hour);
      return drillModel && tag ? [navalTooltip(drillModel, tag, hour, labelAt(hour)(tag).name, 'both', attribution)] : null;
    }} />;
  if (pinned && kind === 'both') return <>{header}{chooser ?? breakdown}</>;
  return <div ref={anchor} style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={anchor}
      title={`${label}: ${drillName}`} day={dayLabel(selectedHour)} following={drill.follow} canFollow={!drill.choices && !!drillTag && liveHour !== null}
      onClose={() => { if (mounted.current && active && pickRef.current.main === pickOwner) dispatch({ type: 'close' }); }} onFollow={follow}
      onPin={track && drillTag && !drillState ? () => {
        if (!mounted.current || !pickRef.current.detailAllowed || pickRef.current.detail !== detailOwner) return;
        const pinned = pinnedBreakdown(drill.follow ? { ...widget, time: { mode: 'follow' } } : widget, { id: 'parts', label: 'Ships and convoys breakdown' }, track.id);
        const offset = Math.floor(selectedHour) - track.branch.firstHour;
        runtime.addWidgetBelow(widget.id, { ...pinned, countries: { kind: 'tags', tags: [shownDrillTag ?? drillTag] },
          options: { ...pinned.options, drillOriginOffset: offset, drillOriginTag: setToken(shownDrillTag ?? drillTag) },
          time: drill.follow ? pinned.time : { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } } });
      } : undefined}
      main={main}>{chooser ?? breakdown}</DrillHost>
  </div>;
}
