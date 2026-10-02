import { useCallback, useEffect, useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { easeTau } from '../../charts/easing';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip, tooltipText } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { movingRowClass, useRowMotion } from '../../charts/useRowMotion';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { readAt, readNote } from '../../dashboard/reads';
import type { ProductionLineDetailReply, ProductionLinesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { bandScale, detailOf, efficiencyFraction, efficiencyPercent, factoryScale, finalTooltip, gapTooltip, historyLanes, iconImage, lineTooltip, linesAt, linesKnown,
  organisationAt, organisationText, outputAt, outputTotal, stepTooltip, totalText, unreadNote, variantAt, variantName, weeklyOutput, weeklyText, windowOf, withNote } from './productionData';
import type { LineAt, LineOrder, TooltipForms } from './productionData';
import { useLineDetail, useProductionLines } from './useProductionLines';
import './productionLines.css';

const NO_WARNINGS: string[] = [];
const NO_WINDOWS: number[] = [];

/** How often lines are read, for the note under a value read earlier. */
const CADENCE = 'production lines are read once a day.';

/** One country's production lines. Live (following playback or a pinned moment):
 * the lines running then, like the game's production tab. Otherwise: one lane per
 * line across the campaign, its thickness the factories assigned (mock-up economy-1).
 * The history holds each line's factory steps; every other value comes a window of
 * reads at a time: the live table's (and the next, so playback never waits) or the
 * one a history tooltip needs. */
export default function ProductionLinesWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useProductionLines(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const live = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && live ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const menu = useContextMenu('Production line actions');
  // The window a history tooltip asked for; the live table's follow the playhead.
  const [hovered, setHovered] = useState<number | null>(null);
  useEffect(() => setHovered(null), [track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const shownHour = playhead != null ? Math.floor(playhead) : null;
  const wanted = useMemo(() => {
    if (!data || !track) return NO_WINDOWS;
    if (shownHour == null) return hovered == null ? NO_WINDOWS : [hovered];
    const window = windowOf(data, shownHour);
    // The next window too, while playback can reach it.
    return (window + 1) * data.detailHours <= track.branch.lastHour ? [window, window + 1] : [window];
  }, [data, track, shownHour, hovered]);
  const windows = useLineDetail(runtime, track, tag, wanted, active);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="grid" message="Load a campaign and choose a country to see its production lines." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading production lines…" />;
  if (!data.reads.length) return <WidgetState kind="unavailable" message="No production lines were recorded for this country in this branch." />;

  const jump = (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]);
  const liveTable = () => {
    const hour = Math.floor(playhead!);
    const window = windowOf(data, hour);
    const detail = windows.data.get(window);
    const failed = windows.errors.get(window);
    if (!detail) return failed ? <WidgetState kind="error" message={failed} onRetry={windows.retry} /> : <WidgetState kind="loading" label="Loading production lines…" />;
    return <LiveTable data={data} detail={detail} hour={hour} order={options.order as LineOrder} more={options.columns === 'more'} owner={`production-lines:${widget.id}`}
      active={active} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} onMenu={jump} />;
  };
  return <div className="production-lines">
    {playhead != null ? liveTable()
      : <History data={data} tag={tag} extent={extent} showEnded={options.ended !== 'hidden'} active={active}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} onMenu={jump}
        windows={windows.data} failed={windows.errors} hovered={hovered} onWant={setHovered} />}
    {menu.menu}
  </div>;
}

/** A tooltip's forms fitted to the window (the kit's `fitTooltip`); every row is
 * read out, and `full` (a caller asking for the text read out) gets them all. */
const fitted = (forms: TooltipForms, height: number, full = false) => full ? forms[0] : fitTooltip(forms, height, { full: forms[0] });

/** The lines running at the hour shown, one row each (mock-up economy-1): the
 * equipment's art and names, factories assigned of those requested, efficiency,
 * output per week and the units made so far; with more columns, the queue place, IC
 * per unit and the organisation. Rows slide when the order changes, new lines flash in
 * and ended ones fade out (the kit's `useRowMotion`). Rows are focusable: hover or
 * focus shows the line's tooltip, right-click or the context-menu key jumps to the
 * read shown (the kit's `useRowTooltips`). */
function LiveTable({ data, detail, hour, order, more, owner, active, hoursPerSecond, onMenu }: {
  data: ProductionLinesReply; detail: ProductionLineDetailReply; hour: number; order: LineOrder; more: boolean; owner: string; active: boolean;
  hoursPerSecond: number; onMenu: (hour: number, x: number, y: number) => void;
}) {
  const tokens = useChartTokens();
  const height = useViewportHeight();
  const read = readAt(data.reads, hour);
  const scale = useMemo(() => factoryScale(data), [data]);
  const items = useMemo(() => linesAt(data, detail, hour, order).map(at => ({ key: at.line.key, item: at })), [data, detail, hour, order]);
  const rows = useRowMotion<LineAt>(items);
  const byKey = useMemo(() => new Map(items.map(({ key, item }) => [key, item])), [items]);
  const content = useCallback((key: string) => {
    const at = byKey.get(key);
    return at ? fitted(lineTooltip(data, at.line, at.detail, at.run, hour, { subtitle: dayLabel(hour), note: readNote(read, hour, CADENCE) }, dayLabel), height) : null;
  }, [byKey, data, hour, read, height]);
  const jumpHour = read ?? hour;
  const tips = useRowTooltips<string>({ owner, active, content, revision: `${hour}:${data.branchId}`, onMenu: (_, x, y) => onMenu(jumpHour, x, y) });
  const glide = { '--row-glide': `${Math.round(3 * easeTau(tokens.motion, hoursPerSecond, tokens.reducedMotion))}ms`,
    '--production-columns': more ? 'minmax(24px, .3fr) minmax(170px, 2.2fr) 1fr 1fr .8fr .8fr .7fr 1.1fr' : 'minmax(170px, 2.4fr) 1.1fr 1.1fr .9fr .9fr',
    height: `calc(var(--production-head) + ${rows.filter(row => row.phase !== 'leaving').length} * var(--production-row))` } as CSSProperties;
  const percent = (value: number) => scale > 0 ? `${Math.min(100, value / scale * 100)}%` : '0%';
  // No line is "none" only where a complete census recorded none; elsewhere the lines are unknown.
  if (!items.length && !rows.length) {
    return linesKnown(data, hour) ? <WidgetState kind="empty" message={`No production lines running on ${dayLabel(hour)}.`} />
      : <WidgetState kind="unavailable" message={`Production lines are not recorded on ${dayLabel(hour)}.`} />;
  }
  return <>
    <p className="production-caption">{read != null ? `Read ${formatDateTime(read)}` : ''}{read != null && read < hour ? ' · lines are read once a day' : ''}</p>
    <div className="production-table" role="list" aria-label={`Production lines on ${dayLabel(hour)}`} style={glide}>
      <div className="production-head" aria-hidden="true">
        {more && <span>#</span>}<span>Equipment</span><span>Factories</span><span>Efficiency</span><span className="is-number">Output / week</span><span className="is-number">Total</span>
        {more && <><span className="is-number">IC / unit</span><span>MIO</span></>}
      </div>
      {rows.map(row => {
        const { line, detail: lineDetail, run } = row.item;
        const variant = variantAt(data, run.variant, hour);
        const art = iconImage(data, variant?.icon ?? null);
        const organisation = organisationAt(data, run.mio, hour);
        const orgArt = iconImage(data, organisation?.icon ?? null);
        const weekly = weeklyOutput(run);
        const output = outputAt(line, lineDetail, hour);
        const leaving = row.phase === 'leaving';
        const tip = content(row.key);
        return <div key={row.key} role="listitem" className={`production-row ${movingRowClass(row)}`}
          style={{ transform: `translateY(calc(var(--production-head) + ${row.slot} * var(--production-row)))` }}
          aria-label={tip ? tooltipText(tip) : variantName(variant, run.variant)}
          {...(leaving ? { tabIndex: -1 } : tips.rowProps(row.key))}>
          <span id={tips.descriptionId(row.key)} className="sr-only">{tip ? tooltipText(tip) : ''}</span>
          {more && <span className="production-number is-muted">{run.priority + 1}</span>}
          <span className="production-equipment">
            {art ? <img className="production-art" src={art} alt="" />
              : <i className="production-art is-missing" title={variant?.iconUnknown ?? 'The game shows no icon for this equipment.'} aria-hidden="true" />}
            <span><b>{variantName(variant, run.variant)}</b><small>{variant ? variant.typeName ?? variant.token : 'Name not recorded'}</small></span>
          </span>
          <span className="production-cell"><span><b className="production-number">{run.activeFactories}</b><small> / {run.requestedFactories}</small></span>
            <span className="production-meter" aria-hidden="true"><i style={{ width: percent(run.activeFactories) }} /><em style={{ left: percent(run.requestedFactories) }} /></span></span>
          <span className="production-cell"><span className="production-number">{`${efficiencyPercent(run)}%`}</span>
            <span className="production-meter" aria-hidden="true"><i style={{ width: `${efficiencyFraction(run) * 100}%` }} /></span></span>
          <span className="production-number is-number">{weekly == null ? '—' : weeklyText(weekly, data.decimals)}</span>
          <span className={`production-number is-number${output && outputTotal(output) != null ? '' : ' is-muted'}`}
            title={output && !output.wholeLife ? `Since ${dayLabel(output.sinceHour)}` : undefined}>{totalText(output)}</span>
          {more && <><span className="production-number is-number">{formatAmount(run.costRaw, data.decimals)}</span>
            <span className="production-organisation">{orgArt ? <img src={orgArt} alt="" />
              : run.mio !== '0' && <i className="production-org-art is-missing" title={organisation?.iconUnknown ?? 'The organisation’s icon is not available.'} aria-hidden="true" />}
              <small title={organisation?.nameUnknown ?? undefined}>{organisationText(organisation, run)}</small></span></>}
        </div>;
      })}
    </div>
  </>;
}

/** One lane per line across the campaign (or the chosen range): a band whose
 * thickness is the factories assigned at each read, accented while it runs and grey
 * once it has ended, named beside it with its factories or when it ended. A tooltip
 * asks for the window of reads behind its hour (`onWant`) and shows the line's
 * variant and factories until it arrives. */
function History({ data, tag, extent, showEnded, active, hoursPerSecond, onMenu, windows, failed, hovered, onWant }: {
  data: ProductionLinesReply; tag: string; extent: [number, number]; showEnded: boolean; active: boolean; hoursPerSecond: number;
  onMenu: (hour: number, x: number, y: number) => void; windows: ReadonlyMap<number, ProductionLineDetailReply>; failed: ReadonlyMap<number, string>;
  hovered: number | null; onWant: (window: number) => void;
}) {
  const until = extent[1];
  const height = useViewportHeight();
  const lanes = useMemo(() => historyLanes(data, until, showEnded, dayLabel), [data, until, showEnded]);
  const most = useMemo(() => bandScale(data), [data]);
  const byKey = useMemo(() => new Map(data.lines.map(line => [line.key, line])), [data]);
  // A tooltip shown while its window loads is built again when it arrives.
  const revision = useMemo(() => ({}), [data, windows, failed]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!lanes.length) return <WidgetState kind="empty" message="No production lines recorded in this time." />;
  const description = `${dayLabel(until)}: ${lanes.map(lane => `${lane.identity?.label ?? lane.id}, ${lane.label}`).join('; ')}.`;
  return <div className="production-history">
    <Timeline label={`Production lines of ${tag}`} extent={extent} playhead={null} lanes={lanes} showIcons={false} showNames={false} bandMax={most}
      stickyAxis={false} fadePresentation truncateLabels={false} holderIconAlign="top"
      description={description} hoursPerSecond={hoursPerSecond} active={active} revision={revision}
      tooltip={(lane, _span, hour, full) => {
        const line = byKey.get(lane.id);
        const step = line && runAt(line.steps, hour);
        if (!line) return null;
        // Over hours whose output read was lost, every tooltip says lines may be missing there.
        const missing = unreadNote(data, hour);
        // A line made between reads, or one at an hour after it ended, gives its final count.
        if (!step && (line.betweenReads || (line.endedBy != null && hour >= line.endedBy))) return fitted(finalTooltip(data, line, hour, { subtitle: dayLabel(hour), note: missing }, dayLabel), height, full);
        // Not recorded then, or before it started: this hour's status, not the lane's at the end shown.
        if (!step) return gapTooltip(data, line, hour, dayLabel(hour), dayLabel, missing);
        const read = readAt(data.reads, hour);
        const head = { subtitle: dayLabel(hour), note: withNote(readNote(read, hour, CADENCE), missing) };
        // The window of reads behind this hour: asked for once the tooltip is built.
        const window = windowOf(data, hour);
        if (window !== hovered) queueMicrotask(() => onWant(window));
        const detail = windows.get(window);
        const lineDetail = detailOf(detail, line.key);
        const run = lineDetail && runAt(lineDetail.runs, hour);
        if (!run) {
          const why = failed.has(window) ? 'Its other values could not be loaded.' : detail ? 'Its other values are not recorded at this hour.' : 'Loading its other values…';
          return fitted(stepTooltip(data, step, hour, head, why), height, full);
        }
        // Its bars (which the keyboard steps through) carry the final count once the view is past its end.
        return fitted(lineTooltip(data, line, lineDetail, run, hour, head, dayLabel, until), height, full);
      }}
      onContextMenu={(hour, x, y) => onMenu(hour, x, y)} />
  </div>;
}
