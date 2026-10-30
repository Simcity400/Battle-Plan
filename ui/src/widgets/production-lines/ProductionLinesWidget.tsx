import ArtFallback from '../../components/ArtFallback';
import { useCallback, useEffect, useMemo, useState, useRef } from 'react';
import type { ComponentProps, CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import GameWindowPopover from '../../components/GameWindowPopover';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import DrillHost, { FollowControl } from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill, pinnedBreakdown } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import { easeTau, motionOf } from '../../charts/easing';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip, tooltipText } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { movingRowClass, useRowMotion } from '../../charts/useRowMotion';
import { useRowTooltips } from '../../charts/useRowTooltips';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { datedCountryToken, readDatedCountry } from '../../dashboard/datedCountry';
import type { DatedCountrySelection } from '../../dashboard/datedCountry';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { readAt } from '../../dashboard/reads';
import type { ProductionLineDetailReply, ProductionLinesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { approximateWithin, bandScale, wantedDetailWindows, breakdownItems, breakdownTooltip, emptyHistoryState, firstViewLineReason, detailOf, efficiencyFraction, efficiencyPercent, factoryScale, finalTooltip, gapTooltip, historyLanes, iconImage, lineTooltip, linesAt, linesKnown,
  organisationAt, organisationText, outputAt, outputTotal, productionLineKey, productionLineToken, productionLineLabel, stepTooltip, totalText, unreadNote, variantAt, variantName, weeklyOutput, weeklyText, windowOf } from './productionData';
import type { LineAt, LineOrder, TooltipForms } from './productionData';
import { useLineDetail, useProductionLines, useLineBreakdown, useProductionCountry } from './useProductionLines';
import ProductionDrill from './ProductionDrill';
import EquipmentDesignDetail from './EquipmentDesignDetail';
import './productionLines.css';

const NO_WARNINGS: string[] = [];
const NO_WINDOWS: number[] = [];
type TableTipPart = 'line' | 'efficiency' | 'output';
const tableTipKey = (key: string, part: TableTipPart) => JSON.stringify([key, part]);
const stopRowEvent = <E,>(handler: (event: E) => void) => (event: E & { stopPropagation: () => void }) => {
  event.stopPropagation(); handler(event);
};

/** How often lines are read, for the note under a value read earlier. */

/** One country's production lines. Live (following playback or a pinned moment):
 * the lines running then, like the game's production tab. Otherwise: one lane per
 * line across the campaign, its thickness the factories assigned.
 * The history holds each line's factory steps; every other value comes a window of
 * reads at a time: the live table's (and the next, so playback never waits) or the
 * one a history tooltip needs. */
export default function ProductionLinesWidget({ widget, runtime, active, onDiagnostics, setOption, countrySlot }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const lines = useProductionLines(widget, runtime, active);
  const { track, tag, data, retry } = lines;
  const error = useHeldFailure(lines.error, data !== undefined, retry);
  const currentData = useRef(data);
  currentData.current = data;
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
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
    return wantedDetailWindows(data, track.branch.lastHour, shownHour, hovered);
  }, [data, track, shownHour, hovered]);
  const windows = useLineDetail(runtime, track, tag, wanted, active);
  // What an open organisation window could not draw.
  const [windowNotes, setWindowNotes] = useState<string[]>(NO_WARNINGS);
  const anchor = useRef<HTMLDivElement>(null);
  const [storedDrill, setDrill] = useState(CLOSED);
  const [lineKey, setLineKey] = useState<string | null>(null);
  const [pickedHour, setPickedHour] = useState<number | null>(null);
  const [pinnedFollow, setPinnedFollow] = useState(false);
  const [factor, setFactor] = useState<string | null>(null);
  const pinned = options.show === 'efficiency' || options.show === 'output';
  const key = pinned ? (track && tag ? productionLineKey(track.source.metadata.campaignKey, track.branch.id, tag, options.line) : null) : lineKey;
  const selected = (pinned ? options.show : storedDrill.breakdown) === 'output' ? 'output' : 'efficiency';
  const shownFactor = factor ?? (pinned && typeof options.factor === 'string' && options.factor ? options.factor : selected);
  useEffect(() => setFactor(null), [key, selected, options.factor]);
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, options.show, options.productionCountry, options.line, widget.time]);
  const owner = useMemo(() => ({ client: runtime.client, scope }), [runtime.client, scope]);
  const [countryPick, setCountryPick] = useState<{ owner: object; selection: DatedCountrySelection } | null>(null);
  const [bodyOwner, setBodyOwner] = useState<object | null>(null);
  const shownPickedHour = bodyOwner === owner ? pickedHour : null;
  const shownPinnedFollow = bodyOwner === owner ? pinnedFollow : widget.time.mode === 'follow';
  const drill = countryPick?.owner === owner ? storedDrill : CLOSED;
  const ownerRef = useRef({ owner, active });
  ownerRef.current = { owner, active };
  useEffect(() => {
    ownerRef.current.active = active;
    return () => { ownerRef.current.active = false; };
  }, [active, owner]);
  const savedCountry = useMemo(() => pinned && track ? readDatedCountry(options.productionCountry,
    track.source.metadata.campaignKey, track.branch.id, widget.countries) : null,
  [pinned, options.productionCountry, widget.countries, track?.source.metadata.campaignKey, track?.branch.id]);
  const selectedCountry = pinned ? savedCountry : countryPick?.owner === owner ? countryPick.selection : null;
  const actor = useProductionCountry(selectedCountry, runtime, track, active);
  const originHour = selectedCountry && track ? track.branch.firstHour + selectedCountry.originOffset : undefined;
  const breakdown = useLineBreakdown(runtime, track, selectedCountry?.tag, selectedCountry && actor.spans && (pinned || drill.breakdown) ? key : null, shownFactor, active, originHour);
  const drillPlaybackHour = usePlaybackHour(runtime.clock, active && (pinned ? shownPinnedFollow : drill.breakdown != null && drill.follow));
  const clockHour = track ? Math.floor(drillPlaybackHour) + track.axisOffset : null;
  const breakdownHour = pinned ? (shownPinnedFollow ? clockHour : shownPickedHour ?? shownHour ?? extent[1]) : drillHour(drill, clockHour);
  const selectedLineIndex = data?.lines.findIndex(line => line.key === key) ?? -1;
  const selectedLine = selectedLineIndex >= 0 ? data?.lines[selectedLineIndex] : null;
  const lineLabel = data && selectedLine && !firstViewLineReason(data) ? productionLineLabel(data, selectedLine, selectedLineIndex, breakdownHour ?? extent[1]) : 'Production line';
  const missingSavedDate = pinned && !shownPinnedFollow && shownPickedHour === null && (widget.time.mode === 'pin' || widget.time.mode === 'range') && !window;
  useEffect(() => { setDrill(CLOSED); setLineKey(null); setPickedHour(null); setPinnedFollow(widget.time.mode === 'follow'); }, [runtime.client, track?.source.id, track?.source.revision, track?.branch.id, tag, widget.time]);
  const headerTag = actor.spans && breakdownHour !== null ? countryDrillTagAt(actor.spans, breakdownHour) : null;
  const headerReason = missingSavedDate ? 'The chosen moment is unavailable.' : !selectedCountry ? 'Saved country selection unavailable. Open the breakdown again.'
    : actor.reason ?? (headerTag ? null : 'Country unavailable at this time.');
  const header = useCountryLabelsAt(runtime.client, track, pinned && countrySlot && headerTag && !headerReason && breakdownHour !== null
    ? [{ tag: headerTag, hour: breakdownHour }] : [], active);
  const pinnedHeader = pinned && countrySlot ? createPortal(<DatedCountryChip hour={missingSavedDate ? null : breakdownHour} reason={headerReason}
    label={headerTag && !headerReason && breakdownHour !== null ? header.labelAt(breakdownHour)(headerTag) : null} />, countrySlot) : null;
  const openBreakdown = (key: string, part: 'efficiency' | 'output', hour: number) => {
    if (currentData.current !== data || ownerRef.current.owner !== owner || !ownerRef.current.active) return;
    // A first view's line keys may name another appearance than the exact history's.
    if (data && firstViewLineReason(data)) return;
    setFactor(null);
    setLineKey(key);
    if (track && tag) setCountryPick({ owner, selection: { campaign: track.source.metadata.campaignKey, branch: track.branch.id,
      tag, originOffset: Math.floor(hour) - track.branch.firstHour } });
    setDrill(state => reduceDrill(state, { type: 'open', breakdown: part, hour, playing: runtime.clock.getSnapshot().playing, playhead: clockHour }));
  };
  useEffect(() => {
    const warnings = new Set(data?.warnings ?? NO_WARNINGS);
    for (const detail of windows.data.values()) for (const warning of detail.warnings) warnings.add(warning);
    for (const warning of breakdown.data?.warnings ?? []) warnings.add(warning);
    for (const run of breakdown.data?.runs ?? []) for (const warning of run.warnings) warnings.add(warning);
    for (const note of windowNotes) warnings.add(note);
    onDiagnostics(warnings.size ? [...warnings] : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, windows.data, breakdown.data, windowNotes, onDiagnostics]);

  if (!track || !tag) return <>{pinnedHeader}<WidgetState kind="empty" icon="grid" message="Load a campaign and choose a country to see its production lines." /></>;
  if (missingSavedDate) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Choose another time in widget settings." /></>;
  if (!pinned && drill.breakdown === null && playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <>{pinnedHeader}<WidgetState kind="unavailable" message="This time is outside the recording." /></>;
  if (!pinned && drill.breakdown === null && (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Choose another time in widget settings." /></>;
  if (!pinned && drill.breakdown === null && error) return <>{pinnedHeader}<WidgetState kind="error" detail={error} onRetry={retry} /></>;
  if (!pinned && drill.breakdown === null && !data) return <>{pinnedHeader}<WidgetState kind="loading" label="Loading production lines…" /></>;
  if (!pinned && drill.breakdown === null && data && !data.reads.length && data.approximate.length) return <>{pinnedHeader}<WidgetState kind="loading" label="Preparing this country’s earlier production history…" /></>;
  if (!pinned && drill.breakdown === null && data && !data.reads.length) return <>{pinnedHeader}<WidgetState kind="unavailable" message="Production lines weren’t recorded for this country." /></>;

  // A first view's line keys may name another appearance: its breakdowns wait, saying why.
  const jump = (hour: number, x: number, y: number, key?: string) => menu.open(x, y, [jumpItem(runtime, track.id, hour),
    ...(key && data ? breakdownItems(data, part => openBreakdown(key, part, hour)) : [])]);
  const liveTable = () => {
    if (!data) return null;
    const hour = Math.floor(playhead!);
    const window = windowOf(data, hour);
    const detail = windows.data.get(window);
    const failed = windows.errors.get(window);
    // A window is answered once the published history reaches its last hour.
    if (!detail) return failed ? <WidgetState kind="error" detail={failed} onRetry={windows.retry} />
      : <WidgetState kind="loading" label={approximateWithin(data, window * data.detailHours, (window + 1) * data.detailHours - 1)
        ? 'This range is still being prepared.' : 'Loading production lines…'} />;
    return <LiveTable data={data} detail={detail} hour={hour} order={options.order as LineOrder} more={options.columns === 'more'} owner={`production-lines:${widget.id}`}
      windows={{ client: runtime.client, source: { sourceId: track.source.id, revision: track.source.revision }, branchId: track.branch.id, onDiagnostics: setWindowNotes }}
      active={active} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} onMenu={jump} onBreakdown={openBreakdown} />;
  };
  const identityNotice = selectedCountry && actor.identityError ? <WidgetState kind="error" detail={actor.identityError} onRetry={actor.retry} /> : null;
  const drillView = !selectedCountry ? <WidgetState kind="unavailable" message={headerReason!} />
    : actor.error ? <WidgetState kind="error" detail={actor.error} onRetry={actor.retry} />
      : actor.reason ? <WidgetState kind="unavailable" message="Country unavailable at this time." />
        : !key ? <p className="drill-empty">Choose a production line in the widget settings.</p>
    : breakdown.error ? <WidgetState kind="error" what="the breakdown" detail={breakdown.error} onRetry={breakdown.retry} />
      : !breakdown.data ? <WidgetState kind="loading" label="Loading breakdown…" />
        : <ProductionDrill key={`${key}:${selected}`} data={breakdown.data} selected={shownFactor}
          onFactor={value => { if (!ownerRef.current.active || ownerRef.current.owner !== owner) return; setFactor(value); if (pinned) setOption?.('factor', value); }} hour={breakdownHour} extent={extent}
          playhead={(pinned ? shownPinnedFollow : drill.follow) ? clockHour : null} active={active} hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
          onPick={hour => { if (!ownerRef.current.active || ownerRef.current.owner !== owner) return; if (pinned) { setBodyOwner(owner); setPinnedFollow(false); setPickedHour(Math.floor(hour)); }
            else setDrill(state => reduceDrill(state, { type: 'pick', hour })); }} />;
  const main = data ? <>{playhead != null ? liveTable()
      : <History data={data} tag={tag} extent={extent} showEnded={options.ended !== 'hidden'} active={active}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} onMenu={jump}
        windows={windows.data} failed={windows.errors} hovered={hovered} onWant={setHovered} />}</> : null;
  return <div ref={anchor} className="production-lines">
    {pinned ? <>{pinnedHeader}<div className="drill-header">
      <span className="drill-day">{breakdownHour == null ? '' : dayLabel(breakdownHour)}</span>
      <FollowControl following={shownPinnedFollow} onFollow={() => { if (!ownerRef.current.active || ownerRef.current.owner !== owner) return; setBodyOwner(owner); setPickedHour(null); setPinnedFollow(true); }} onPause={() => { if (!ownerRef.current.active || ownerRef.current.owner !== owner) return; setBodyOwner(owner); setPickedHour(breakdownHour); setPinnedFollow(false); }} /></div>{identityNotice}{drillView}</>
      : <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown != null}
      title={`${selected === 'output' ? 'Output' : 'Efficiency'} · ${lineLabel}`} day={breakdownHour == null ? '' : dayLabel(breakdownHour)}
      following={drill.follow} canFollow={true} anchor={anchor} main={main}
      onClose={() => { if (ownerRef.current.active && ownerRef.current.owner === owner) setDrill(CLOSED); }}
      onFollow={() => { if (ownerRef.current.active && ownerRef.current.owner === owner) setDrill(state => reduceDrill(state, { type: 'follow' })); }}
      onPin={selectedCountry && actor.spans ? () => { if (!ownerRef.current.active || ownerRef.current.owner !== owner) return; const copy = pinnedBreakdown(widget, { id: selected, label: `${selected} · ${lineLabel}` }, track.id);
        const offset = Math.floor(breakdownHour ?? track.branch.lastHour) - track.branch.firstHour;
        runtime.addWidgetBelow(widget.id, { ...copy, time: drill.follow ? { mode: 'follow' }
          : { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } }, options: { ...copy.options,
            productionCountry: datedCountryToken(selectedCountry), line: key == null ? '' : productionLineToken(track.source.metadata.campaignKey, track.branch.id, tag, key), factor: shownFactor } }); } : undefined}>
      {identityNotice}{drillView}</DrillHost>}
    {menu.menu}
    <EquipmentDesignDetail key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} runtime={runtime} track={track}
      hour={breakdownHour ?? playhead ?? extent[1]} active={active} />
  </div>;
}

/** A tooltip's forms fitted to the window (the kit's `fitTooltip`); every row is
 * read out, and `full` (a caller asking for the text read out) gets them all. */
const fitted = (forms: TooltipForms, height: number, full = false) => full ? forms[0] : fitTooltip(forms, height, { full: forms[0] });

/** The lines running at the hour shown, one row each: the
 * equipment's art and names, factories assigned of those requested, efficiency,
 * output per week and the units made so far; with more columns, the queue place, IC
 * per unit and the organisation. Rows slide when the order changes, new lines flash in
 * and ended ones fade out (the kit's `useRowMotion`). Rows are focusable: hover or
 * focus shows the line's tooltip, right-click or the context-menu key jumps to the
 * read shown (the kit's `useRowTooltips`). */
function LiveTable({ data, detail, hour, order, more, owner, windows, active, hoursPerSecond, onMenu, onBreakdown }: {
  data: ProductionLinesReply; detail: ProductionLineDetailReply; hour: number; order: LineOrder; more: boolean; owner: string; active: boolean;
  /** Where an organisation's policy window is read. */
  windows: Pick<ComponentProps<typeof GameWindowPopover>, 'client' | 'source' | 'branchId' | 'onDiagnostics'>;
  hoursPerSecond: number | undefined; onMenu: (hour: number, x: number, y: number, key?: string) => void;
  onBreakdown: (key: string, part: 'efficiency' | 'output', hour: number) => void;
}) {
  const tokens = useChartTokens();
  const height = useViewportHeight();
  const read = readAt(data.reads, hour);
  const scale = useMemo(() => factoryScale(data), [data]);
  const items = useMemo(() => linesAt(data, detail, hour, order).map(at => ({ key: at.line.key, item: at })), [data, detail, hour, order]);
  const rows = useRowMotion<LineAt>(items);
  const targets = useMemo(() => new Map<string, { at: LineAt; part: TableTipPart }>(items.flatMap(({ key, item }) =>
    (['line', 'efficiency', 'output'] as const).map(part => [tableTipKey(key, part), { at: item, part }] as const))), [items]);
  const head = useMemo(() => ({ subtitle: dayLabel(hour) }), [hour]);
  const content = useCallback((key: string) => {
    const target = targets.get(key);
    if (!target) return null;
    const { at, part } = target;
    return fitted(part === 'line' ? lineTooltip(data, at.line, at.detail, at.run, hour, head, dayLabel)
      : breakdownTooltip(data, at.run, hour, part, head), height);
  }, [targets, data, hour, head, height]);
  const jumpHour = read ?? hour;
  const tips = useRowTooltips<string>({ owner, active, content, revision: `${hour}:${data.branchId}`, onMenu: (key, x, y) => onMenu(jumpHour, x, y, targets.get(key)?.at.line.key) });
  const cellProps = (key: string) => {
    const props = tips.rowProps(key);
    return { ...props, onPointerMove: stopRowEvent(props.onPointerMove), onPointerLeave: stopRowEvent(props.onPointerLeave),
      onFocus: stopRowEvent(props.onFocus), onBlur: stopRowEvent(props.onBlur),
      onContextMenu: stopRowEvent(props.onContextMenu), onKeyDown: stopRowEvent(props.onKeyDown) };
  };
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
        const rowKey = tableTipKey(row.key, 'line');
        const efficiencyKey = tableTipKey(row.key, 'efficiency');
        const outputKey = tableTipKey(row.key, 'output');
        const tip = content(rowKey);
        return <div key={row.key} role="listitem" className={`production-row ${movingRowClass(row)}`}
          style={{ transform: `translateY(calc(var(--production-head) + ${row.slot} * var(--production-row)))` }}
          aria-label={tip ? tooltipText(tip) : variantName(variant, run.variant)}
          {...(leaving ? { tabIndex: -1 } : tips.rowProps(rowKey))}>
          <span id={tips.descriptionId(rowKey)} className="sr-only">{tip ? tooltipText(tip) : ''}</span>
          {more && <span className="production-number is-muted">{run.priority + 1}</span>}
          <span className="production-equipment">
            {art ? <img className="production-art" src={art} alt="" />
              : <ArtFallback className="production-art" icon="hammer" />}
            <span><b>{variantName(variant, run.variant)}</b>{variant?.typeName && variant.typeName !== variantName(variant, run.variant) && <small>{variant.typeName}</small>}</span>
          </span>
          <span className="production-cell"><span><b className="production-number">{run.activeFactories}</b><small> / {run.requestedFactories}</small></span>
            <span className="production-meter" aria-hidden="true"><i style={{ width: percent(run.activeFactories) }} /><em style={{ left: percent(run.requestedFactories) }} /></span></span>
          <span className="production-cell" {...(leaving ? { tabIndex: -1 } : cellProps(efficiencyKey))}>
            <span id={tips.descriptionId(efficiencyKey)} className="sr-only">{tooltipText(breakdownTooltip(data, run, hour, 'efficiency', head)[0])}</span>
            <button type="button" className="production-factor production-number" disabled={leaving} aria-label={`${variantName(variant, run.variant)} efficiency breakdown`}
              aria-describedby={tips.descriptionId(efficiencyKey)} onClick={() => onBreakdown(row.key, 'efficiency', hour)}>{`${efficiencyPercent(run)}%`}</button>
            <span className="production-meter" aria-hidden="true"><i style={{ width: `${efficiencyFraction(run) * 100}%` }} /></span></span>
          <span className="production-number is-number" {...(leaving ? { tabIndex: -1 } : cellProps(outputKey))}>
            <span id={tips.descriptionId(outputKey)} className="sr-only">{tooltipText(breakdownTooltip(data, run, hour, 'output', head)[0])}</span>
            <button type="button" className="production-factor" disabled={leaving} aria-label={`${variantName(variant, run.variant)} output breakdown`}
              aria-describedby={tips.descriptionId(outputKey)} onClick={() => onBreakdown(row.key, 'output', hour)}>{weekly == null ? '—' : weeklyText(weekly, data.decimals)}</button></span>
          <span className={`production-number is-number${output && outputTotal(output) != null ? '' : ' is-muted'}`}
            title={output && !output.wholeLife ? `Since ${dayLabel(output.sinceHour)}` : undefined}>{totalText(output)}</span>
          {more && <><span className="production-number is-number">{formatAmount(run.costRaw, data.decimals)}</span>
            <span className="production-organisation">{run.mio !== '0' && !leaving
              ? <GameWindowPopover {...windows} hour={hour} target={{ kind: 'mioPolicies', organisation: run.mio }}
                className="production-factor production-organisation-trigger" heading={organisationText(organisation, run)}
                label={organisation?.name ? `Show ${organisation.name}’s policy window` : 'Show the organisation’s policy window'}>
                {orgArt ? <img src={orgArt} alt="" /> : <ArtFallback icon="factory" />}<small>{organisationText(organisation, run)}</small>
              </GameWindowPopover>
              : <>{orgArt ? <img src={orgArt} alt="" /> : run.mio !== '0' && <ArtFallback icon="factory" />}
                <small>{organisationText(organisation, run)}</small></>}</span></>}
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
  data: ProductionLinesReply; tag: string; extent: [number, number]; showEnded: boolean; active: boolean; hoursPerSecond: number | undefined;
  onMenu: (hour: number, x: number, y: number, key?: string) => void; windows: ReadonlyMap<number, ProductionLineDetailReply>; failed: ReadonlyMap<number, string>;
  hovered: number | null; onWant: (window: number) => void;
}) {
  const until = extent[1];
  const height = useViewportHeight();
  const lanes = useMemo(() => historyLanes(data, until, showEnded, dayLabel), [data, until, showEnded]);
  const most = useMemo(() => bandScale(data), [data]);
  const byKey = useMemo(() => new Map(data.lines.map(line => [line.key, line])), [data]);
  // A tooltip shown while its window loads is built again when it arrives.
  const revision = useMemo(() => ({}), [data, windows, failed]); // eslint-disable-line react-hooks/exhaustive-deps
  // No lane over hours a first view does not answer yet is not "none".
  if (!lanes.length) return emptyHistoryState(data, extent) === 'preparing'
    ? <WidgetState kind="loading" label="Preparing this country’s earlier production history…" />
    : <WidgetState kind="empty" message="No production lines recorded in this time." />;
  const description = `${dayLabel(until)}: ${lanes.map(lane => `${lane.identity?.label ?? lane.id}, ${lane.label}`).join('; ')}.`;
  return <div className="production-history">
    {data.approximate.length > 0 && <p className="production-caption" role="status">Showing the published production history; later history is still being prepared.</p>}
    <Timeline label={`Production lines of ${tag}`} extent={extent} playhead={null} lanes={lanes} showIcons={false} showNames={false} bandMax={most}
      unobserved={data.approximate}
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
        const head = { subtitle: dayLabel(hour), note: missing };
        // The window of reads behind this hour: asked for once the tooltip is built.
        const window = windowOf(data, hour);
        if (window !== hovered) queueMicrotask(() => onWant(window));
        const detail = windows.get(window);
        const lineDetail = detailOf(detail, line.key);
        const run = lineDetail && runAt(lineDetail.runs, hour);
        if (!run) {
          const why = failed.has(window) ? 'Couldn’t load line details.' : detail ? 'Its other values are not recorded at this hour.' : 'Loading its other values…';
          return fitted(stepTooltip(data, step, hour, head, why), height, full);
        }
        // Its bars (which the keyboard steps through) carry the final count once the view is past its end.
        return fitted(lineTooltip(data, line, lineDetail, run, hour, head, dayLabel, until), height, full);
      }}
      onContextMenu={(hour, x, y, lane) => onMenu(hour, x, y, lane?.id)} />
  </div>;
}
