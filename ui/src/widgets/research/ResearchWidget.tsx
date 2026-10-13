import { useCallback, useContext, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import ContentSetPicker from '../../components/ContentSetPicker';
import { useContextMenu } from '../../components/ContextMenu';
import { Icon } from '../../components/Icons';
import { Popover, PopoverHeading, usePopover } from '../../components/Popover';
import type { AnchorRect } from '../../components/Popover';
import { Segmented } from '../../components/Segmented';
import ListSkeleton from '../../components/Skeleton';
import WidgetState from '../../components/WidgetState';
import { jumpItem, rowJumpProps } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf } from '../../charts/table/options';
import { hideTooltip, showTooltip } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { contentTitle } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, toggleContentSet } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { ResearchCoverage, ResearchEntry, ResearchSlotHistory } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS, VIEWS } from './definition';
import ResearchExplanation from './ResearchExplanation';
import { researchNumber } from './researchBreakdown';
import { researchJourney } from './researchJourney';
import { isUnjoinedResearch, researchArtworkTokens, researchAt, researchFolderList, researchFolderLooks, researchLatestProgress, researchStateAt } from './researchData';
import type { ResearchFolderLook } from './researchData';
import { completedRow, completedTooltip, coverageAt, currentTooltip, gapTooltip, heldRow, heldTooltip, researchDate, researchDateRange, researchDays, researchItemDate, researchLanes, researchRows } from './researchLayout';
import type { ResearchItem, ResearchLane, ResearchRow, ResearchText } from './researchLayout';
import { historyTokens, useResearchReply } from './useResearch';
import { useTechnologyArtwork } from './useTechnologyArtwork';
import '../national-focus/nationalFocus.css';
import './research.css';

const NO_WARNINGS: string[] = [];
const NO_COVERAGE: ResearchCoverage[] = [];
const NO_LANES: ResearchLane[] = [];
const NO_LOOKS: ReadonlyMap<string, ResearchFolderLook> = new Map();
type Breakdown = { slotIndex: number; entry: ResearchEntry };
type Tip = (content: () => TooltipContent, key: string, x: number, y: number) => void;
type Jump = (hour: number | (() => number | null)) => ReturnType<typeof rowJumpProps>;

/** A technology's picture, or the flask when content has none. */
function Art({ icon, className }: { icon: string | undefined; className: string }) {
  return <span className={className}>{icon ? <img src={icon} alt="" /> : <ArtFallback icon="flask" />}</span>;
}

/** Hover and keyboard focus show the same tooltip. */
function tipProps(tip: Tip, untip: () => void, key: string, content: () => TooltipContent) {
  return {
    onPointerMove: (event: { clientX: number; clientY: number }) => tip(content, key, event.clientX, event.clientY),
    onPointerLeave: untip,
    onFocus: (event: { currentTarget: Element }) => { const box = event.currentTarget.getBoundingClientRect(); tip(content, key, box.right, box.top); },
    onBlur: untip,
  };
}

/** One row of a slot's lane. A completion opens its breakdown; what an older
 * recording's slot held, the gaps and the opening are read-only. */
function LaneRow({ item, slots, coverage, text, dimmed, tip, untip, jump, onOpen }: {
  item: ResearchItem; slots: readonly ResearchSlotHistory[]; coverage: readonly ResearchCoverage[]; text: ResearchText; dimmed: (token: string) => boolean;
  tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  if (item.kind === 'gap') return <div className="research-marker is-gap" tabIndex={0} {...tipProps(tip, untip, item.key, () => gapTooltip(item))}>
    <span>Not recorded<br />{researchDateRange(item.hour, item.lastHour)}</span></div>;
  if (item.kind === 'opened' || item.kind === 'closed') return <div className={`research-marker is-${item.kind}`}><span>{researchItemDate(item)}</span></div>;
  const token = item.kind === 'completed' ? item.entry.token : item.run.token;
  const name = text.name(token), date = researchItemDate(item);
  const body = <><Art icon={text.icon(token)} className="research-cell-art" /><span className="research-cell-name">{name}</span><small>{date}</small></>;
  const className = `research-cell${dimmed(token) ? ' is-dimmed' : ''}`;
  if (item.kind === 'held') return <div className={`${className} is-held`} tabIndex={0} aria-label={`${name}, ${date}`}
    {...tipProps(tip, untip, item.key, () => heldTooltip(heldRow(coverage, item.slot, item.run, item.hour), text))}
    {...jump(item.run.endedByHour ?? item.run.lastHour)}>{body}</div>;
  return <button type="button" className={className} aria-label={`${name}, completed ${date}, show time breakdown`}
    onClick={event => { const target = event.currentTarget; untip(); onOpen({ slotIndex: item.slot.index, entry: item.entry }, () => target.isConnected ? target.getBoundingClientRect() : null); }}
    {...tipProps(tip, untip, item.key, () => completedTooltip(completedRow(slots, item.slot, item.entry, item.hour), text))}
    {...jump(item.hour)}>{body}</button>;
}

/** Live or pinned: what the slot is doing at this hour, under its lane. */
function NowCell({ lane, slots, hour, text, tip, untip, jump, onOpen }: {
  lane: ResearchLane; slots: readonly ResearchSlotHistory[]; hour: number; text: ResearchText;
  tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  const { slot } = lane;
  const state = researchStateAt(slot, hour);
  if (!state) {
    const closed = lane.items.at(-1)?.kind === 'closed';
    return <div className="research-now-cell is-unavailable" role="status"><b>{closed ? 'Closed' : 'Unavailable'}</b>{!closed && <small>Not recorded at this hour</small>}</div>;
  }
  const entry = slots.find(owner => owner.index === slot.index)?.entries.find(candidate => candidate.outcome === 'active'
    && candidate.firstObservedHour === state.entryFirstObservedHour && candidate.token === state.token);
  if (!entry && !isUnjoinedResearch(state)) {
    const banked = BigInt(state.bankedDaysRaw);
    return <div className="research-now-cell is-idle" role="status"><Art icon={undefined} className="research-now-art is-idle" />
      <b className="research-idle">Idle</b>{banked > 0n && <small>{researchNumber(banked)} days saved</small>}</div>;
  }
  const token = entry?.token ?? state.token!;
  const name = text.name(token);
  const progress = researchLatestProgress(slot, state, hour);
  const left = state.details?.remainingDays ?? null;
  const content = () => currentTooltip(token, slot.index, progress, left, !!entry, text);
  const body = <>
    <Art icon={text.icon(token)} className="research-now-art" /><b>{name}</b>
    {progress && <small className="is-progress">{Math.round(progress.fraction * 100)}%</small>}
    {progress && <span className="research-now-meter"><i style={{ width: `${progress.fraction * 100}%` }} /></span>}
  </>;
  const tips = tipProps(tip, untip, `now:${slot.index}`, content);
  return entry
    // Jump goes to the earliest joined assignment, including earlier slots.
    ? <button type="button" className="research-now-cell" aria-label={`${name}, researching in slot ${slot.index + 1}, show time breakdown`} {...tips}
      {...jump(() => researchJourney([...slots], slot, entry).firstHour)}
      onClick={event => { const target = event.currentTarget; untip(); onOpen({ slotIndex: slot.index, entry }, () => target.isConnected ? target.getBoundingClientRect() : null); }}>{body}</button>
    : <div className="research-now-cell" tabIndex={0} aria-label={`${name}, researching in slot ${slot.index + 1}`} {...tips}>{body}</div>;
}

/** The folders a filter shows: up to four colour dots and a short label. */
function FolderTrigger({ folders, chosen }: { folders: readonly ResearchFolderLook[]; chosen: ReadonlySet<string> }) {
  const picked = folders.filter(folder => chosen.has(folder.key));
  const shown = (picked.length ? picked : folders).slice(0, 4);
  return <span className="research-folder-trigger"><span className="research-folder-dots" aria-hidden="true">
    {shown.map(folder => <i key={folder.key} style={{ background: folder.colour }} />)}</span>
  <span>{picked.length === 0 ? 'All folders' : picked.length === 1 ? picked[0].name : `${picked.length} folders`}</span></span>;
}

export default function ResearchWidget({ widget, runtime, active, onDiagnostics, headerSlot, setOption }: WidgetProps) {
  const menu = useContextMenu('Research actions');
  const owner = useId();
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const actions = useContext(HeaderActionsSlot);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'list' ? 'list' : 'columns';
  const { track, tag, data, error, retry } = useResearchReply(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const moment = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const { categories } = useChartTokens();
  const slots = useMemo(() => data && hour != null ? researchAt(data.history, hour) : null, [data, hour]);
  const coverage = useMemo(() => data && hour != null ? coverageAt(data.history.coverage, hour) : NO_COVERAGE, [data, hour]);
  const lanes = useMemo(() => slots && hour != null ? researchLanes(slots, coverage, hour) : NO_LANES, [slots, coverage, hour]);
  // Colours follow the country's whole history, so they hold through playback.
  const looks = useMemo(() => data ? researchFolderLooks(data, historyTokens(data.history), categories) : NO_LOOKS, [data, categories]);
  const folders = useMemo(() => researchFolderList(looks), [looks]);
  const chosenTokens = useMemo(() => contentSet(options.folders), [options.folders]);
  const chosen = useMemo(() => new Set(chosenTokens.map(contentKey)), [chosenTokens]);
  const dimmed = useCallback((token: string) => chosen.size > 0 && !chosen.has(looks.get(token)?.key ?? ''), [chosen, looks]);
  const imageTokens = useMemo(() => researchArtworkTokens(slots, hour, moment), [slots, hour, moment]);
  const images = useTechnologyArtwork(runtime.client, track, imageTokens, active && !!data);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, images.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const artwork = useMemo(() => new Map(data?.artwork.map(art => [art.token, { ...art, icon: images.artwork.get(art.token)?.icon ?? art.icon }])), [data, images.artwork]);
  const text = useMemo<ResearchText>(() => ({
    name: token => contentTitle(artwork.get(token)?.name, token, 'Technology name unavailable'),
    icon: token => artwork.get(token)?.icon ?? undefined,
    folder: token => looks.get(token),
  }), [artwork, looks]);
  const tip = useCallback<Tip>((content, key, x, y) => showTooltip(owner, content(), x, y, key), [owner]);
  const untip = useCallback(() => hideTooltip(owner), [owner]);
  useEffect(() => untip, [untip]);
  const breakdown = usePopover('start');
  const [shown, setShown] = useState<Breakdown | null>(null);
  const open = useCallback((target: Breakdown, anchor: () => AnchorRect | null) => { setShown(target); breakdown.openAt(anchor); }, [breakdown]);
  const jump = useCallback<Jump>(at => track ? rowJumpProps(runtime, track.id, at, menu.open) : {}, [runtime, track, menu.open]);
  const completed = lanes.reduce((total, lane) => total + lane.items.filter(item => item.kind === 'completed').length, 0);
  const held = lanes.some(lane => lane.items.some(item => item.kind === 'held') || lane.slot.states.some(isUnjoinedResearch));
  const slot = actions ?? headerSlot;
  const header = <>
    {slot && createPortal(<Segmented value={view} options={VIEWS} onChange={value => setOption?.('view', value)} label="Research view" />, slot)}
    {headerSlot && completed > 0 && createPortal(<span className="pill">{usNumber.format(completed)} completed</span>, headerSlot)}
  </>;
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <>{header}<div className="dash-empty" role="status">This time is outside the recording.</div></>;
  if (error) return <>{header}<WidgetState kind="error" detail={error} onRetry={retry} /></>;
  if (!data || !slots) return <>{header}<ListSkeleton label="Loading research…" /></>;
  if (!lanes.length) return <>{header}<div className="dash-empty" role="status"><span className="empty-icon"><Icon name="history" /></span>
    <p>No research slots recorded by {researchDate(hour)}.</p></div></>;
  const shownSlot = shown ? slots.find(owner => owner.index === shown.slotIndex) : undefined;
  const shownEntry = shownSlot?.entries.find(entry => entry.firstObservedHour === shown!.entry.firstObservedHour && entry.token === shown!.entry.token);
  const choices = folders.map(folder => ({ value: setToken(folder.key), label: folder.name }));
  const toggle = (token: string) => { const next = toggleContentSet(options.folders, token, Number.MAX_SAFE_INTEGER); if (next != null) setOption?.('folders', next); };
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="research-widget" aria-label="Research history">
    {header}
    {held && <p className="research-note" role="note"><Icon name="info" /><span>This recording is too old to show when research finished.
      {view === 'columns' ? ' Each column lists what that slot worked on, in order; “until” is when it had left the slot, not a completion.'
        : ' Rows are what each slot held; “until” is when it had left the slot, not a completion.'}</span></p>}
    {choices.length > 0 && <div className="research-toolbar">
      <ContentSetPicker label="Research folders" automatic="All folders" choices={choices} selected={chosenTokens} onToggle={toggle}
        onReset={() => { if (chosenTokens.length) setOption?.('folders', ''); }} trigger={<FolderTrigger folders={folders} chosen={chosen} />}
        triggerClassName="button small research-folder-button" />
    </div>}
    {view === 'columns'
      ? <ResearchColumns lanes={lanes} slots={slots} coverage={coverage} hour={hour} moment={moment} text={text} dimmed={dimmed} tip={tip} untip={untip} jump={jump} onOpen={open} />
      : <ResearchList lanes={lanes} slots={slots} coverage={coverage} held={held} text={text} chosen={chosen} looks={looks}
        sort={tableSortOf(options.tableSort, DEFAULT_SORT)} density={options.density === 'compact' ? 'compact' : 'comfortable'}
        setOption={setOption} menu={row => row.kind === 'gap' ? [] : [jumpItem(runtime, track.id, row.kind === 'held' ? row.run.endedByHour ?? row.run.lastHour : row.hour)]}
        onOpen={open} />}
    <Popover menu={breakdown} className="research-popover" aria-label="Research time breakdown">
      {breakdown.open && shownSlot && shownEntry && <>
        <PopoverHeading title={text.name(shownEntry.token)} menu={breakdown} />
        <ResearchExplanation history={data.history} slots={slots} slot={shownSlot} entry={shownEntry} description={artwork.get(shownEntry.token)?.description} />
      </>}
    </Popover>
    {menu.menu}
  </section>;
}

/** One column per slot, oldest at the top. A slot that opened during the
 * recording starts at the row the other columns had reached by then. */
function ResearchColumns({ lanes, slots, coverage, hour, moment, text, dimmed, tip, untip, jump, onOpen }: {
  lanes: readonly ResearchLane[]; slots: readonly ResearchSlotHistory[]; coverage: readonly ResearchCoverage[]; hour: number; moment: boolean; text: ResearchText;
  dimmed: (token: string) => boolean; tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  return <div className="research-scroll">
    <div className="research-lanes" style={{ '--research-slots': lanes.length } as CSSProperties}>
      {lanes.map(lane => <div className="research-lane-head" key={`head:${lane.slot.index}`}>Slot {lane.slot.index + 1}</div>)}
      {lanes.map(lane => <ol className="research-lane" key={`lane:${lane.slot.index}`} aria-label={`Slot ${lane.slot.index + 1}`}
        style={{ marginTop: `calc(${lane.offset} * var(--research-row))` }}>
        {lane.items.map(item => <li key={item.key}>
          <LaneRow item={item} slots={slots} coverage={coverage} text={text} dimmed={dimmed} tip={tip} untip={untip} jump={jump} onOpen={onOpen} /></li>)}
      </ol>)}
      {moment && <>
        <div className="research-now-label">Now · {researchDate(hour)}</div>
        {lanes.map(lane => <div className="research-now-slot" key={`now:${lane.slot.index}`}>
          <NowCell lane={lane} slots={slots} hour={hour} text={text} tip={tip} untip={untip} jump={jump} onOpen={onOpen} /></div>)}
      </>}
    </div>
  </div>;
}

/** Every completed technology (or what an older recording's slots held) as one
 * table row; the recording's gaps keep their place while sorted by date. */
function ResearchList({ lanes, slots, coverage, held, text, chosen, looks, sort, density, setOption, menu, onOpen }: {
  lanes: readonly ResearchLane[]; slots: readonly ResearchSlotHistory[]; coverage: readonly ResearchCoverage[]; held: boolean; text: ResearchText;
  chosen: ReadonlySet<string>; looks: ReadonlyMap<string, ResearchFolderLook>; sort: { column: string; direction: 'ascending' | 'descending' };
  density: 'comfortable' | 'compact'; setOption: WidgetProps['setOption'];
  menu: (row: ResearchRow) => ReturnType<typeof jumpItem>[]; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const rows = useMemo(() => researchRows(lanes, slots, coverage).filter(row => row.kind === 'gap'
    ? sort.column === 'date' : chosen.size === 0 || chosen.has(looks.get(row.token)?.key ?? '')), [lanes, slots, coverage, sort.column, chosen, looks]);
  const columns = useMemo<readonly TableColumn<ResearchRow>[]>(() => [
    { key: 'date', label: 'Date', header: 'Date', width: 'max-content', value: row => row.hour, renderMissing: true,
      render: row => <span className="research-list-date">{row.kind === 'gap' ? 'Not recorded' : researchItemDate(row)}</span> },
    { key: 'technology', label: held ? 'Technology held' : 'Technology', header: held ? 'Technology held' : 'Technology', width: 'minmax(var(--table-name-min), 1fr)',
      value: row => row.kind === 'gap' ? null : text.name(row.token), renderMissing: true,
      render: row => row.kind === 'gap' ? <span className="research-list-gap">{researchDateRange(row.hour, row.lastHour)}</span>
        : <span className="research-list-tech"><Art icon={text.icon(row.token)} className="research-list-art" /><span>{text.name(row.token)}</span></span> },
    { key: 'folder', label: 'Folder', header: 'Folder', width: 'minmax(6em, 0.6fr)',
      value: row => row.kind === 'gap' ? null : looks.get(row.token)?.name ?? null, renderMissing: true,
      render: row => { const look = row.kind === 'gap' ? undefined : looks.get(row.token);
        return look ? <span className="research-list-folder"><i style={{ background: look.colour }} />{look.name}</span> : null; } },
    { key: 'slot', label: 'Slot', header: 'Slot', numeric: true, width: 'max-content', value: row => row.kind === 'gap' ? null : row.slot.index + 1, renderMissing: true,
      render: row => row.kind === 'gap' ? null : row.slot.index + 1 },
    { key: 'took', label: held ? 'Held' : 'Took', header: held ? 'Held' : 'Took', numeric: true, width: 'max-content',
      value: row => row.kind === 'completed' ? row.tookHours : row.kind === 'held' ? row.heldHours : null, renderMissing: true,
      render: row => { if (row.kind === 'gap') return null; const hours = row.kind === 'completed' ? row.tookHours : row.heldHours;
        return hours === null ? <span className="research-list-missing">—</span> : researchDays(hours); } },
  ], [held, text, looks]);
  const tooltip = useCallback((row: ResearchRow) => row.kind === 'completed' ? completedTooltip(row, text) : row.kind === 'held' ? heldTooltip(row, text) : gapTooltip(row), [text]);
  const click = (row: ResearchRow) => {
    if (row.kind !== 'completed') return;
    const at = pointer.current, focused = document.activeElement;
    pointer.current = null;
    const box: AnchorRect | null = at ? { left: at.x, right: at.x, top: at.y, bottom: at.y }
      : focused instanceof HTMLElement ? focused.getBoundingClientRect() : null;
    onOpen({ slotIndex: row.slot.index, entry: row.entry }, () => box);
  };
  const body: ReactNode = rows.length
    ? <Table label="Research history" rows={rows} columns={columns} rowKey={row => row.key} sort={sort} setOption={setOption}
      cellBars={false} rankMarkers={false} density={density} muted={row => row.kind === 'gap'} onRowClick={click} menu={menu} tooltip={tooltip} />
    : <div className="dash-empty" role="status">No research in these folders by this date.</div>;
  return <div className="research-list" onPointerDownCapture={event => { pointer.current = { x: event.clientX, y: event.clientY }; }}>{body}</div>;
}
