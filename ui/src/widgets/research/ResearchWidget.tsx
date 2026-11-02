import { useCallback, useContext, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { NarrowTable } from '../../charts/table/options';
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
import { VirtualList } from '../../components/VirtualList';
import { jumpItem, rowJumpProps } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import Table from '../../charts/table/Table';
import { PreparedLogCoverage } from '../../charts/log/LogList';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { hideTooltip, showTooltip } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { contentTitle } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, toggleContentSet } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useParityRows } from '../../dashboard/parityRows';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { ResearchCoverage, ResearchEntry, ResearchSlotHistory } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS, VIEWS } from './definition';
import ResearchExplanation from './ResearchExplanation';
import { researchNumber } from './researchBreakdown';
import { ResearchWithDoctrines } from './DoctrinePanel';
import { researchJourney } from './researchJourney';
import { isUnjoinedResearch, researchArt, researchArtwork, researchAt, researchEntryAt, researchFolderAt, researchFolderList, researchFolderLooks, researchHeldRuns, researchLatestProgress, researchRecordedTemplates, researchStateAt } from './researchData';
import type { ResearchFolderLook, ResearchFolderLooks } from './researchData';
import { completedRow, completedTooltip, coverageAt, currentTooltip, gapTooltip, heldRow, heldTooltip, researchDate, researchDateRange, researchDays, researchItemDate, researchLanesAt, researchRows } from './researchLayout';
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
const laneInteractive = (item: ResearchItem) => item.kind !== 'opened' && item.kind !== 'closed';
/** Legacy slot states have no occurrence index; only completion assignments record it. */
const rowIndex = (row: ResearchRow) => row.kind === 'completed' ? row.entry.databaseIndex ?? null : null;

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
  item: ResearchItem; slots: readonly ResearchSlotHistory[]; coverage: readonly ResearchCoverage[]; text: ResearchText;
  dimmed: (token: string, databaseIndex?: number | null) => boolean;
  tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  if (item.kind === 'gap') return <div className="research-marker is-gap" tabIndex={0} {...tipProps(tip, untip, item.key, () => gapTooltip(item))}>
    <span>Not recorded<br />{researchDateRange(item.hour, item.lastHour)}</span></div>;
  if (item.kind === 'opened' || item.kind === 'closed') return <div className={`research-marker is-${item.kind}`}><span>{researchItemDate(item)}</span></div>;
  const token = item.kind === 'completed' ? item.entry.token : item.run.token;
  const databaseIndex = item.kind === 'completed' ? item.entry.databaseIndex ?? null : null;
  const name = text.name(token, databaseIndex), date = researchItemDate(item);
  const body = <><Art icon={text.icon(token)} className="research-cell-art" /><span className="research-cell-name">{name}</span><small>{date}</small></>;
  const className = `research-cell${dimmed(token, databaseIndex) ? ' is-dimmed' : ''}`;
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
  const entry = researchEntryAt(slot, state, hour);
  if (!entry && !isUnjoinedResearch(state)) {
    const banked = BigInt(state.bankedDaysRaw);
    return <div className="research-now-cell is-idle" role="status"><Art icon={undefined} className="research-now-art is-idle" />
      <b className="research-idle">Idle</b>{banked > 0n && <small>{researchNumber(banked)} days saved</small>}</div>;
  }
  const token = entry?.token ?? state.token!;
  const databaseIndex = entry?.databaseIndex ?? null;
  const name = text.name(token, databaseIndex);
  const progress = researchLatestProgress(slot, state, hour);
  const left = state.details?.remainingDays ?? null;
  const content = () => currentTooltip(token, slot.index, progress, left, !!entry, text, null, databaseIndex);
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

export default function ResearchWidget(props: WidgetProps) {
  return <ResearchWithDoctrines {...props} body={ResearchBody} />;
}
function ResearchBody({ widget, runtime, active, onDiagnostics, headerSlot, setOption }: WidgetProps) {
  const menu = useContextMenu('Research actions');
  const owner = useId();
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const actions = useContext(HeaderActionsSlot);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'list' ? 'list' : 'columns';
  const { track, tag, data, error, retry, preparing } = useResearchReply(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour;
  const moment = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const { categories } = useChartTokens();
  const slots = data?.history.slots ?? null;
  const coverage = useMemo(() => data && hour != null ? coverageAt(data.history.coverage, hour) : NO_COVERAGE, [data, hour]);
  const lanes = useMemo(() => data && hour != null ? researchLanesAt(data.history, hour) : NO_LANES, [data, hour]);
  // Colours follow the country's whole history, so they hold through playback.
  const looks = useMemo(() => data ? researchFolderLooks(data, historyTokens(data.history), categories) : NO_LOOKS, [data, categories]);
  const folders = useMemo(() => researchFolderList(looks), [looks]);
  const chosenTokens = useMemo(() => contentSet(options.folders), [options.folders]);
  const chosen = useMemo(() => new Set(chosenTokens.map(contentKey)), [chosenTokens]);
  const dimmed = useCallback((token: string, databaseIndex?: number | null) =>
    chosen.size > 0 && !chosen.has(researchFolderAt(looks, token, databaseIndex)?.key ?? ''), [chosen, looks]);
  const completedTokens = useMemo(() => lanes.flatMap(lane => [
    ...lane.items.flatMap(item => item.kind === 'completed' ? [item.entry.token] : []),
    ...lane.items.flatMap(item => item.kind === 'held' ? [item.run.token] : []),
  ]), [lanes]);
  const assignments = moment && hour != null ? lanes.flatMap(lane => {
    const state = researchStateAt(lane.slot, hour), entry = state && researchEntryAt(lane.slot, state, hour);
    return entry ? [entry.token] : isUnjoinedResearch(state) ? [state.token] : [];
  }) : [];
  const assignmentKey = JSON.stringify(assignments);
  const imageTokens = useMemo(() => [...new Set([...assignments, ...completedTokens])], [completedTokens, assignmentKey]);
  const recorded = useMemo(() => researchRecordedTemplates(data?.history), [data]);
  const images = useTechnologyArtwork(runtime.client, track, imageTokens, active && !!data, recorded);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, images.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const artwork = useMemo(() => researchArtwork(data, images.artwork), [data, images.artwork]);
  const text = useMemo<ResearchText>(() => ({
    name: (token, databaseIndex) => contentTitle(researchArt(artwork, token, databaseIndex)?.name, token, 'Technology name unavailable'),
    icon: token => artwork.get(token)?.icon ?? undefined,
    artworkFailure: token => images.failures?.get(token),
    folder: (token, databaseIndex) => researchFolderAt(looks, token, databaseIndex),
  }), [artwork, looks, images.failures]);
  const tip = useCallback<Tip>((content, key, x, y) => showTooltip(owner, content(), x, y, key), [owner]);
  const untip = useCallback(() => hideTooltip(owner), [owner]);
  useEffect(() => untip, [untip]);
  const breakdown = usePopover('start');
  const detailsSlots = useMemo(() => breakdown.open && data && hour != null ? researchAt(data.history, hour) : null, [breakdown.open, data, hour]);
  const [shown, setShown] = useState<Breakdown | null>(null);
  const open = useCallback((target: Breakdown, anchor: () => AnchorRect | null) => { setShown(target); breakdown.openAt(anchor); }, [breakdown]);
  const jump = useCallback<Jump>(at => track ? rowJumpProps(runtime, track.id, at, menu.open) : {}, [runtime, track, menu.open]);
  const completed = useMemo(() => lanes.reduce((total, lane) => total + lane.items.filter(item => item.kind === 'completed').length, 0), [lanes]);
  const held = lanes.some(lane => (researchHeldRuns(lane.slot)[0]?.firstHour ?? Infinity) <= (hour ?? -Infinity));
  const slot = actions ?? headerSlot;
  const header = <>
    {slot && createPortal(<Segmented value={view} options={VIEWS} onChange={value => setOption?.('view', value)} label="Research view" />, slot)}
    {headerSlot && completed > 0 && createPortal(<span className="pill">{usNumber.format(completed)} completed</span>, headerSlot)}
  </>;
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <>{header}<div className="dash-empty" role="status">This time is outside the recording.</div></>;
  const failure = error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : null;
  if (failure && !data) return <>{header}{failure}</>;
  if (!data || !slots) return <>{header}{preparing ? <PreparedLogCoverage coverage={null} /> : <ListSkeleton label="Loading research…" />}</>;
  if (!lanes.length && preparedPrefix(data)?.pending.length) return <>{header}{failure}<PreparedLogCoverage coverage={preparedPrefix(data)?.coverage} /></>;
  if (!lanes.length) return <>{header}{failure}<div className="dash-empty" role="status"><span className="empty-icon"><Icon name="history" /></span>
    <p>No research slots recorded by {researchDate(hour)}.</p></div></>;
  const shownSlot = shown ? detailsSlots?.find(owner => owner.index === shown.slotIndex) : undefined;
  const shownEntry = shownSlot?.entries.find(entry => entry.firstObservedHour === shown!.entry.firstObservedHour && entry.token === shown!.entry.token);
  const choices = folders.map(folder => ({ value: setToken(folder.key), label: folder.name }));
  const toggle = (token: string) => { const next = toggleContentSet(options.folders, token, Number.MAX_SAFE_INTEGER); if (next != null) setOption?.('folders', next); };
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="research-widget" aria-label="Research history">
    {header}
    {failure}
    {held && <p className="research-note" role="note"><Icon name="info" /><span>This recording is too old to show when research finished.
      {view === 'columns' ? ' Each column lists what that slot worked on, in order; “until” is when it had left the slot, not a completion.'
        : ' Rows are what each slot held; “until” is when it had left the slot, not a completion.'}</span></p>}
    {choices.length > 0 && <div className="research-toolbar">
      <ContentSetPicker label="Research folders" automatic="All folders" choices={choices} selected={chosenTokens} onToggle={toggle}
        onReset={() => { if (chosenTokens.length) setOption?.('folders', ''); }} trigger={<FolderTrigger folders={folders} chosen={chosen} />}
        triggerClassName="button small research-folder-button" />
    </div>}
    {view === 'columns'
      ? <ResearchColumns lanes={lanes} slots={slots} coverage={coverage} hour={hour} moment={moment} text={text} dimmed={dimmed} tip={tip} untip={untip} jump={jump} onOpen={open} data={data} />
      : <ResearchList lanes={lanes} slots={slots} coverage={coverage} held={held} text={text} chosen={chosen} looks={looks}
        sort={tableSortOf(options.tableSort, DEFAULT_SORT)} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        setOption={setOption} menu={row => row.kind === 'gap' ? [] : [jumpItem(runtime, track.id, row.kind === 'held' ? row.run.endedByHour ?? row.run.lastHour : row.hour)]}
        onOpen={open} />}
    <PreparedLogCoverage coverage={preparedPrefix(data)?.coverage} />
    <Popover menu={breakdown} className="research-popover" aria-label="Research time breakdown">
      {breakdown.open && shownSlot && shownEntry && <>
        <PopoverHeading title={text.name(shownEntry.token, shownEntry.databaseIndex)} menu={breakdown} />
        <ResearchExplanation history={data.history} slots={detailsSlots!} slot={shownSlot} entry={shownEntry}
          description={researchArt(artwork, shownEntry.token, shownEntry.databaseIndex)?.description} />
      </>}
    </Popover>
    {menu.menu}
  </section>;
}

/** One column per slot, oldest at the top. A slot that opened during the
 * recording starts at the row the other columns had reached by then. */
function ResearchColumns({ lanes, slots, coverage, hour, moment, text, dimmed, tip, untip, jump, onOpen, data }: {
  lanes: readonly ResearchLane[]; slots: readonly ResearchSlotHistory[]; coverage: readonly ResearchCoverage[]; hour: number; moment: boolean; text: ResearchText;
  dimmed: (token: string) => boolean; tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
  data: import('../../generated/dashboard').ResearchReply;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  if (import.meta.env.MODE === 'parity') {
    useParityRows(scroll, () => {
      const rows = lanes.flatMap((lane, group) => [
        ...lane.items.map(item => { const { slot: _slot, ...value } = item as ResearchItem & { slot?: ResearchSlotHistory };
          return { key: `${lane.slot.index}:${item.key}`, group, item: { ...value, slot: lane.slot.index, offset: lane.offset } }; }),
        ...(moment ? (() => { const current = researchStateAt(lane.slot, hour);
          return [{ key: `current:${lane.slot.index}`, group, item: { slot: lane.slot.index, offset: lane.offset,
            current: current ? { ...current, lastHour: Math.min(current.lastHour, hour) } : null,
            entry: current ? researchEntryAt(lane.slot, current, hour) : null,
            progress: current ? researchLatestProgress(lane.slot, current, hour) : null, hour } }]; })() : []),
      ]).map((row, index) => ({ ...row, index }));
      return { kind: 'windowed', label: 'Research history', complete: true, total: rows.length, hour, moment, rows,
        text: data.artwork.map(({ token, name, description, baseDurationDays }) => ({ token, name, description, baseDurationDays })) };
    });
  }
  return <div className="research-scroll" ref={scroll}>
    <div className="research-lanes" style={{ '--research-slots': lanes.length } as CSSProperties}>
      {lanes.map(lane => <div className="research-lane-head" key={`head:${lane.slot.index}`}>Slot {lane.slot.index + 1}</div>)}
      {lanes.map(lane => <VirtualList key={`lane:${lane.slot.index}`} identity={data.history} items={lane.items} itemKey={item => item.key}
        estimate={32} className="research-lane" flowWhenFits style={{ padding: undefined, marginTop: `calc(${lane.offset} * var(--research-row))` }}
        label={`Slot ${lane.slot.index + 1}`} scrollRef={scroll} retainExpansion={false} isInteractive={laneInteractive}>
        {item => <LaneRow item={item} slots={slots} coverage={coverage} text={text} dimmed={dimmed} tip={tip} untip={untip} jump={jump} onOpen={onOpen} />}
      </VirtualList>)}
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
function ResearchList({ lanes, slots, coverage, held, text, chosen, looks, sort, density, narrow, setOption, menu, onOpen }: {
  lanes: readonly ResearchLane[]; slots: readonly ResearchSlotHistory[]; coverage: readonly ResearchCoverage[]; held: boolean; text: ResearchText;
  chosen: ReadonlySet<string>; looks: ResearchFolderLooks; sort: { column: string; direction: 'ascending' | 'descending' };
  density: 'comfortable' | 'compact'; narrow: NarrowTable; setOption: WidgetProps['setOption'];
  menu: (row: ResearchRow) => ReturnType<typeof jumpItem>[]; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  const pointer = useRef<{ x: number; y: number } | null>(null);
  // Only the last coverage bound advances between lane/coverage additions; it
  // cannot change an already revealed row or the gaps between earlier spans.
  const knownRows = useMemo(() => researchRows(lanes, slots, coverage), [lanes, slots, coverage.length]);
  const rows = useMemo(() => knownRows.filter(row => row.kind === 'gap'
    ? sort.column === 'date' : chosen.size === 0 || chosen.has(researchFolderAt(looks, row.token, rowIndex(row))?.key ?? '')), [knownRows, sort.column, chosen, looks]);
  const columns = useMemo<readonly TableColumn<ResearchRow>[]>(() => [
    { key: 'date', label: 'Date', header: 'Date', width: 'max-content', value: row => row.hour, renderMissing: true,
      render: row => <span className="research-list-date">{row.kind === 'gap' ? 'Not recorded' : researchItemDate(row)}</span> },
    { key: 'technology', label: held ? 'Technology held' : 'Technology', header: held ? 'Technology held' : 'Technology', width: 'minmax(var(--table-name-min), 1fr)',
      value: row => row.kind === 'gap' ? null : text.name(row.token, rowIndex(row)), renderMissing: true,
      render: row => row.kind === 'gap' ? <span className="research-list-gap">{researchDateRange(row.hour, row.lastHour)}</span>
        : <span className="research-list-tech"><Art icon={text.icon(row.token)} className="research-list-art" /><span>{text.name(row.token, rowIndex(row))}</span></span> },
    { key: 'folder', label: 'Folder', header: 'Folder', width: 'minmax(6em, 0.6fr)', priority: 2,
      value: row => row.kind === 'gap' ? null : researchFolderAt(looks, row.token, rowIndex(row))?.name ?? null, renderMissing: true,
      render: row => { const look = row.kind === 'gap' ? undefined : researchFolderAt(looks, row.token, rowIndex(row));
        return look ? <span className="research-list-folder"><i style={{ background: look.colour }} />{look.name}</span> : null; } },
    { key: 'slot', label: 'Slot', header: 'Slot', numeric: true, width: 'max-content', priority: 3, value: row => row.kind === 'gap' ? null : row.slot.index + 1, renderMissing: true,
      render: row => row.kind === 'gap' ? null : row.slot.index + 1 },
    { key: 'took', label: held ? 'Held' : 'Took', header: held ? 'Held' : 'Took', numeric: true, width: 'max-content', priority: 1,
      text: row => { if (row.kind === 'gap') return null; const hours = row.kind === 'completed' ? row.tookHours : row.heldHours; return hours === null ? null : researchDays(hours); },
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
    ? <Table rowRevision={row => [row, columns]} label="Research history" rows={rows} columns={columns} rowKey={row => row.key} sort={sort} setOption={setOption} narrow={narrow}
      {...(import.meta.env.MODE === 'parity' ? { parity: { complete: true, item: (row: ResearchRow) => {
        if (row.kind === 'gap') return row;
        const { slot, ...item } = row;
        const state = row.kind === 'held' ? researchStateAt(slot, row.run.lastHour) : null;
        return { ...item, slot: slot.index, ...(row.kind === 'held' ? {
          progress: state ? researchLatestProgress(slot, state, row.run.lastHour) : null,
        } : {}) };
      } } } : {})}
      cellBars={false} rankMarkers={false} density={density} muted={row => row.kind === 'gap'} onRowClick={click} menu={menu} tooltip={tooltip} />
    : <div className="dash-empty" role="status">No research in these folders by this date.</div>;
  return <div className="research-list" onPointerDownCapture={event => { pointer.current = { x: event.clientX, y: event.clientY }; }}>{body}</div>;
}
