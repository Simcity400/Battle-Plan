import { useCallback, useEffect, useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import { ChipSet } from '../../components/ChipSet';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { barEndLabel, divergingBar } from '../../charts/divergingBar';
import { dayLabel, drawLine, drawValueRule, placeEndLabels } from '../../charts/draw';
import { easeTau } from '../../charts/easing';
import { jumpItem } from '../../charts/jump';
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
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { readAt, readNote } from '../../dashboard/reads';
import type { EquipmentStockpileReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { AI_NOTE, automaticLines, CHOSEN_KEYS, hasRecords, balanceAt, balanceMode, barExtent, chartRuns, choiceKey, choiceRow, formatPercent, oneChoicePerRow, GROUP_NOUNS, groupLabel, groupTooltip, labelAt,
  NO_DESIGN, orderGroups, otherDesigns, overTimeTooltip, shareModeOf, shareOf, shareScale, stockAt, stockChoices, stockGroups, typesComplete, undrawableReason, weeklyAt, columnVars } from './stockpileData';
import type { Grouping, RowOrder, StockGroup } from './stockpileData';
import { useEquipmentStockpile } from './useEquipmentStockpile';
import './equipmentStockpile.css';

const NO_WARNINGS: string[] = [];
/** How often each record is read, for the note under a value read earlier. */
const STOCK_CADENCE = 'the stockpile is read once a day.';
const LOGISTICS_CADENCE = 'the surplus and shortage are read once a day, just after the game’s daily update.';
const LINE_CADENCE = 'production lines are read once a day.';
/** One bar row's height; rows slide by it when the order changes. */
const ROW_HEIGHT = 34;
/** Room for the names at the line ends, at the right of the plot. */
const END_LABEL_ROOM = 136;

/** The notes under a group's values at an hour: when the stockpile and, for a
 * player-controlled country, the surplus were read. */
function readNotes(data: EquipmentStockpileReply, hour: number): string | undefined {
  const stockRead = readAt(data.reads, hour);
  const lineRead = readAt(data.lineReads, hour);
  const notes = [balanceMode(data, hour) === 'player' ? readNote(readAt(data.logisticsReads, hour), hour, LOGISTICS_CADENCE) : undefined,
    readNote(stockRead, hour, STOCK_CADENCE),
    // Production dated on its own where it was read apart from the stockpile.
    lineRead !== stockRead ? readNote(lineRead, hour, LINE_CADENCE) : undefined].filter(Boolean);
  return notes.length ? notes.join(' ') : undefined;
}

/** One country's equipment stockpile (mock-up economy-1): for a player-controlled
 * country each equipment type's surplus or shortage as a share of what is needed, as
 * bars or over time; for an AI-controlled one, for which the game computes none, what
 * it holds. The hover adds produced per week and the variants in stock. */
export default function EquipmentStockpileWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useEquipmentStockpile(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  const menu = useContextMenu('Stockpile actions');
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  const grouping = (options.grouping as Grouping) ?? 'type';
  const includedText = typeof options.included === 'string' ? options.included : '';
  // Stored as `setToken(key)`, so a key a setting cannot store can be included too.
  const included = useMemo(() => contentSet(includedText).map(contentKey), [includedText]);

  if (!track || !tag) return <WidgetState kind="empty" icon="grid" message="Load a campaign and choose a country to see its equipment stockpile." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the stockpile…" />;
  // Whatever was recorded shows (production without a stockpile read, say); each value
  // not recorded says so.
  if (!hasRecords(data)) return <WidgetState kind="unavailable" message="No stockpile, production or logistics was recorded for this country in this branch." />;

  const jump = (hour: number) => [jumpItem(runtime, track.id, hour)];
  return <div className="equipment-stockpile">
    {options.view === 'time'
      ? <OverTime data={data} tokens={tokens} viewport={viewport} options={options} runtime={runtime} track={track} extent={extent} playhead={playhead}
        active={active} grouping={grouping} included={included} onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} setOption={setOption} />
      : <StockBars data={data} tokens={tokens} hour={head} grouping={grouping} included={included} order={options.order as RowOrder}
        scale={options.scale === 'moment' ? 'moment' : 'campaign'} owner={`equipment-stockpile:${widget.id}`} active={active} viewport={viewport}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} onMenu={(hour, x, y) => menu.open(x, y, jump(hour))} />}
    {menu.menu}
  </div>;
}

const percent = (fraction: number) => `${Math.round(fraction * 100000) / 1000}%`;

/** One row per group, sliding to a new order (mock-up economy-1). For a
 * player-controlled country each row's surplus (right) or shortage (left, red) as a
 * share of what is needed, all on one scale, with the exact amount at the bar's end;
 * else a bar from zero to what is in stock. Rows are focusable: hover or focus shows
 * the group's tooltip, right-click or the menu key jumps to the read shown. */
function StockBars({ data, tokens, hour, grouping, included, order, scale, owner, active, viewport, hoursPerSecond, onMenu }: {
  data: EquipmentStockpileReply; tokens: ChartTokens; hour: number; grouping: Grouping; included: readonly string[]; order: RowOrder;
  scale: 'campaign' | 'moment'; owner: string; active: boolean; viewport: number; hoursPerSecond: number; onMenu: (hour: number, x: number, y: number) => void;
}) {
  const groups = useMemo(() => stockGroups(data, grouping, hour, included), [data, grouping, hour, included]);
  // The campaign scales depend on the groups, not on the names shown at an hour.
  const allGroups = useMemo(() => stockGroups(data, grouping, Infinity, included), [data, grouping, included]);
  const campaignStock = useMemo(() => barExtent(data, allGroups, null), [data, allGroups]);
  const campaignShare = useMemo(() => shareScale(data, allGroups, null), [data, allGroups]);
  const mode = grouping === 'variant' ? 'variant' : balanceMode(data, hour);
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
    const content = groupTooltip(data, group, grouping, hour, { title: group.label, subtitle: dayLabel(hour), note: readNotes(data, hour) }, viewport);
    return [group.key, { content, text: tooltipText(content) }] as const;
  })), [data, groups, grouping, hour, viewport]);
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
          return <li key={row.key} className={movingRowClass(row)} style={{ transform: `translateY(${row.slot * ROW_HEIGHT}px)`, height: ROW_HEIGHT }}
            aria-label={groupLabel(data, group, hour)} {...rowTips.rowProps(row.key)}>
            {/* The full tooltip, even where the window trims it (the tooltip layer is not read out). */}
            <span id={rowTips.descriptionId(row.key)} className="sr-only">{described.get(row.key)?.text}</span>
            <span className="stockpile-name">
              {/* Equipment art is not wired yet: the game draws a type with the country's best variant and that variant's icon, both recorded. */}
              <span className="stockpile-art" aria-hidden="true" />
              <span>{group.label}</span>
            </span>
            {diverging ? divergingRow(group) : stockRow(group)}
          </li>;
        })}
      </ol>}
  </div>;
}

/** The chosen groups over time, each in its own palette colour, with the zero line
 * emphasised and names at the line ends (mock-up economy-1): the surplus as a share
 * of need where the game computed it, else what was in stock. */
function OverTime({ data, tokens, viewport, options, runtime, track, extent, playhead, active, grouping, included, onMenu, setOption }: {
  data: EquipmentStockpileReply; tokens: ChartTokens; viewport: number; options: Record<string, unknown>; runtime: WidgetProps['runtime']; track: BoundTrack;
  extent: [number, number]; playhead: number | null; active: boolean; grouping: Grouping; included: readonly string[];
  onMenu: (hour: number, x: number, y: number) => void; setOption: WidgetProps['setOption'];
}) {
  // The share chart wherever the balance is recorded; variants have none of their own.
  const shareMode = shareModeOf(data, grouping);
  const head = Math.floor(playhead ?? extent[1]);
  const groups = useMemo(() => stockGroups(data, grouping, Infinity, included), [data, grouping, included]);
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
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) return null;
          const ai = shareMode && balanceMode(data, hour) === 'ai';
          // A variant is named as recorded at the hovered hour.
          return overTimeTooltip(dayLabel(hour), lines.map(line => ({ group: { ...line.group, label: labelAt(line.group, grouping, hour) }, color: line.color,
            stock: stockAt(data, line.group, hour), weekly: weeklyAt(data, line.group, hour),
            ...(shareMode ? { logistics: balanceAt(data, line.group, hour) } : {}) })),
          data.decimals, [readNotes(data, hour), ai ? AI_NOTE : undefined].filter(Boolean).join(' ') || undefined, viewport);
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
