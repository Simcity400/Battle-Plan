import { useHeldEdges } from '../../dashboard/regionGate';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import type { DrillAction } from '../../charts/drill/drillState';
import { clipToSpans } from '../../charts/series';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { chosenDatedCountry } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions } from '../../dashboard/options';
import type { EquipmentStockpileReply } from './types';
import type { LiveValues } from '../../charts/live';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { atReply } from './stockpileAt.ts';
import { BREAKDOWN, OPTIONS } from './definition';
import { hasRecords, choiceKey, labelAt, stockGroups } from './stockpileData.ts';
import { stockpileView } from './stockpileCatalogue';
import type { StockGroup } from './stockpileData';
import { useEquipmentStockpile, useStockpileCountry } from './useEquipmentStockpile';
import StockpileDrill from './StockpileDrill.tsx';
import StockpileTable from './StockpileTable';
import type { StockCount } from './stockpileView';
import { pinnedGrouping, pinStockpileDrill, stockpileDrillGroup } from './stockpileDrill';
import type { StockpileMode } from './stockpileDrill';
import './equipmentStockpile.css';
import { motionOf } from '../../charts/easing.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';
import { drillAction } from '../../charts/drill/drillState.ts';
import { useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';

const NO_WARNINGS: string[] = [];

const HOLDERS = { holders: true };

export default function EquipmentStockpileWidget(props: WidgetProps) {
  return props.widget.options?.show === BREAKDOWN.id ? <EquipmentStockpileCountry {...props} />
    : <CountryComparisons {...props} component={EquipmentStockpileCountry} />;
}

function EquipmentStockpileCountry({ widget, runtime, active, onDiagnostics, countrySlot, within }: ComparisonProps) {
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const saved = options.show === BREAKDOWN.id;
  const { track, tag, catalogue, data: reply, hour: viewHour, at: stock, error, retry } = useEquipmentStockpile(widget, runtime, active && !saved, within);
  const data = useMemo(() => reply && clipped(reply, within), [reply, within]);
  const axisOffset = track?.axisOffset ?? 0;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = useRegionHour(runtime, useRegion(widget), active);
  const menu = useContextMenu('Stockpile actions');
  const grouping = pinnedGrouping(widget.options);
  const count: StockCount = options.count === 'ic' ? 'ic' : 'units';
  const includedText = typeof options.included === 'string' ? options.included : '';
  const included = useMemo(() => contentSet(includedText).map(contentKey), [includedText]);
  const anchor = useRef<HTMLDivElement | null>(null);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, grouping, included, options.show, options.stockpileCountry, options.partGroup, widget.time, within]);
  const pickOwner = useMemo(() => ({ scope, client: runtime.client }), [scope, runtime.client]);
  const [heldEdges, holdEdges] = useHeldEdges();
  const drillNeeds = useMemo(() => ({ ...HOLDERS, changes: heldEdges }), [heldEdges]);
  const drillControl = useDrill(runtime, useRegion(widget), saved ? BREAKDOWN.id : null, undefined, undefined, drillNeeds);
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [drillOwner, setDrillOwner] = useState<object | null>(null);
  const [selection, setSelection] = useState<{ owner: object; key: string; mode: StockpileMode } | null>(null);
  const shownDrill = saved || drillOwner === pickOwner ? drill : CLOSED;
  const mode: StockpileMode = selection?.mode ?? 'stock';
  const playbackHour = Math.floor(runtime.clock.getHour()) + axisOffset;
  const detailTime = drillAction(shownDrill);
  const detailRegion = useRegion(widget, detailTime);
  const detailCoverage = useRegionCoverage(runtime, detailRegion);
  const at = useRegionHour(runtime, detailRegion, active && (saved || shownDrill.breakdown !== null), heldEdges);
  const detailHead = shownDrill.breakdown !== null && shownDrill.follow ? recordedHour(track, playbackHour) : playhead ?? head;
  const [automaticOrigin, setAutomaticOrigin] = useState<{ owner: object; hour: number } | null>(null);
  if (saved && !options.stockpileCountry && head !== null && automaticOrigin?.owner !== pickOwner) setAutomaticOrigin({ owner: pickOwner, hour: head });
  const origin = automaticOrigin?.owner === pickOwner ? automaticOrigin.hour : head;
  const savedCountry = useMemo(() => saved && track && (options.stockpileCountry || origin !== null) ? chosenDatedCountry(options.stockpileCountry, track.source.metadata.campaignKey,
    track.branch.id, widget.countries, tag, track.branch.firstHour, track.branch.lastHour, origin ?? Number.NaN) : null,
  [saved, options.stockpileCountry, widget.countries, tag, origin, track?.source.metadata.campaignKey, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour]);
  const [countryPick, setCountryPick] = useState<{ owner: object; selection: DatedCountrySelection } | null>(null);
  const selectedCountry = saved ? savedCountry : countryPick?.owner === pickOwner ? countryPick.selection : null;
  const missingSavedDate = saved && shownDrill.breakdown === null && (widget.time.mode === 'pin' || widget.time.mode === 'range') && !window;
  const actor = useStockpileCountry(selectedCountry, runtime, track, widget, active && (saved || shownDrill.breakdown !== null), detailTime, included);
  useEffect(() => holdEdges(actor.spans), [actor.spans, holdEdges]);
  const detailData = selectedCountry ? actor.data : undefined;
  useEffect(() => {
    const warnings = (saved || shownDrill.breakdown !== null ? detailData : data)?.warnings;
    onDiagnostics(warnings?.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, detailData, saved, shownDrill.breakdown, onDiagnostics]);
  const detailGroups = useMemo(() => detailData ? stockGroups(detailData, grouping, Infinity, included) : [], [detailData, grouping, included]);
  const picked = saved ? stockpileDrillGroup(detailGroups, String(options.partGroup ?? ''))
    : selection?.owner === pickOwner ? detailGroups.find(group => group.key === selection.key) : undefined;
  const headerTag = actor.spans && at !== null ? countryDrillTagAt(actor.spans, at) : null;
  const headerReason = missingSavedDate ? 'The chosen moment is unavailable.' : at === null ? 'This time is outside the recording.' : !selectedCountry ? options.stockpileCountry
    ? 'Choose a country again.'
    : 'Choose a country for this widget.'
    : actor.reason ?? (headerTag ? null : 'Country unavailable at this time.');
  const header = useCountryLabelsAt(runtime.client, track, saved && countrySlot && headerTag && !headerReason && at !== null ? [{ tag: headerTag, hour: at }] : [], active);
  const pinnedHeader = saved && countrySlot ? createPortal(<DatedCountryChip failure={actor.error} hour={missingSavedDate ? null : at} reason={headerReason}
    label={headerTag && !headerReason && at !== null ? header.labelAt(at)(headerTag) : null} />, countrySlot) : null;
  const detailPickOwner = useMemo(() => ({ pickOwner, selectedCountry }), [pickOwner, selectedCountry]);
  const showingDetail = saved || shownDrill.breakdown !== null;
  const shown = useShownHour();
  useEffect(() => { shown(showingDetail ? (detailData ? actor.hour : null) : (data ? viewHour : null)); });
  const headRoot = stock.root, actorRoot = actor.at.root;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; headRoot(element); actorRoot(element); }, [headRoot, actorRoot]);
  const ownerRef = useRef({ owner: pickOwner, active });
  ownerRef.current = { owner: pickOwner, active };
  useEffect(() => {
    ownerRef.current.active = active;
    return () => { ownerRef.current.active = false; };
  }, [active, pickOwner]);
  const act = (action: DrillAction) => { if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return; setDrillOwner(pickOwner); dispatch(action); };
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(null); setSelection(null); setCountryPick(null); }, [pickOwner]);

  if (!track || !tag) return <>{pinnedHeader}<WidgetState kind="empty" icon="grid" message="Load a campaign and choose a country to see its equipment stockpile." /></>;
  if (!saved && shownDrill.breakdown === null && playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <>{pinnedHeader}<WidgetState kind="outside" /></>;
  if (shownDrill.breakdown === null && (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Choose another time in widget settings." /></>;
  if (!saved && shownDrill.breakdown === null && error) return <>{pinnedHeader}<WidgetState kind="error" failure={error} onRetry={retry} /></>;
  if (!saved && shownDrill.breakdown === null && !data) return <>{pinnedHeader}<WidgetState kind="loading" label="Loading the stockpile…" /></>;
  if (!saved && shownDrill.breakdown === null && data && !hasRecords(data)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Stockpile data unavailable for this country." /></>;

  const jump = (hour: number) => [jumpItem(runtime, track.id, hour)];
  const open = (group: StockGroup, hour: number, mode: StockpileMode) => {
    if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return;
    setSelection({ owner: pickOwner, key: group.key, mode });
    setCountryPick({ owner: pickOwner, selection: { campaign: track.source.metadata.campaignKey, branch: track.branch.id,
      tag, originOffset: Math.floor(hour) - track.branch.firstHour } });
    act({ type: 'open', breakdown: BREAKDOWN.id, hour, follow: drillControl.followsAt(hour) });
  };
  const main = !saved && data && viewHour !== null ? <div className="equipment-stockpile">
    <StockpileTable data={data} hour={viewHour} widget={widget} live={stock.live} shownAt={stock.shownAt}
      at={(values: LiveValues<number>) => catalogue && stock.data ? clipped(stockpileView(catalogue, atReply(stock.data, values, viewHour, track.branch.id), viewHour), within) : undefined}
      included={included} count={count} narrow={narrowTableOf(options)}
      active={active && (shownDrill.breakdown === null || options.drill === 'popout')} viewport={viewport}
      runtime={runtime} track={track} onDiagnostics={onDiagnostics} onPick={open} />
  </div> : null;
  const identityNotice = selectedCountry && actor.identityError ? <WidgetState kind="error" failure={actor.identityError} onRetry={actor.identityRetry} /> : null;
  const detail = at === null ? <WidgetState kind="outside" read={detailCoverage} />
    : !selectedCountry ? <WidgetState kind="unavailable" message={headerReason!} />
    : actor.error ? <WidgetState kind="error" failure={actor.error} onRetry={actor.retry} />
      : actor.reason ? <WidgetState kind="unavailable" message={actor.reason} />
        : picked && detailData && actor.hour !== null ? <StockpileDrill data={detailData} catalogue={actor.catalogue} holders={actor.holders} included={included} group={picked}
    hour={actor.hour}
    client={runtime.client} gate={drillControl.gate} track={track}
    extent={extent} playhead={detailHead} playback="cursor" afterPlayhead selectedHour={at} branchStart={track.branch.firstHour}
    hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active && (saved || shownDrill.breakdown !== null)}
    pickOwner={detailPickOwner} onPick={hour => act({ type: 'pick', hour })}
    onContextMenu={(hour, x, y) => menu.open(x, y, jump(hour))} />
    : <p className="drill-empty">The chosen equipment or saved design is not defined by this campaign.</p>;
  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const pinHeld = drillControl.gate.checkHold(selectedCountry && track ? { tag: selectedCountry.tag, origin: track.branch.firstHour + selectedCountry.originOffset } : null);
  const pinCountry = pinHeld.held;
  return <div ref={root} className="stockpile-drill-host">
    {saved ? <div className="drill-body">
      {pinnedHeader}
      {(shownDrill.breakdown === null ? widget.time.mode !== 'follow' : !shownDrill.follow) && <button type="button" className="button small quiet"
        onClick={() => act({ type: 'follow' })}>Follow playback</button>}{identityNotice}{detail}
    </div> : <DrillHost drill={drillControl} runtime={runtime} subject={shownDrill.breakdown !== null} anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title={picked && at !== null ? labelAt(picked, grouping, at) : BREAKDOWN.label} day={at === null ? '' : dayLabel(at)}
      returnFocus=".stockpile-table-rows tr"
      canFollow onClose={() => act({ type: 'close' })} main={main}
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={picked && selectedCountry && !actor.reason && at !== null && pinTime && pinCountry && choiceKey(picked, at) !== null ? () => {
        if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return;
        const pinned = pinStockpileDrill(widget, track, pinCountry.tag, picked, grouping, mode, at, pinTime, pinCountry.token);
        if (pinned) runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>{identityNotice}{detail}</DrillHost>}
    {menu.menu}
  </div>;
}

function clipped(reply: EquipmentStockpileReply, within: ComparisonProps['within']): EquipmentStockpileReply {
  return within ? { ...reply, stockHours: clipToSpans(reply.stockHours, within), lineHours: clipToSpans(reply.lineHours, within),
    balanceHours: clipToSpans(reply.balanceHours, within), aiHours: clipToSpans(reply.aiHours, within) } : reply;
}
