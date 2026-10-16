import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { Select } from '../../components/Select';
import { createPortal } from 'react-dom';
import ContentSetPicker from '../../components/ContentSetPicker';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel, drawLine, drawValueRule, placeEndLabels } from '../../charts/draw';
import { drawLineSummary, extentOfSummary, pickSummaryLines, summaryGeometry } from '../../charts/chartSummary';
import { jumpItem } from '../../charts/jump';
import { pickLines } from '../../charts/linePick';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import type { DrillAction } from '../../charts/drill/drillState';
import Legend from '../../charts/Legend';
import { lineStyles } from '../../charts/palette';
import { clipToSpans, extentOfAll, lineEndHour, runAt } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount, formatAxisPercent } from '../../charts/valueAxis';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { chosenShown, storedChoices, toggleChosen } from '../../dashboard/chosen';
import type { WidgetProps } from '../../dashboard/definitions';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { readDatedCountry } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useParityRows } from '../../dashboard/parityRows';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import { BREAKDOWN, OPTIONS } from './definition';
import { AI_NOTE, automaticLines, CHOSEN_KEYS, hasRecords, balanceAt, balanceMode, chartRuns, choiceKey, choiceRow, formatPercent, oneChoicePerRow, GROUP_NOUNS, labelAt,
  SAVED_DESIGN_UNAVAILABLE, overTimeTooltip, shareModeOf, shareOf, stockAt, stockChoices, stockGroups, dailyAt, chartDrawable } from './stockpileData';
import { usePreparedStockpileChart } from './usePreparedStockpileChart';
import { groupIn } from './preparedStockpileChart';
import type { Grouping, StockGroup } from './stockpileData';
import { useEquipmentStockpile, useStockpileCountry } from './useEquipmentStockpile';
import StockpileDrill from './StockpileDrill.tsx';
import StockpileTable from './StockpileTable';
import type { StockCount } from './stockpileView';
import { pinnedGrouping, pinStockpileDrill, stockpileDrillGroup } from './stockpileDrill';
import type { StockpileMode } from './stockpileDrill';
import { stockpileParityModel } from './stockpileParityModel';
import './equipmentStockpile.css';

const NO_WARNINGS: string[] = [];
/** Room for the names at the line ends, at the right of the plot. */
const END_LABEL_ROOM = 136;

/** One country's equipment stockpile: for a player-controlled country each equipment
 * type's surplus or shortage (the game's Logistics figure), as a table or over time; for
 * an AI-controlled one, for which the game computes none, what it holds. Rows name the
 * models held or being made and their output a day; the hover adds the models in stock. */
export default function EquipmentStockpileWidget(props: WidgetProps) {
  return props.widget.options?.show === BREAKDOWN.id ? <EquipmentStockpileCountry {...props} />
    : <CountryComparisons {...props} component={EquipmentStockpileCountry} />;
}

function EquipmentStockpileCountry({ widget, runtime, active, onDiagnostics, setOption, countrySlot, within }: ComparisonProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data: reply, numbers: legacyNumbers, error, retry } = useEquipmentStockpile(widget, runtime, active, within);
  const data = useMemo(() => reply && within ? { ...reply, stockHours: clipToSpans(reply.stockHours, within), lineHours: clipToSpans(reply.lineHours, within),
    balanceHours: clipToSpans(reply.balanceHours, within), aiHours: clipToSpans(reply.aiHours, within) } : reply, [reply, within]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  const menu = useContextMenu('Stockpile actions');
  // Types, each listing its models (a model's own row is its type's breakdown).
  const grouping = pinnedGrouping(widget.options);
  const count: StockCount = options.count === 'ic' ? 'ic' : 'units';
  const includedText = typeof options.included === 'string' ? options.included : '';
  // Stored as `setToken(key)`, so a key a setting cannot store can be included too.
  const included = useMemo(() => contentSet(includedText).map(contentKey), [includedText]);
  const anchor = useRef<HTMLDivElement>(null);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, grouping, included, options.show, options.stockpileCountry, options.partGroup, widget.time, within]);
  const numbers = runtime.client.preparedQuery ? undefined : legacyNumbers;
  const pickOwner = useMemo(() => ({ scope, client: runtime.client, numbers }), [scope, runtime.client, numbers]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [drillOwner, setDrillOwner] = useState<object | null>(null);
  const [selection, setSelection] = useState<{ owner: object; key: string; mode: StockpileMode } | null>(null);
  const shownDrill = drillOwner === pickOwner ? drill : CLOSED;
  const saved = options.show === BREAKDOWN.id;
  const groups = useMemo(() => data ? stockGroups(data, grouping, Infinity, included) : [], [data, grouping, included]);
  // The breakdown shows a type's models; the mode a pin records follows the row opened.
  const mode: StockpileMode = selection?.mode ?? 'stock';
  const clockHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || shownDrill.breakdown !== null && shownDrill.follow));
  const playbackHour = Math.floor(clockHour) + (track?.axisOffset ?? 0);
  const detailHead = shownDrill.breakdown !== null && shownDrill.follow ? playbackHour : playhead ?? head;
  const at = saved && shownDrill.breakdown === null ? head : drillHour(shownDrill, playbackHour) ?? head;
  const savedCountry = useMemo(() => saved && track ? readDatedCountry(options.stockpileCountry,
    track.source.metadata.campaignKey, track.branch.id, widget.countries) : null,
  [saved, options.stockpileCountry, widget.countries, track?.source.metadata.campaignKey, track?.branch.id]);
  const [countryPick, setCountryPick] = useState<{ owner: object; selection: DatedCountrySelection } | null>(null);
  const selectedCountry = saved ? savedCountry : countryPick?.owner === pickOwner ? countryPick.selection : null;
  const missingSavedDate = saved && shownDrill.breakdown === null && (widget.time.mode === 'pin' || widget.time.mode === 'range') && !window;
  const actor = useStockpileCountry(selectedCountry, runtime, track, active, at);
  const detailData = selectedCountry ? actor.data : undefined;
  useEffect(() => {
    const warnings = (saved || shownDrill.breakdown !== null ? detailData : data)?.warnings;
    onDiagnostics(warnings?.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, detailData, saved, shownDrill.breakdown, onDiagnostics]);
  const detailGroups = useMemo(() => detailData ? stockGroups(detailData, grouping, Infinity, included) : [], [detailData, grouping, included]);
  const picked = saved ? stockpileDrillGroup(detailGroups, String(options.partGroup ?? ''))
    : selection?.owner === pickOwner ? detailGroups.find(group => group.key === selection.key) : undefined;
  const headerTag = actor.spans ? countryDrillTagAt(actor.spans, at) : null;
  const headerReason = missingSavedDate ? 'The chosen moment is unavailable.' : !selectedCountry ? options.stockpileCountry
    ? 'Choose a country again.'
    : 'The original country choice was not saved. Select a stockpile breakdown again.'
    : actor.reason ?? (headerTag ? null : 'Country unavailable at this time.');
  const header = useCountryLabelsAt(runtime.client, track, saved && countrySlot && headerTag && !headerReason ? [{ tag: headerTag, hour: at }] : [], active);
  const pinnedHeader = saved && countrySlot ? createPortal(<DatedCountryChip hour={missingSavedDate ? null : at} reason={headerReason}
    label={headerTag && !headerReason ? header.labelAt(at)(headerTag) : null} />, countrySlot) : null;
  const detailPickOwner = useMemo(() => ({ pickOwner, selectedCountry }), [pickOwner, selectedCountry]);
  const showingDetail = saved || shownDrill.breakdown !== null;
  const exactData = showingDetail ? detailData : data;
  const exactGroups = showingDetail ? picked ? [picked] : [] : groups;
  const exactHour = showingDetail ? at : head;
  const parityLabel = showingDetail ? 'Equipment stockpile breakdown exact values' : 'Equipment stockpile exact values';
  useParityRows(anchor, () => stockpileParityModel(exactData, exactGroups, exactHour, parityLabel),
    { ready: () => stockpileParityModel(exactData, exactGroups, exactHour, parityLabel).complete });
  const ownerRef = useRef({ owner: pickOwner, active });
  ownerRef.current = { owner: pickOwner, active };
  useEffect(() => {
    ownerRef.current.active = active;
    return () => { ownerRef.current.active = false; };
  }, [active, pickOwner]);
  const act = (action: DrillAction) => { if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return; setDrillOwner(pickOwner); dispatch(action); };
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(null); setSelection(null); setCountryPick(null); }, [pickOwner]);

  if (!track || !tag) return <>{pinnedHeader}<WidgetState kind="empty" icon="grid" message="Load a campaign and choose a country to see its equipment stockpile." /></>;
  if (!saved && shownDrill.breakdown === null && playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="This time is outside the recording." /></>;
  if (shownDrill.breakdown === null && (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Choose another time in widget settings." /></>;
  if (!saved && shownDrill.breakdown === null && error) return <>{pinnedHeader}<WidgetState kind="error" detail={error} onRetry={retry} /></>;
  if (!saved && shownDrill.breakdown === null && !data) return <>{pinnedHeader}<WidgetState kind="loading" label="Loading the stockpile…" /></>;
  // Whatever was recorded shows (production without a stockpile read, say); each value
  // not recorded says so.
  if (!saved && shownDrill.breakdown === null && data && !hasRecords(data)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Stockpile data unavailable for this country." /></>;

  const jump = (hour: number) => [jumpItem(runtime, track.id, hour)];
  const open = (group: StockGroup, hour: number, mode: StockpileMode) => {
    if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return;
    setSelection({ owner: pickOwner, key: group.key, mode });
    setCountryPick({ owner: pickOwner, selection: { campaign: track.source.metadata.campaignKey, branch: track.branch.id,
      tag, originOffset: Math.floor(hour) - track.branch.firstHour } });
    const playing = runtime.clock.getSnapshot().playing;
    act({ type: 'open', breakdown: BREAKDOWN.id, hour, playing, playhead: playhead ?? (playing ? playbackHour : null) });
  };
  const main = data ? <div className="equipment-stockpile">
    {options.view === 'time'
      ? <OverTime data={data} tokens={tokens} viewport={viewport} options={options} runtime={runtime} track={track} extent={extent} playhead={playhead}
        within={within} active={active && (shownDrill.breakdown === null || options.drill === 'popout')} grouping={grouping} groups={groups}
        onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} setOption={setOption} pickOwner={pickOwner} onPick={open} />
      : <StockpileTable data={data} hour={head} included={included} count={count} setCount={setOption ? value => setOption('count', value) : undefined}
        active={active && (shownDrill.breakdown === null || options.drill === 'popout')} viewport={viewport}
        runtime={runtime} track={track} onDiagnostics={onDiagnostics} onPick={open} />}
  </div> : null;
  const detail = !selectedCountry ? <WidgetState kind="unavailable" message={headerReason!} />
    : actor.error ? <WidgetState kind="error" detail={actor.error} onRetry={actor.retry} />
      : actor.reason ? <WidgetState kind="unavailable" message={actor.reason} />
        : picked && detailData ? <StockpileDrill data={detailData} group={picked} hour={at} client={runtime.client} track={track} aliases={actor.spans}
    extent={extent} playhead={detailHead} playback="cursor" afterPlayhead selectedHour={at} branchStart={track.branch.firstHour}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active && (saved || shownDrill.breakdown !== null)}
    pickOwner={detailPickOwner} onPick={hour => act(saved
      ? { type: 'open', breakdown: BREAKDOWN.id, hour, playing: false, playhead: null } : { type: 'pick', hour })}
    onContextMenu={(hour, x, y) => menu.open(x, y, jump(hour))} />
    : <p className="drill-empty">The chosen equipment or saved design is not defined by this campaign.</p>;
  return <div ref={anchor} className="stockpile-drill-host">
    {saved ? <div className="drill-body">
      {pinnedHeader}
      {(shownDrill.breakdown === null ? widget.time.mode !== 'follow' : !shownDrill.follow) && <button type="button" className="button small quiet"
        onClick={() => act({ type: 'open', breakdown: BREAKDOWN.id, hour: playbackHour, playing: true, playhead: playbackHour })}>Follow playback</button>}{detail}
    </div> : <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={shownDrill.breakdown !== null}
      title={picked ? labelAt(picked, grouping, at) : BREAKDOWN.label} day={dayLabel(at)} following={shownDrill.follow}
      returnFocus={options.view === 'time' ? '.stockpile-chart canvas' : '.stockpile-table-rows tr'}
      canFollow onClose={() => act({ type: 'close' })} onFollow={() => act({ type: 'follow' })} main={main}
      onPin={picked && selectedCountry && !actor.reason && choiceKey(picked, at) !== null ? () => {
        if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return;
        const pinned = pinStockpileDrill(widget, track, selectedCountry.tag, picked, grouping, mode, at, shownDrill.follow, selectedCountry);
        if (pinned) runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>{detail}</DrillHost>}
    {menu.menu}
  </div>;
}

/** The chosen groups over time, each in its own palette colour, with the zero line
 * emphasised and names at the line ends: the surplus as a share
 * of need where the game computed it, else what was in stock. */
function OverTime({ data, within, tokens, viewport, options, runtime, track, extent, playhead, active, grouping, groups, onMenu, setOption, pickOwner, onPick }: {
  data: EquipmentStockpileReply; within?: readonly import('../../charts/series').Span[] | null; tokens: ChartTokens; viewport: number; options: Record<string, unknown>; runtime: WidgetProps['runtime']; track: BoundTrack;
  extent: [number, number]; playhead: number | null; active: boolean; grouping: Grouping; groups: StockGroup[];
  onMenu: (hour: number, x: number, y: number) => void; setOption: WidgetProps['setOption'];
  pickOwner: object; onPick: (group: StockGroup, hour: number, mode: StockpileMode) => void;
}) {
  // The share chart wherever the balance is recorded; variants have none of their own.
  const shareMode = shareModeOf(data, grouping);
  const head = Math.floor(playhead ?? extent[1]);
  const series = useMemo(() => new Map(groups.map(group => [group.key, chartRuns(data, group, grouping)])), [data, groups, grouping]);
  const drawable = useMemo(() => groups.filter(group => chartDrawable(data, group, grouping)), [data, groups, grouping]);
  // Choices are stored as `setToken(choiceKey)` (the kit's `storedChoices`), so a key a
  // setting cannot store can be chosen too, and a variant is stored with its design, so
  // another design a later branch gives its handle is never drawn for it. Only what can
  // be drawn counts; the chart works on the rows' keys.
  const source = useMemo(() => storedChoices(stockChoices(groups, GROUP_NOUNS[grouping], group => drawable.includes(group))), [groups, grouping, drawable]);
  // Each grouping keeps its own chosen lines.
  const chosenKey = CHOSEN_KEYS[grouping];
  const storedText = typeof options[chosenKey] === 'string' ? options[chosenKey] as string : '';
  const stored = useMemo(() => contentSet(storedText), [storedText]);
  // Filtered to what can be drawn before the top types are taken, so none is lost to a
  // type with no share.
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
  const incoming = usePreparedStockpileChart(runtime.client, track, data.tag, selectedGroups, grouping, head, active, [shareMode ? 'reads-2' : 'reads-0'], within ?? undefined);
  // Each line keeps its colour from the game's order where it is free.
  const styles = useMemo(() => lineStyles(chosen.map(key => ({ key, preferred: groups.findIndex(group => group.key === key) })), tokens.categories), [chosen, groups, tokens]);
  // Only the hours shown (a date range, or the branch): an axis dragged past a range
  // draws, fits and describes nothing beyond it.
  const lines = useMemo(() => chosen.flatMap(key => {
    const group = groups.find(item => item.key === key);
    const style = styles.get(key);
    const summary = incoming.summaries.get(key);
    return group && style ? [{ group: groupIn(incoming.displayData ?? data, group), color: style.color, dash: style.primary, summary,
      runs: clipToSpans(incoming.prepared ? summary?.runs ?? [] : series.get(key) ?? [], [{ firstHour: extent[0], lastHour: extent[1] }]) }] : [];
  }), [chosen, groups, styles, series, incoming.summaries, incoming.displayData, incoming.prepared, data, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const chosenScope = JSON.stringify(chosen);
  const choiceOwner = useMemo(() => ({ pickOwner, chosenScope, extent }), [pickOwner, chosenScope, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const [selection, setSelection] = useState<{ owner: object; hour: number; keys: readonly string[] } | null>(null);
  const pending = selection?.owner === choiceOwner ? selection : null;
  const layers = useMemo<ChartLayer[]>(() => [
    { draw: (ctx, frame, chartTokens) => drawValueRule(ctx, frame, chartTokens, 0) },
    ...lines.map((line): ChartLayer => ({
      draw: (ctx, frame, chartTokens, state) => line.summary
        ? drawLineSummary(ctx, frame, chartTokens, line.summary, line.color, false, state.playhead, { dash: line.dash.length ? line.dash : undefined })
        : drawLine(ctx, frame, chartTokens, line.runs, line.color, { until: state.playhead ?? undefined, dash: line.dash.length ? line.dash : undefined }),
      extent: x => line.summary ? extentOfSummary(line.summary, x[0], Math.min(x[1], until ?? x[1])) : extentOfAll([line.runs], x[0], Math.min(x[1], until ?? x[1])),
    })),
  ], [lines, until]);
  const actions = new Map(groups.map(group => {
    const choice = choiceKey(group, head, chosenTokens.map(contentKey));
    if (choice == null) return [group.key, { reason: SAVED_DESIGN_UNAVAILABLE }];
    // Deselecting a row drops every saved choice of it, not only the one shown.
    const token = setToken(choice);
    const kept = chosenTokens.includes(token) ? stored.filter(item => item === token || choiceRow(contentKey(item)) !== group.key) : stored;
    return [group.key, toggleChosen(source, kept, chosenTokens, token, Number.MAX_SAFE_INTEGER)];
  }));
  const pick = (key: string) => {
    const action = actions.get(key);
    if (action && 'value' in action && setOption) setOption(chosenKey, action.value);
  };
  const endLabels = options.endLabels !== false;
  const caption = shareMode ? 'Surplus / shortage (% of need)' : 'In stock';
  return <>
    {incoming.error && <WidgetState kind="error" detail={incoming.error} onRetry={incoming.retry} />}
    {setOption && <div className="stockpile-caption-line"><span>Types shown</span>
      <ContentSetPicker label="Types shown over time" automatic="Main types"
        choices={drawable.map(group => ({ value: setToken(group.key), label: labelAt(group, grouping, head) }))}
        selected={chosen.map(setToken)} onToggle={value => pick(contentKey(value))} onReset={() => setOption(chosenKey, '')} /></div>}
    {!endLabels && <Legend label="Shown over time" items={lines.map(line => {
      if (shareMode) {
        const value = shareOf(balanceAt(data, line.group, head));
        return { id: line.group.key, label: labelAt(line.group, grouping, head), color: line.color, value: value == null ? 'Not recorded' : formatPercent(value) };
      }
      const stock = stockAt(data, line.group, head);
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
      <TimeChart label={`${shareMode ? 'Surplus as a share of need' : 'Equipment in stock'}: ${lines.map(line => labelAt(line.group, grouping, head)).join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        // The kit's axis rule: an auto-fitted axis holds zero, and log scale applies only
        // where every share shown is positive, as in every time chart.
        formatValue={shareMode ? formatAxisPercent : undefined}
        padding={endLabels ? { right: END_LABEL_ROOM } : undefined} layers={layers} active={active}
        revision={`${head}:${data.branchId}:${chosen.join(' ')}:${grouping}:${viewport}:${endLabels}:${shareMode}`}
        // The keyboard steps through the reads; the kit adds the start of a range or zoom
        // that begins between two reads, so the value carried into it is reachable too.
        keyHours={shareMode ? data.logisticsReads : data.reads}
        pickOwner={choiceOwner} onPick={(hour, context) => {
          const reached = incoming.data?.prepared?.from === hour ? incoming.data : undefined;
          const known = new Set(lines.filter(line => runAt(line.summary?.runs ?? [], hour) || reached
            && (shareMode ? shareOf(balanceAt(reached, groupIn(reached, line.group), hour)) : stockAt(reached, groupIn(reached, line.group), hour)) !== null).map(line => line.group.key));
          const keys = incoming.prepared ? pickSummaryLines(lines.flatMap(line => line.summary ? [{ key: line.group.key, summary: line.summary }] : []), hour, known, context)
            : pickLines(lines.map(line => ({ key: line.group.key, runs: line.runs })), hour, context);
          if (keys.length === 1) {
            const group = lines.find(line => line.group.key === keys[0])?.group;
            if (group) { setSelection(null); onPick(group, hour, shareMode ? 'balance' : 'stock'); }
          } else setSelection(keys.length ? { owner: choiceOwner, hour, keys } : null);
        }}
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) return null;
          const reached = incoming.prepared ? incoming.data?.prepared?.from === hour ? incoming.data : undefined : data;
          if (!reached) return null;
          const ai = shareMode && balanceMode(reached, hour) === 'ai';
          // A variant is named as recorded at the hovered hour.
          return overTimeTooltip(dayLabel(hour), lines.map(line => { const group = groupIn(reached, line.group); return ({ group: { ...group, label: labelAt(group, grouping, hour) }, color: line.color,
            stock: stockAt(reached, group, hour), daily: dailyAt(reached, group, hour),
            ...(shareMode ? { logistics: balanceAt(reached, group, hour) } : {}) }); }),
          data.decimals, ai ? AI_NOTE : undefined, viewport);
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!endLabels) return;
          const ends = lines.flatMap(line => {
            if (line.summary) {
              const end = summaryGeometry(frame, line.summary, false, until).end;
              const last = line.summary.buckets.filter(bucket => bucket !== null).at(-1)?.lastHour
                ?? lineEndHour(line.summary.runs ?? [], frame.x[0], Math.min(until ?? extent[1], frame.x[1], extent[1]));
              return end && last != null ? [{ id: line.group.key, color: line.color, muted: false, y: end.y, text: labelAt(line.group, grouping, last) }] : [];
            }
            // At the end of the line drawn, never past the recorded hours.
            const last = lineEndHour(line.runs, frame.x[0], Math.min(until ?? extent[1], frame.x[1], extent[1]));
            const run = last == null ? null : runAt(line.runs, last);
            return run && last != null ? [{ id: line.group.key, color: line.color, muted: false, y: frame.toY(run.value), text: labelAt(line.group, grouping, last) }] : [];
          });
          placeEndLabels(ctx, frame, chartTokens, ends);
        }}
        onSummaryView={incoming.onSummaryView} onHover={incoming.onHover} onNavigateHour={incoming.onNavigateHour}
        keyboardHour={incoming.keyboardHour} keyboardRevision={incoming.keyboardRevision}
        onContextMenu={(hour, x, y) => onMenu(hour, x, y)} />
    </div>
  </>;
}
