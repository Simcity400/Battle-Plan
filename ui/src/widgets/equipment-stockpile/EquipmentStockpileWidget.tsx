import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { Select } from '../../components/Select';
import type { CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { ChipSet } from '../../components/ChipSet';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { barEndLabel, divergingBar } from '../../charts/divergingBar';
import { dayLabel, drawLine, drawValueRule, placeEndLabels } from '../../charts/draw';
import { easeTau } from '../../charts/easing';
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
import { tooltipText } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { movingRowClass, useRowMotion } from '../../charts/useRowMotion';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { formatAmount, formatAxisPercent, formatSigned } from '../../charts/valueAxis';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { formatDateTime } from '../../dashboard/calendar';
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
import { readAt } from '../../dashboard/reads';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import { BREAKDOWN, OPTIONS } from './definition';
import { AI_NOTE, automaticLines, CHOSEN_KEYS, hasRecords, balanceAt, balanceMode, barExtent, chartRuns, choiceKey, choiceRow, formatPercent, oneChoicePerRow, GROUP_NOUNS, groupLabel, groupTooltip, labelAt,
  NO_DESIGN, orderGroups, otherDesigns, overTimeTooltip, shareModeOf, shareOf, shareScale, stockAt, stockChoices, stockGroups, typesComplete, undrawableReason, weeklyAt, columnVars } from './stockpileData';
import type { Grouping, RowOrder, StockGroup } from './stockpileData';
import { useEquipmentStockpile, useStockpileCountry } from './useEquipmentStockpile';
import { useStockpileArtwork } from './useStockpileArtwork';
import StockpileDrill from './StockpileDrill.tsx';
import { pinStockpileDrill, stockpileDrillGroup } from './stockpileDrill';
import type { StockpileMode } from './stockpileDrill';
import './equipmentStockpile.css';

const NO_WARNINGS: string[] = [];
/** How often each record is read, for the note under a value read earlier. */
/** One bar row's height; rows slide by it when the order changes. */
const ROW_HEIGHT = 34;
/** Room for the names at the line ends, at the right of the plot. */
const END_LABEL_ROOM = 136;

/** One country's equipment stockpile: for a player-controlled
 * country each equipment type's surplus or shortage as a share of what is needed, as
 * bars or over time; for an AI-controlled one, for which the game computes none, what
 * it holds. The hover adds produced per week and the variants in stock. */
export default function EquipmentStockpileWidget(props: WidgetProps) {
  return props.widget.options?.show === BREAKDOWN.id ? <EquipmentStockpileCountry {...props} />
    : <CountryComparisons {...props} component={EquipmentStockpileCountry} />;
}

function EquipmentStockpileCountry({ widget, runtime, active, onDiagnostics, setOption, countrySlot, within }: ComparisonProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data: reply, numbers, contentPending, error, retry } = useEquipmentStockpile(widget, runtime, active);
  const data = useMemo(() => reply && within ? { ...reply, stockHours: clipToSpans(reply.stockHours, within), lineHours: clipToSpans(reply.lineHours, within),
    balanceHours: clipToSpans(reply.balanceHours, within), aiHours: clipToSpans(reply.aiHours, within) } : reply, [reply, within]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  const menu = useContextMenu('Stockpile actions');
  const grouping = (options.grouping as Grouping) ?? 'type';
  const includedText = typeof options.included === 'string' ? options.included : '';
  // Stored as `setToken(key)`, so a key a setting cannot store can be included too.
  const included = useMemo(() => contentSet(includedText).map(contentKey), [includedText]);
  const anchor = useRef<HTMLDivElement>(null);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, grouping, included, options.show, options.stockpileCountry, options.partGroup, options.partMode, widget.time, within]);
  const pickOwner = useMemo(() => ({ scope, client: runtime.client, numbers }), [scope, runtime.client, numbers]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [drillOwner, setDrillOwner] = useState<object | null>(null);
  const [selection, setSelection] = useState<{ owner: object; key: string; mode: StockpileMode } | null>(null);
  const shownDrill = drillOwner === pickOwner ? drill : CLOSED;
  const saved = options.show === BREAKDOWN.id;
  const groups = useMemo(() => data ? stockGroups(data, grouping, Infinity, included) : [], [data, grouping, included]);
  const mode = saved ? options.partMode as StockpileMode : selection?.mode ?? 'balance';
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
  const actor = useStockpileCountry(selectedCountry, runtime, track, active);
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
    ? 'This country selection belongs to another country, campaign or branch, or is malformed.'
    : 'The original country choice was not saved. Select a stockpile breakdown again.'
    : actor.reason ?? (headerTag ? null : 'The selected native country is not recorded at this moment.');
  const header = useCountryLabelsAt(runtime.client, track, saved && countrySlot && headerTag && !headerReason ? [{ tag: headerTag, hour: at }] : [], active);
  const pinnedHeader = saved && countrySlot ? createPortal(<DatedCountryChip hour={missingSavedDate ? null : at} reason={headerReason}
    label={headerTag && !headerReason ? header.labelAt(at)(headerTag) : null} />, countrySlot) : null;
  const detailPickOwner = useMemo(() => ({ pickOwner, selectedCountry, numbers: actor.numbers }), [pickOwner, selectedCountry, actor.numbers]);
  const ownerRef = useRef({ owner: pickOwner, active });
  ownerRef.current = { owner: pickOwner, active };
  useEffect(() => {
    ownerRef.current.active = active;
    return () => { ownerRef.current.active = false; };
  }, [active, pickOwner]);
  const act = (action: DrillAction) => { if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return; setDrillOwner(pickOwner); dispatch(action); };
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(null); setSelection(null); setCountryPick(null); }, [pickOwner]);

  if (!track || !tag) return <>{pinnedHeader}<WidgetState kind="empty" icon="grid" message="Load a campaign and choose a country to see its equipment stockpile." /></>;
  if (!saved && shownDrill.breakdown === null && playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." /></>;
  if (shownDrill.breakdown === null && (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <>{pinnedHeader}<WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." /></>;
  if (!saved && shownDrill.breakdown === null && error) return <>{pinnedHeader}<WidgetState kind="error" message={error} onRetry={retry} /></>;
  if (!saved && shownDrill.breakdown === null && !data) return <>{pinnedHeader}<WidgetState kind="loading" label="Loading the stockpile…" /></>;
  // Whatever was recorded shows (production without a stockpile read, say); each value
  // not recorded says so.
  if (!saved && shownDrill.breakdown === null && data && !hasRecords(data)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="No stockpile, production or logistics was recorded for this country in this branch." /></>;

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
        active={active && (shownDrill.breakdown === null || options.drill === 'popout')} grouping={grouping} groups={groups}
        onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} setOption={setOption} pickOwner={pickOwner} onPick={open} />
      : <StockBars data={data} tokens={tokens} hour={head} grouping={grouping} included={included} allGroups={groups} order={options.order as RowOrder}
        scale={options.scale === 'moment' ? 'moment' : 'campaign'} owner={`equipment-stockpile:${widget.id}:${tag}`}
        active={active && (shownDrill.breakdown === null || options.drill === 'popout')} viewport={viewport}
        runtime={runtime} track={track} onDiagnostics={onDiagnostics} onPick={open}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} />}
  </div> : null;
  const detail = !selectedCountry ? <WidgetState kind="unavailable" message={headerReason!} />
    : actor.error ? <WidgetState kind="error" message={actor.error} onRetry={actor.retry} />
      : actor.reason ? <WidgetState kind="unavailable" message={actor.reason} />
        : picked && detailData ? <StockpileDrill data={detailData} group={picked} mode={mode} percentage={mode === 'balance'} hour={at}
    extent={extent} playhead={detailHead} playback="cursor" afterPlayhead selectedHour={at} branchStart={track.branch.firstHour}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active && (saved || shownDrill.breakdown !== null)}
    pickOwner={detailPickOwner} onPick={hour => act(saved
      ? { type: 'open', breakdown: BREAKDOWN.id, hour, playing: false, playhead: null } : { type: 'pick', hour })}
    onContextMenu={(hour, x, y) => menu.open(x, y, jump(hour))} />
    : <p className="drill-empty">The chosen equipment or saved design is not defined by this campaign.</p>;
  return <div ref={anchor} className="stockpile-drill-host">
    {contentPending && <p className="stockpile-caption">Equipment names are updating.</p>}
    {saved ? <div className="drill-body">
      {pinnedHeader}
      {(shownDrill.breakdown === null ? widget.time.mode !== 'follow' : !shownDrill.follow) && <button type="button" className="button small quiet"
        onClick={() => act({ type: 'open', breakdown: BREAKDOWN.id, hour: playbackHour, playing: true, playhead: playbackHour })}>Follow playback</button>}{detail}
    </div> : <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={shownDrill.breakdown !== null}
      title={picked ? labelAt(picked, grouping, at) : BREAKDOWN.label} day={dayLabel(at)} following={shownDrill.follow}
      returnFocus={options.view === 'time' ? '.stockpile-chart canvas' : picked ? `[data-stockpile-key="${setToken(picked.key)}"]` : undefined}
      canFollow onClose={() => act({ type: 'close' })} onFollow={() => act({ type: 'follow' })} main={main}
      onPin={picked && selectedCountry && !actor.reason && choiceKey(picked, at) !== null ? () => {
        if (!ownerRef.current.active || ownerRef.current.owner !== pickOwner) return;
        const pinned = pinStockpileDrill(widget, track, selectedCountry.tag, picked, grouping, mode, at, shownDrill.follow, selectedCountry);
        if (pinned) runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>{detail}</DrillHost>}
    {menu.menu}
  </div>;
}

const percent = (fraction: number) => `${Math.round(fraction * 100000) / 1000}%`;

/** One row per group, sliding to a new order. For a
 * player-controlled country each row's surplus (right) or shortage (left, red) as a
 * share of what is needed, all on one scale, with the exact amount at the bar's end;
 * else a bar from zero to what is in stock. Rows are focusable: hover or focus shows
 * the group's tooltip, right-click or the menu key jumps to the read shown. */
function StockBars({ data, tokens, hour, grouping, included, allGroups, order, scale, owner, active, viewport, hoursPerSecond, onMenu, runtime, track, onDiagnostics, onPick }: {
  data: EquipmentStockpileReply; tokens: ChartTokens; hour: number; grouping: Grouping; included: readonly string[]; order: RowOrder;
  allGroups: StockGroup[];
  scale: 'campaign' | 'moment'; owner: string; active: boolean; viewport: number; hoursPerSecond: number; onMenu: (hour: number, x: number, y: number) => void;
  runtime: WidgetProps['runtime']; track: BoundTrack; onDiagnostics: WidgetProps['onDiagnostics'];
  onPick: (group: StockGroup, hour: number, mode: StockpileMode) => void;
}) {
  const groups = useMemo(() => stockGroups(data, grouping, hour, included), [data, grouping, hour, included]);
  const mode = grouping === 'variant' ? 'variant' : balanceMode(data, hour);
  const artCoverage = mode === 'player' ? data.balanceHours : data.stockHours;
  const artHour = runAt(artCoverage, hour) ? readAt(mode === 'player' ? data.logisticsReads : data.reads, hour) : null;
  const variants = useMemo(() => grouping === 'variant' ? groups.flatMap(group => group.variants.map(variant => variant.id)) : [], [groups, grouping]);
  const artwork = useStockpileArtwork(runtime, track, data.tag, artHour, variants, active);
  const art = useMemo(() => new Map(artwork?.data?.entries.map(entry => [entry.key, entry]) ?? []), [artwork]);
  const diagnostics = useMemo(() => [...data.warnings, ...(artwork?.data?.warnings ?? []), ...(artwork?.error ? [artwork.error] : [])], [data, artwork]);
  useDiagnostics(onDiagnostics, diagnostics);
  // The campaign scales depend on the groups, not on the names shown at an hour.
  const campaignStock = useMemo(() => barExtent(data, allGroups, null), [data, allGroups]);
  const campaignShare = useMemo(() => shareScale(data, allGroups, null), [data, allGroups]);
  const diverging = mode === 'player';
  const stockExtent = scale === 'moment' ? barExtent(data, groups, hour) : campaignStock;
  const share = scale === 'moment' ? shareScale(data, groups, hour) : campaignShare;
  const read = diverging ? readAt(data.logisticsReads, hour) : readAt(data.reads, hour);
  const jumpHour = read ?? hour;
  // The hour drawn last, so a bar glides only within one stretch of recording.
  const drawnHour = useRef(hour);
  useEffect(() => { drawnHour.current = hour; }, [hour]);
  const spans = diverging ? data.balanceHours : data.stockHours;
  const glides = runAt(spans, drawnHour.current) === runAt(spans, hour);
  const items = useMemo(() => orderGroups(data, groups, hour, order).map(group => ({ key: group.key, item: group })), [data, groups, hour, order]);
  const rows = useRowMotion(items);
  const byKey = useMemo(() => new Map(groups.map(group => [group.key, group])), [groups]);
  // Every group's tooltip and spoken text, built once per hour (and reply, grouping
  // and window); moving the pointer reuses them.
  const described = useMemo(() => new Map(groups.map(group => {
    const picture = art.get(grouping === 'variant' ? `variant:${group.variants[0]?.id}` : `type:${group.key}`);
    const note = [picture && artHour !== null
      ? `Picture: ${picture.name ?? `variant ${picture.variant}`}, read ${formatDateTime(artHour)}.${picture.reason ? ` ${picture.reason}.` : ''}`
      : grouping === 'domain' ? 'An arm combines equipment types and has no single native best variant.'
        : artwork?.data ? 'A native picture is unavailable at this value read.' : undefined].filter(Boolean).join(' ');
    const content = groupTooltip(data, group, grouping, hour, { title: group.label, subtitle: dayLabel(hour), note }, viewport);
    return [group.key, { content, text: tooltipText(content) }] as const;
  })), [data, groups, grouping, hour, viewport, art, artHour, artwork?.data]);
  const content = useCallback((key: string) => described.get(key)?.content ?? null, [described]);
  const rowTips = useRowTooltips<string>({ owner, active, content, revision: `${hour}:${data.branchId}:${grouping}`, onMenu: (_, x, y) => onMenu(jumpHour, x, y) });
  const glide = Math.round(3 * easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion));
  // Rows are placed apart so they can slide, so every row and the shortage/surplus header
  // share one name column, as wide as the longest name over the campaign (so it never
  // jumps), set on their common parent.
  const labels = useMemo(() => allGroups.map(group => group.label), [allGroups]);
  const amounts = groups.map(group => {
    const value = diverging ? balanceAt(data, group, hour)?.balance ?? null : stockAt(data, group, hour);
    return value == null ? 'Not recorded' : diverging ? formatSigned(value, data.decimals) : formatAmount(value, data.decimals);
  });
  const columns = columnVars(labels, amounts) as CSSProperties;
  const style = { '--stock-glide': `${glides ? glide : 0}ms`, '--row-glide': `${glide * 2}ms`, height: items.length * ROW_HEIGHT } as CSSProperties;
  const caption = diverging ? `Surplus or shortage as a share of what is needed${read != null ? ` · read ${formatDateTime(read)}` : ''}`
    : `In stock${read != null ? ` · read ${formatDateTime(read)}` : ''}${mode === 'ai' ? ` · ${AI_NOTE}` : mode === 'unknown' ? ' · the surplus or shortage was not recorded at this time'
      : ' · the game counts surplus and need by equipment type, not by variant'}`;

  const divergingRow = (group: StockGroup) => {
    const logistics = balanceAt(data, group, hour);
    // An arm a type of unknown arm could belong to is not known; otherwise not recorded.
    if (!logistics) return <span className="stockpile-missing">{group.types.length && !typesComplete(data, group, hour) ? 'Not known' : 'Not recorded'}</span>;
    const value = shareOf(logistics);
    // A row whose need is not derived (or is none) has no share: only its exact amount shows.
    const bar = value != null && share > 0 ? divergingBar(value, share, share) : { zero: 0.5, start: 0.5, width: 0 };
    const label = barEndLabel(bar, logistics.balance < 0n ? -1 : 1);
    const negative = logistics.balance < 0n;
    return <span className="stockpile-track is-diverging" aria-hidden="true">
      <i className="stockpile-zero" style={{ left: '50%' }} />
      {value != null && <i className={`stockpile-bar${negative ? ' is-negative' : ''}`} style={{ left: percent(bar.start), width: percent(bar.width) }} />}
      <b className={`stockpile-end is-${label.side}${negative ? ' is-negative' : ''}${value == null ? ' is-unshared' : ''}`} style={{ left: percent(label.at) }}>
        {formatSigned(logistics.balance, data.decimals)}
      </b>
    </span>;
  };
  const stockRow = (group: StockGroup) => {
    const stock = stockAt(data, group, hour);
    if (stock == null) return <span className="stockpile-missing">Not recorded</span>;
    const bar = divergingBar(Number(stock), Number(stockExtent.negative), Number(stockExtent.positive));
    return <>
      <span className="stockpile-track" aria-hidden="true">
        {stockExtent.negative > 0n && <i className="stockpile-zero" style={{ left: percent(bar.zero) }} />}
        <i className={`stockpile-bar${stock < 0n ? ' is-negative' : ''}`} style={{ left: percent(bar.start), width: percent(bar.width) }} />
      </span>
      <b className={stock < 0n ? 'is-negative' : undefined}>{formatAmount(stock, data.decimals)}</b>
    </>;
  };
  return <div className="stockpile-rows" style={columns}>
    <p className="stockpile-caption">{caption}</p>
    {diverging && <p className="stockpile-sides" aria-hidden="true"><span>shortage</span><span>surplus</span></p>}
    {items.length === 0 ? <p className="stockpile-empty">Nothing of the included types was recorded.</p>
      : <ol className={`stockpile-bars${diverging ? ' is-diverging' : ''}`} style={style} aria-label={`${diverging ? 'Surplus or shortage' : 'Equipment in stock'} on ${dayLabel(hour)}`}>
        {rows.map(row => {
          const group = byKey.get(row.key) ?? row.item;
          const picture = art.get(grouping === 'variant' ? `variant:${group.variants[0]?.id}` : `type:${group.key}`);
          const pick = () => { const current = byKey.get(row.key); if (current) onPick(current, hour, diverging ? 'balance' : 'stock'); };
          return <li key={row.key} className={movingRowClass(row)} style={{ transform: `translateY(${row.slot * ROW_HEIGHT}px)`, height: ROW_HEIGHT }}
            aria-label={groupLabel(data, group, hour)} data-stockpile-key={setToken(group.key)} {...rowTips.rowProps(row.key)}
            onClick={pick}
            onKeyDown={event => {
              rowTips.rowProps(row.key).onKeyDown?.(event);
              if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); pick(); }
            }}>
            {/* The full tooltip, even where the window trims it (the tooltip layer is not read out). */}
            <span id={rowTips.descriptionId(row.key)} className="sr-only">{described.get(row.key)?.text}</span>
            <span className="stockpile-name">
              {picture?.icon
                ? <img className="stockpile-art" src={picture.icon} alt="" />
                : <span className="stockpile-art" aria-hidden="true" />}
              <span>{group.label}{grouping === 'type' && picture?.name && <small>{picture.name}</small>}</span>
            </span>
            {diverging ? divergingRow(group) : stockRow(group)}
          </li>;
        })}
      </ol>}
  </div>;
}

/** The chosen groups over time, each in its own palette colour, with the zero line
 * emphasised and names at the line ends: the surplus as a share
 * of need where the game computed it, else what was in stock. */
function OverTime({ data, tokens, viewport, options, runtime, track, extent, playhead, active, grouping, groups, onMenu, setOption, pickOwner, onPick }: {
  data: EquipmentStockpileReply; tokens: ChartTokens; viewport: number; options: Record<string, unknown>; runtime: WidgetProps['runtime']; track: BoundTrack;
  extent: [number, number]; playhead: number | null; active: boolean; grouping: Grouping; groups: StockGroup[];
  onMenu: (hour: number, x: number, y: number) => void; setOption: WidgetProps['setOption'];
  pickOwner: object; onPick: (group: StockGroup, hour: number, mode: StockpileMode) => void;
}) {
  // The share chart wherever the balance is recorded; variants have none of their own.
  const shareMode = shareModeOf(data, grouping);
  const head = Math.floor(playhead ?? extent[1]);
  const series = useMemo(() => new Map(groups.map(group => [group.key, chartRuns(data, group, grouping)])), [data, groups, grouping]);
  const drawable = useMemo(() => groups.filter(group => (series.get(group.key)?.length ?? 0) > 0), [groups, series]);
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
  // Saved choices whose handle holds another design here: shown, never drawn.
  const others = useMemo(() => otherDesigns(groups, stored.map(contentKey)), [groups, stored]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  // Each line keeps its colour from the game's order where it is free.
  const styles = useMemo(() => lineStyles(chosen.map(key => ({ key, preferred: groups.findIndex(group => group.key === key) })), tokens.categories), [chosen, groups, tokens]);
  // Only the hours shown (a date range, or the branch): an axis dragged past a range
  // draws, fits and describes nothing beyond it.
  const lines = useMemo(() => chosen.flatMap(key => {
    const group = groups.find(item => item.key === key);
    const style = styles.get(key);
    return group && style ? [{ group, color: style.color, dash: style.primary, runs: clipToSpans(series.get(key) ?? [], [{ firstHour: extent[0], lastHour: extent[1] }]) }] : [];
  }), [chosen, groups, styles, series, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const chosenScope = JSON.stringify(chosen);
  const choiceOwner = useMemo(() => ({ pickOwner, chosenScope, extent }), [pickOwner, chosenScope, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const [selection, setSelection] = useState<{ owner: object; hour: number; keys: readonly string[] } | null>(null);
  const pending = selection?.owner === choiceOwner ? selection : null;
  const layers = useMemo<ChartLayer[]>(() => [
    { draw: (ctx, frame, chartTokens) => drawValueRule(ctx, frame, chartTokens, 0) },
    ...lines.map((line): ChartLayer => ({
      draw: (ctx, frame, chartTokens, state) =>
        drawLine(ctx, frame, chartTokens, line.runs, line.color, { until: state.playhead ?? undefined, dash: line.dash.length ? line.dash : undefined }),
      extent: x => extentOfAll([line.runs], x[0], Math.min(x[1], until ?? x[1])),
    })),
  ], [lines, until]);
  const actions = new Map(groups.map(group => {
    const choice = choiceKey(group, head, chosenTokens.map(contentKey));
    if (choice == null) return [group.key, { reason: NO_DESIGN }];
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
  const aiThroughout = data.balanceHours.length === 0 && data.aiHours.length > 0;
  const caption = shareMode ? 'Surplus as a share of what is needed, where the game computed it'
    : grouping === 'variant' ? 'In stock · the game counts surplus and need by equipment type, not by variant'
      : aiThroughout ? `In stock · ${AI_NOTE}` : 'In stock · the surplus or shortage is not in this recording';
  return <>
    {setOption && <ChipSet label="Shown over time" chips={[...groups.map(group => {
      const action = actions.get(group.key);
      const cannot = !drawable.includes(group) ? undrawableReason(data, group, grouping) : undefined;
      return { value: group.key, label: labelAt(group, grouping, head), unavailable: cannot ?? (action && 'reason' in action ? action.reason : undefined) };
    }), ...others.map(other => ({ value: setToken(other.key), label: `${labelAt(other.group, grouping, head)} (saved design)`, unavailable: other.reason }))]}
      selected={chosen} max={Number.MAX_SAFE_INTEGER} onToggle={pick} />}
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
      ? 'Nothing here can be plotted: what is needed cannot be derived for these types, so they have no share to draw.'
      : 'Nothing chosen was recorded in stock.'}</p>}
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
          const keys = pickLines(lines.map(line => ({ key: line.group.key, runs: line.runs })), hour, context);
          if (keys.length === 1) {
            const group = lines.find(line => line.group.key === keys[0])?.group;
            if (group) { setSelection(null); onPick(group, hour, shareMode ? 'balance' : 'stock'); }
          } else setSelection(keys.length ? { owner: choiceOwner, hour, keys } : null);
        }}
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) return null;
          const ai = shareMode && balanceMode(data, hour) === 'ai';
          // A variant is named as recorded at the hovered hour.
          return overTimeTooltip(dayLabel(hour), lines.map(line => ({ group: { ...line.group, label: labelAt(line.group, grouping, hour) }, color: line.color,
            stock: stockAt(data, line.group, hour), weekly: weeklyAt(data, line.group, hour),
            ...(shareMode ? { logistics: balanceAt(data, line.group, hour) } : {}) })),
          data.decimals, ai ? AI_NOTE : undefined, viewport);
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!endLabels) return;
          const ends = lines.flatMap(line => {
            // At the end of the line drawn, never past the recorded hours.
            const last = lineEndHour(line.runs, frame.x[0], Math.min(until ?? extent[1], frame.x[1], extent[1]));
            const run = last == null ? null : runAt(line.runs, last);
            return run && last != null ? [{ id: line.group.key, color: line.color, muted: false, y: frame.toY(run.value), text: labelAt(line.group, grouping, last) }] : [];
          });
          placeEndLabels(ctx, frame, chartTokens, ends);
        }}
        onContextMenu={(hour, x, y) => onMenu(hour, x, y)} />
    </div>
  </>;
}
