import { useGate, useHeldEdges, type Gate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useCallback, useEffect, useMemo,  useRef, useState } from 'react';
import { narrowTableOf } from '../../charts/table/options.ts';
import { Select } from '../../components/Select';
import { createPortal } from 'react-dom';
import ContentSetPicker from '../../components/ContentSetPicker';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { extentOfSummary, pickSummaryLines } from '../../charts/chartSummary';
import { lineSummaryLayer, valueRuleLayer, type DeclarativeEndLabels } from '../../charts/renderTimeChart';
import { jumpItem } from '../../charts/jump';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import type { DrillAction } from '../../charts/drill/drillState';
import Legend from '../../charts/Legend';
import { lineStyles } from '../../charts/palette';
import { clipToSpans, lineEndHour, runAt } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { chosenShown, storedChoices, toggleChosen } from '../../dashboard/chosen';
import type { WidgetProps } from '../../dashboard/definitions';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { chosenDatedCountry } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import type { EquipmentStockpileReply, StockpileCatalogueReply } from './types';
import type { LiveHead } from '../../charts/TimeChart';
import type { LiveValues } from '../../charts/live';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { atReply } from './stockpileAt.ts';
import { BREAKDOWN, OPTIONS } from './definition';
import { AI_NOTE, automaticLines, CHOSEN_KEYS, hasRecords, balanceAt, balanceMode, choiceKey, choiceRow, formatPercent, oneChoicePerRow, GROUP_NOUNS, labelAt,
  SAVED_DESIGN_UNAVAILABLE, overTimeTooltip, shareModeOf, shareOf, stockAt, stockChoices, stockGroups, dailyAt, chartDrawable } from './stockpileData.ts';
import { useStockpileChart } from './useStockpileChart';
import { groupIn, stockpileGroupKey, stockpileView } from './stockpileCatalogue';
import type { PreparedStockpileReply } from './stockpileCatalogue';
import type { Grouping, StockGroup } from './stockpileData';
import { useEquipmentStockpile, useStockpileCountry } from './useEquipmentStockpile';
import StockpileDrill from './StockpileDrill.tsx';
import StockpileTable from './StockpileTable';
import type { StockCount } from './stockpileView';
import { pinnedGrouping, pinStockpileDrill, stockpileDrillGroup } from './stockpileDrill';
import type { StockpileMode } from './stockpileDrill';
import './equipmentStockpile.css';
import { motionOf } from '../../charts/easing.ts';
import type { StockpileHolder } from '../../generated/dashboard';
import { recordedHour } from '../../dashboard/regionTime.ts';
import { drillAction } from '../../charts/drill/drillState.ts';
import { useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';

const NO_WARNINGS: string[] = [];
const END_LABEL_ROOM = 136;

const HOLDERS = { holders: true };

export default function EquipmentStockpileWidget(props: WidgetProps) {
  return props.widget.options?.show === BREAKDOWN.id ? <EquipmentStockpileCountry {...props} />
    : <CountryComparisons {...props} component={EquipmentStockpileCountry} />;
}

function EquipmentStockpileCountry({ widget, runtime, active, onDiagnostics, setOption, countrySlot, within }: ComparisonProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const saved = options.show === BREAKDOWN.id;
  const { track, tag, holders, catalogue, data: reply, hour: viewHour, at: stock, error, retry } = useEquipmentStockpile(widget, runtime, active && !saved, within);
  const data = useMemo(() => reply && clipped(reply, within), [reply, within]);
  const axisOffset = track?.axisOffset ?? 0;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const liveHead = useMemo<LiveHead | undefined>(() => widget.time.mode === 'follow'
    ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined, [widget.time.mode, runtime.clock, axisOffset]);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = useRegionHour(runtime, useRegion(widget), active);
  const gate = useGate(runtime, widget, SPEC, active);
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
  const groups = useMemo(() => data ? stockGroups(data, grouping, Infinity, included) : [], [data, grouping, included]);
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
    {options.view === 'time'
      ? <OverTime data={data} catalogue={catalogue} holders={holders} included={included} tokens={tokens} viewport={viewport} options={options} runtime={runtime} gate={gate} track={track}
        extent={extent} playhead={playhead} liveHead={liveHead} shownHour={viewHour} active={active && (shownDrill.breakdown === null || options.drill === 'popout')} grouping={grouping} groups={groups}
        onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} setOption={setOption} pickOwner={pickOwner} onPick={open} />
      : <StockpileTable data={data} hour={viewHour} widget={widget} live={stock.live} shownAt={stock.shownAt}
        at={(values: LiveValues<number>) => catalogue && stock.data ? clipped(stockpileView(catalogue, atReply(stock.data, values, viewHour, track.branch.id), viewHour), within) : undefined}
        included={included} count={count} narrow={narrowTableOf(options)}
        active={active && (shownDrill.breakdown === null || options.drill === 'popout')} viewport={viewport}
        runtime={runtime} track={track} onDiagnostics={onDiagnostics} onPick={open} />}
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
      returnFocus={options.view === 'time' ? '.stockpile-chart canvas' : '.stockpile-table-rows tr'}
      canFollow onClose={() => act({ type: 'close' })} main={main}
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={picked && selectedCountry && !actor.reason && at !== null && pinTime && pinCountry && choiceKey(picked, at) !== null ? () => {
        if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return;
        const pinned = pinStockpileDrill(widget, track, pinCountry.tag, picked, grouping, mode, at, pinTime, pinCountry.token);
        if (pinned) runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>{identityNotice}{detail}</DrillHost>}
    {menu.menu}
  </div>;
}

function OverTime({ data, catalogue, holders, included, tokens, viewport, options, runtime, gate, track, extent, playhead, liveHead, shownHour, active, grouping, groups, onMenu, setOption, pickOwner, onPick }: {
  data: EquipmentStockpileReply; catalogue: StockpileCatalogueReply | undefined; holders: readonly StockpileHolder[]; included: readonly string[];
  tokens: ChartTokens; viewport: number; options: Record<string, unknown>; runtime: WidgetProps['runtime']; gate: Gate; track: BoundTrack;
  extent: [number, number]; playhead: number | null; liveHead: LiveHead | undefined; shownHour: number; active: boolean; grouping: Grouping; groups: StockGroup[];
  onMenu: (hour: number, x: number, y: number) => void; setOption: WidgetProps['setOption'];
  pickOwner: object; onPick: (group: StockGroup, hour: number, mode: StockpileMode) => void;
}) {
  const shareMode = shareModeOf(data, grouping);
  const head = Math.floor(playhead ?? extent[1]);
  const shown = shownHour;
  const navigation = (data as PreparedStockpileReply).prepared;
  const drawable = useMemo(() => groups.filter(group => chartDrawable(data, group, grouping)), [data, groups, grouping]);
  const source = useMemo(() => storedChoices(stockChoices(groups, GROUP_NOUNS[grouping], group => drawable.includes(group))), [groups, grouping, drawable]);
  const chosenKey = CHOSEN_KEYS[grouping];
  const storedText = typeof options[chosenKey] === 'string' ? options[chosenKey] as string : '';
  const stored = useMemo(() => contentSet(storedText), [storedText]);
  const automatic = useMemo(() => automaticLines(data, groups, grouping, group => drawable.includes(group)), [data, groups, grouping, drawable]);
  const automaticTokens = useMemo(() => automatic.flatMap(key => {
    const group = groups.find(item => item.key === key);
    const choice = group ? choiceKey(group, head) : null;
    return choice == null ? [] : [setToken(choice)];
  }), [automatic, groups, head]);
  const chosenTokens = useMemo(() => oneChoicePerRow(chosenShown(source, stored, automaticTokens)), [source, stored, automaticTokens]);
  const chosen = useMemo(() => chosenTokens.map(token => choiceRow(contentKey(token))), [chosenTokens]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const selectedGroups = useMemo(() => groups.filter(group => chosen.includes(group.key)), [groups, chosen]);
  const requests = useMemo(() => selectedGroups.map(group => ({ key: group.key, group: stockpileGroupKey(group), measure: shareMode ? 'share' as const : 'stock' as const })),
    [selectedGroups, shareMode]);
  const incoming = useStockpileChart(runtime.client, gate, track, catalogue, holders, included, requests, head, active);
  const styles = useMemo(() => lineStyles(chosen.map(key => ({ key, preferred: groups.findIndex(group => group.key === key) })), tokens.categories), [chosen, groups, tokens]);
  const lines = useMemo(() => chosen.flatMap(key => {
    const group = groups.find(item => item.key === key);
    const style = styles.get(key);
    const summary = incoming.summaries.get(key);
    return group && style ? [{ group, color: style.color, dash: style.primary, summary,
      runs: clipToSpans(summary?.runs ?? [], [{ firstHour: extent[0], lastHour: extent[1] }]) }] : [];
  }), [chosen, groups, styles, incoming.summaries, extent[0], extent[1]]);
  const chosenScope = JSON.stringify(chosen);
  const choiceOwner = useMemo(() => ({ pickOwner, chosenScope, extent }), [pickOwner, chosenScope, extent[0], extent[1]]);
  const [selection, setSelection] = useState<{ owner: object; hour: number; keys: readonly string[] } | null>(null);
  const pending = selection?.owner === choiceOwner ? selection : null;
  const layers = useMemo<ChartLayer[]>(() => [
    valueRuleLayer(0),
    ...lines.flatMap((line): ChartLayer[] => line.summary ? [lineSummaryLayer({
      summary: () => line.summary,
      color: line.color,
      options: { dash: line.dash.length ? line.dash : undefined },
      extent: x => extentOfSummary(line.summary!, x[0], Math.min(x[1], until ?? x[1])),
    })] : []),
  ], [lines, until]);
  const endLabels = options.endLabels !== false;
  const endLabelsSpec = useMemo<DeclarativeEndLabels | undefined>(() => {
    if (!endLabels) return undefined;
    const items: DeclarativeEndLabels['items'][number][] = [];
    let layerIndex = 1;
    for (const line of lines) {
      if (!line.summary) continue;
      const last = line.summary.buckets.filter(bucket => bucket !== null).at(-1)?.lastHour
        ?? lineEndHour(line.summary.runs ?? [], extent[0], Math.min(until ?? extent[1], extent[1]));
      if (last != null) {
        items.push({
          id: line.group.key,
          layerIndex,
          color: line.color,
          muted: false,
          label: labelAt(line.group, grouping, last),
          format: { kind: 'static', requireFit: true },
        });
      }
      layerIndex++;
    }
    return { extentMax: extent[1], items };
  }, [endLabels, lines, extent[0], extent[1], until, grouping]);
  const actions = new Map(groups.map(group => {
    const choice = choiceKey(group, head, chosenTokens.map(contentKey));
    if (choice == null) return [group.key, { reason: SAVED_DESIGN_UNAVAILABLE }];
    const token = setToken(choice);
    const kept = chosenTokens.includes(token) ? stored.filter(item => item === token || choiceRow(contentKey(item)) !== group.key) : stored;
    return [group.key, toggleChosen(source, kept, chosenTokens, token, Number.MAX_SAFE_INTEGER)];
  }));
  const pick = (key: string) => {
    const action = actions.get(key);
    if (action && 'value' in action && setOption) setOption(chosenKey, action.value);
  };
  const caption = shareMode ? 'Surplus / shortage (% of need)' : 'In stock';
  return <>
    {incoming.error && <WidgetState kind="error" failure={incoming.error} onRetry={incoming.retry} />}
    {setOption && <div className="stockpile-caption-line"><span>Types shown</span>
      <ContentSetPicker label="Types shown over time" automatic="Main types"
        choices={drawable.map(group => ({ value: setToken(group.key), label: labelAt(group, grouping, head) }))}
        selected={chosen.map(setToken)} onToggle={value => pick(contentKey(value))} onReset={() => setOption(chosenKey, '')} /></div>}
    {!endLabels && <Legend label="Shown over time" items={lines.map(line => {
      if (shareMode) {
        const value = shareOf(balanceAt(data, line.group, shown));
        return { id: line.group.key, label: labelAt(line.group, grouping, head), color: line.color, value: value == null ? 'Not recorded' : formatPercent(value) };
      }
      const stock = stockAt(data, line.group, shown);
      return { id: line.group.key, label: labelAt(line.group, grouping, head), color: line.color, value: stock == null ? 'Not recorded' : formatAmount(stock, data.decimals) };
    })} />}
    <p className="stockpile-caption">{caption}</p>
    {pending && <div className="select-row">Choose drawn equipment on {dayLabel(pending.hour)} <Select autoFocus label="Drawn equipment" value="" onChange={key => {
      const group = lines.find(line => line.group.key === key)?.group;
      if (group && pending.keys.includes(group.key)) { onPick(group, pending.hour, shareMode ? 'balance' : 'stock'); setSelection(null); }
    }} options={[{ value: '', label: 'Choose equipment…', disabled: true },
      ...lines.filter(line => pending.keys.includes(line.group.key)).map(line => ({ value: line.group.key, label: labelAt(line.group, grouping, pending.hour) }))]} /></div>}
    {lines.length === 0 && <p className="stockpile-empty">{shareMode
      ? 'Need data unavailable.'
      : 'No stock history.'}</p>}
    <div className="stockpile-chart">
      <TimeChart label={`${shareMode ? 'Surplus as a share of need' : 'Equipment in stock'}: ${lines.map(line => labelAt(line.group, grouping, head)).join(', ')}`} extent={extent} playhead={playhead} liveHead={liveHead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        formatValue={shareMode ? 'percent' : undefined}
        endLabels={endLabelsSpec}
        padding={endLabels ? { right: END_LABEL_ROOM } : undefined} layers={layers} active={active}
        revision={`${data.branchId}:${chosen.join(' ')}:${grouping}:${viewport}:${endLabels}:${shareMode}`}
        keyHours={shareMode ? navigation?.logisticsReads ?? data.logisticsReads : navigation?.reads ?? data.reads}
        pickOwner={choiceOwner} onPick={(hour, context) => {
          const reached = incoming.data?.prepared?.hour === hour ? incoming.data : undefined;
          const known = new Set(lines.filter(line => runAt(line.summary?.runs ?? [], hour) || reached
            && (shareMode ? shareOf(balanceAt(reached, groupIn(reached, line.group), hour)) : stockAt(reached, groupIn(reached, line.group), hour)) !== null).map(line => line.group.key));
          const keys = pickSummaryLines(lines.flatMap(line => line.summary ? [{ key: line.group.key, summary: line.summary }] : []), hour, known, context);
          if (keys.length === 1) {
            const group = lines.find(line => line.group.key === keys[0])?.group;
            if (group) { setSelection(null); onPick(group, hour, shareMode ? 'balance' : 'stock'); }
          } else setSelection(keys.length ? { owner: choiceOwner, hour, keys } : null);
        }}
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) return null;
          const reached = incoming.data?.prepared?.hour === hour ? incoming.data : undefined;
          if (!reached) return null;
          const ai = shareMode && balanceMode(reached, hour) === 'ai';
          return overTimeTooltip(dayLabel(hour), lines.map(line => { const group = groupIn(reached, line.group); return ({ group: { ...group, label: labelAt(group, grouping, hour) }, color: line.color,
            stock: stockAt(reached, group, hour), daily: dailyAt(reached, group, hour),
            ...(shareMode ? { logistics: balanceAt(reached, group, hour) } : {}) }); }),
          data.decimals, ai ? AI_NOTE : undefined, viewport);
        }}
        onSummaryView={incoming.onSummaryView} onHover={incoming.onHover}
        onContextMenu={(hour, x, y) => onMenu(hour, x, y)} />
    </div>
  </>;
}

function clipped(reply: EquipmentStockpileReply, within: ComparisonProps['within']): EquipmentStockpileReply {
  return within ? { ...reply, stockHours: clipToSpans(reply.stockHours, within), lineHours: clipToSpans(reply.lineHours, within),
    balanceHours: clipToSpans(reply.balanceHours, within), aiHours: clipToSpans(reply.aiHours, within) } : reply;
}
