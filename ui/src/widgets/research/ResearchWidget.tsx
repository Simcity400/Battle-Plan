import { useCallback, useContext, useId, useMemo, useRef, useState } from 'react';
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
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useOpenTooltip } from '../../charts/openTooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { HeaderActionsSlot } from '../../charts/ZoomReset';
import { contentTitle } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, toggleContentSet } from '../../dashboard/options';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useHeadHour } from '../army-size/headHour';
import { useRecordedRows } from '../../dashboard/widgetPerf';
import { perfLog } from '../../shell/perf';
import type { ResearchData, ResearchSlotHistory } from './types';
import { DEFAULT_SORT, OPTIONS, VIEWS } from './definition';
import ResearchExplanation from './ResearchExplanation';
import { ResearchWithDoctrines } from './DoctrinePanel';
import { researchJourney } from './researchJourney';
import { isUnjoinedResearch, researchArt, researchArtwork, researchAt, researchEntryAt, researchFolderAt, researchFolderList, researchFolderLooks,
  researchLatestProgress, researchRecordedTemplates } from './researchData';
import type { ResearchFolderLook, ResearchFolderLooks } from './researchData';
import { completedRow, completedTooltip, coverageAt, currentTooltip, gapTooltip, researchDate, researchDateRange, researchDays, researchItemDate,
  recordedResearchLanes, researchLanesAt, researchRevealKey, researchRows } from './researchLayout';
import type { ResearchItem, ResearchLane, ResearchRow, ResearchText } from './researchLayout';
import { slotNumbers, slotsStructure } from './researchSlots';
import type { SlotInspection } from './researchSlots';
import { historyTokens, useResearchReply, useResearchSlots } from './useResearch';
import { useTechnologyArtwork } from './useTechnologyArtwork';
import './research.css';
import type { ResearchCoverage, ResearchEntry } from '../../generated/dashboard';
import type { ResearchSlotsAt } from './types';

const NO_WARNINGS: string[] = [];
const NO_TOKENS: readonly string[] = [];
const NO_COVERAGE: ResearchCoverage[] = [];
const NO_LANES: ResearchLane[] = [];
const NO_LOOKS: ReadonlyMap<string, ResearchFolderLook> = new Map();
type Breakdown = { slotIndex: number; entry: ResearchEntry; hour: number };
type Tip = (content: () => TooltipContent | null, key: string, x: number, y: number) => void;
type Jump = (hour: number | (() => number | null)) => ReturnType<typeof rowJumpProps>;
const laneInteractive = (item: ResearchItem) => item.kind !== 'opened' && item.kind !== 'closed';
const rowIndex = (row: ResearchRow) => row.kind === 'completed' ? row.entry.databaseIndex : null;

function Art({ icon, className }: { icon: string | undefined; className: string }) {
  return <span className={className}>{icon ? <img src={icon} alt="" /> : <ArtFallback icon="flask" />}</span>;
}

function tipProps(tip: Tip, untip: () => void, key: string, content: () => TooltipContent | null) {
  return {
    onPointerMove: (event: { clientX: number; clientY: number }) => tip(content, key, event.clientX, event.clientY),
    onPointerLeave: untip,
    onFocus: (event: { currentTarget: Element }) => { const box = event.currentTarget.getBoundingClientRect(); tip(content, key, box.right, box.top); },
    onBlur: untip,
  };
}

function LaneRow({ item, slots, text, dimmed, tip, untip, jump, onOpen }: {
  item: ResearchItem; slots: readonly ResearchSlotHistory[]; text: ResearchText;
  dimmed: (token: string, databaseIndex?: number | null) => boolean;
  tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  if (item.kind === 'gap') return <div className="research-marker is-gap" tabIndex={0} {...tipProps(tip, untip, item.key, () => gapTooltip(item))}>
    <span>Not recorded<br />{researchDateRange(item.hour, item.lastHour)}</span></div>;
  if (item.kind === 'opened' || item.kind === 'closed') return <div className={`research-marker is-${item.kind}`}><span>{researchItemDate(item)}</span></div>;
  const { token, databaseIndex } = item.entry;
  const name = text.name(token, databaseIndex), date = researchItemDate(item);
  const className = `research-cell${dimmed(token, databaseIndex) ? ' is-dimmed' : ''}`;
  return <button type="button" className={className} aria-label={`${name}, completed ${date}, show time breakdown`}
    onClick={event => { const target = event.currentTarget; untip(); onOpen({ slotIndex: item.slot.index, entry: item.entry, hour: item.hour }, () => target.isConnected ? target.getBoundingClientRect() : null); }}
    {...tipProps(tip, untip, item.key, () => completedTooltip(completedRow(slots, item.slot, item.entry, item.hour), text))}
    {...jump(item.hour)}><Art icon={text.icon(token)} className="research-cell-art" /><span className="research-cell-name">{name}</span><small>{date}</small></button>;
}

function NowCell({ lane, history, now, text, tip, untip, jump, onOpen, latest }: {
  lane: ResearchLane; history: ResearchData['history']; now: ResearchSlotsAt | undefined; text: ResearchText;
  tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
  latest: { current: ResearchSlotsAt | undefined; inspect?: (slot: number) => SlotInspection | undefined };
}) {
  const { slot } = lane;
  const read = now?.slots.find(item => item.index === slot.index);
  const full = history.slots.find(owner => owner.index === slot.index);
  if (!read || !now || !full) {
    const closed = lane.items.at(-1)?.kind === 'closed';
    if (!now && !closed) return <div className="research-now-cell" role="status">Loading research slot…</div>;
    return <div className="research-now-cell is-unavailable" role="status"><b>{closed ? 'Closed' : 'Unavailable'}</b>
      {!closed && <small>{now?.missing ?? 'Not recorded at this hour'}</small>}</div>;
  }
  const numbers = slotNumbers(read, now.hour);
  const entry = researchEntryAt(full, read.state, now.hour);
  if (!entry && !isUnjoinedResearch(read.state)) {
    return <div className="research-now-cell is-idle" role="status" data-research-slot={slot.index}><Art icon={undefined} className="research-now-art is-idle" />
      <b className="research-idle">Idle</b><small data-paint="saved">{numbers.saved}</small></div>;
  }
  const token = entry?.token ?? read.state.token!;
  const databaseIndex = entry?.databaseIndex ?? null;
  const name = text.name(token, databaseIndex);
  const content = () => {
    const shown = latest.inspect?.(slot.index);
    if ((shown ? shown.token : (latest.current ?? now).slots.find(item => item.index === slot.index)?.state.token) !== read.state.token) return null;
    if (shown) return currentTooltip(token, slot.index, shown.fraction === null ? null : { fraction: shown.fraction, hour: shown.hour }, shown.left, !!entry, text, null, databaseIndex);
    const reply = latest.current ?? now, current = reply.slots.find(item => item.index === slot.index) ?? read;
    const progress = researchLatestProgress({ ...full, states: [current.state], progress: current.progress ? [current.progress] : [] }, current.state, reply.hour);
    return currentTooltip(token, slot.index, progress, slotNumbers(current, reply.hour).left, !!entry, text, null, databaseIndex);
  };
  const body = <>
    <Art icon={text.icon(token)} className="research-now-art" /><b>{name}</b>
    {numbers.fraction !== null && <small className="is-progress" data-paint="percent">{numbers.percent}</small>}
    {numbers.fraction !== null && <span className="research-now-meter"><i data-paint="bar" style={{ width: `${numbers.fraction * 100}%` }} /></span>}
  </>;
  const tips = tipProps(tip, untip, `now:${slot.index}`, content);
  return entry
    ? <button type="button" className="research-now-cell" data-research-slot={slot.index} aria-label={`${name}, researching in slot ${slot.index + 1}, show time breakdown`} {...tips}
      {...jump(() => { const slots = researchAt(history, latest.current?.hour ?? now.hour); const at = slots.find(owner => owner.index === slot.index);
        return at ? researchJourney(slots, at, entry).firstHour : null; })}
      onClick={event => { const target = event.currentTarget; untip(); onOpen({ slotIndex: slot.index, entry, hour: latest.current?.hour ?? now.hour }, () => target.isConnected ? target.getBoundingClientRect() : null); }}>{body}</button>
    : <div className="research-now-cell" data-research-slot={slot.index} tabIndex={0} aria-label={`${name}, researching in slot ${slot.index + 1}`} {...tips}>{body}</div>;
}

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
  const actions = useContext(HeaderActionsSlot);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'list' ? 'list' : 'columns';
  const { track, tag, data, error, retry } = useResearchReply(widget, runtime, active);
  const following = widget.time.mode === 'follow';
  const moment = following || widget.time.mode === 'pin';
  const revealed = useCallback((axis: number) => {
    if (!data || !track) return '';
    const hour = Math.floor(axis) + track.axisOffset;
    return hour < track.branch.firstHour || hour > track.branch.lastHour ? 'outside' : researchRevealKey(data.history, hour);
  }, [data, track]);
  const headHour = useHeadHour(runtime.clock, active && following, revealed);
  const window = track && resolveWindow(widget, track, runtime.tracks, headHour);
  const hour = window ? Math.floor(window.lastHour) : undefined;
  const root = useRef<HTMLElement | null>(null);
  const now = useResearchSlots(widget, runtime, active && moment && !!data, slotsStructure, root);
  const { latest } = now;
  const { categories } = useChartTokens();
  const slots = useMemo(() => data && hour !== undefined ? researchAt(data.history, hour) : null, [data, hour]);
  const coverage = useMemo(() => data && hour !== undefined ? coverageAt(data.history.coverage, hour) : NO_COVERAGE, [data, hour]);
  const lanes = useMemo(() => data && hour !== undefined ? researchLanesAt(data.history, hour) : NO_LANES, [data, hour]);
  const looks = useMemo(() => data ? researchFolderLooks(data, historyTokens(data.history), categories) : NO_LOOKS, [data, categories]);
  const folders = useMemo(() => researchFolderList(looks), [looks]);
  const chosenTokens = useMemo(() => contentSet(options.folders), [options.folders]);
  const chosen = useMemo(() => new Set(chosenTokens.map(contentKey)), [chosenTokens]);
  const dimmed = useCallback((token: string, databaseIndex?: number | null) =>
    chosen.size > 0 && !chosen.has(researchFolderAt(looks, token, databaseIndex)?.key ?? ''), [chosen, looks]);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const includeRow = useCallback((row: ResearchRow) => row.kind === 'gap' ? sort.column === 'date' : !dimmed(row.token, rowIndex(row)), [dimmed, sort.column]);
  const rows = useMemo(() => view === 'list' && slots ? researchRows(lanes, slots, coverage).filter(includeRow) : [], [view, slots, lanes, coverage, includeRow]);
  const requested = perfLog && track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour;
  const keys = (shown: readonly ResearchLane[]) => shown.flatMap(lane => [
    'slot:' + lane.slot.index + ':' + lane.offset, ...lane.items.map(item => lane.slot.index + ':' + item.key),
  ]);
  useRecordedRows('research history', data && typeof requested === 'number' ? {
    hour: requested, rows: () => view === 'columns' ? keys(lanes) : rows.map(row => row.key),
    exact: async () => {
      const observed = recordedResearchLanes(data.history, requested);
      return view === 'columns' ? keys(observed) : researchRows(observed, researchAt(data.history, requested),
        coverageAt(data.history.coverage, requested)).filter(includeRow).map(row => row.key);
    },
  } : undefined, now.placement);
  const imageTokens = useMemo(() => data ? [...new Set(historyTokens(data.history).map(use => typeof use === 'string' ? use : use[0]))] : NO_TOKENS, [data]);
  const recorded = useMemo(() => researchRecordedTemplates(data?.history), [data]);
  const images = useTechnologyArtwork(runtime.client, track, imageTokens, active && !!data, recorded);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...(now.data?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, now.data, images.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const artwork = useMemo(() => researchArtwork(data, images.artwork), [data, images.artwork]);
  const text = useMemo<ResearchText>(() => ({
    name: (token, databaseIndex) => contentTitle(researchArt(artwork, token, databaseIndex)?.name, token, 'Technology name unavailable'),
    icon: token => artwork.get(token)?.icon ?? undefined,
    artworkFailure: token => images.failures?.get(token),
    folder: (token, databaseIndex) => researchFolderAt(looks, token, databaseIndex),
  }), [artwork, looks, images.failures]);
  const tips = useOpenTooltip(owner, now.live.subscribe);
  const tip = useCallback<Tip>((content, key, x, y) => tips.show(content, key, x, y), [tips]);
  const untip = tips.hide;
  const breakdown = usePopover('start');
  const [shown, setShown] = useState<Breakdown | null>(null);
  const open = useCallback((target: Breakdown, anchor: () => AnchorRect | null) => {
    const displayed = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour;
    if (displayed === undefined || target.hour > displayed) return;
    const observation = target.entry.outcomeObservedHour;
    if (observation != null && observation > displayed) return;
    setShown({ ...target, hour: Math.max(target.hour, observation ?? target.hour) });
    breakdown.openAt(anchor);
  }, [breakdown, runtime, track, widget]);
  const jump = useCallback<Jump>(at => track ? rowJumpProps(runtime, track.id, at, menu.open) : {}, [runtime, track, menu.open]);
  const completed = useMemo(() => lanes.reduce((total, lane) => total + lane.items.filter(item => item.kind === 'completed').length, 0), [lanes]);
  const slot = actions ?? headerSlot;
  const header = <>
    {slot && createPortal(<Segmented value={view} options={VIEWS} onChange={value => setOption?.('view', value)} label="Research view" />, slot)}
    {headerSlot && completed > 0 && createPortal(<span className="pill">{usNumber.format(completed)} completed</span>, headerSlot)}
  </>;
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (hour === undefined || hour < track.branch.firstHour || hour > track.branch.lastHour) return <>{header}<div className="dash-empty" role="status">This time is outside the recording.</div></>;
  const failure = error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : null;
  if (failure && !data) return <>{header}{failure}</>;
  if (!data || !slots) return <>{header}<ListSkeleton label="Loading research…" /></>;
  if (!lanes.length) return <>{header}{failure}<div className="dash-empty" role="status"><span className="empty-icon"><Icon name="history" /></span>
    <p>No research slots recorded by {researchDate(hour)}.</p></div></>;
  const detailHour = shown && Math.min(shown.hour, following ? runtime.clock.getHour() + track.axisOffset : hour);
  const detailSlots = detailHour === null ? null : researchAt(data.history, detailHour);
  const shownSlot = shown ? detailSlots?.find(item => item.index === shown.slotIndex) : undefined;
  const shownEntry = shownSlot?.entries.find(entry => entry.firstObservedHour === shown!.entry.firstObservedHour && entry.token === shown!.entry.token);
  const choices = folders.map(folder => ({ value: setToken(folder.key), label: folder.name }));
  const toggle = (token: string) => { const next = toggleContentSet(options.folders, token, Number.MAX_SAFE_INTEGER); if (next != null) setOption?.('folders', next); };
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="research-widget" aria-label="Research history"
    ref={now.root}>
    {header}
    {failure}
    {now.error && moment && <WidgetState kind="error" failure={now.error} onRetry={now.retry} />}
    {choices.length > 0 && <div className="research-toolbar">
      <ContentSetPicker label="Research folders" automatic="All folders" choices={choices} selected={chosenTokens} onToggle={toggle}
        onReset={() => { if (chosenTokens.length) setOption?.('folders', ''); }} trigger={<FolderTrigger folders={folders} chosen={chosen} />}
        triggerClassName="button small research-folder-button" />
    </div>}
    {view === 'columns'
      ? <ResearchColumns lanes={lanes} slots={slots} hour={hour} moment={moment} now={now.data} latest={latest} text={text} dimmed={dimmed} tip={tip} untip={untip} jump={jump} onOpen={open} data={data} />
      : <ResearchList rows={rows} text={text} looks={looks}
        sort={sort} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        setOption={setOption} menu={row => row.kind === 'gap' ? [] : [jumpItem(runtime, track.id, row.hour)]}
        onOpen={open} />}
    <Popover menu={breakdown} className="research-popover" aria-label="Research time breakdown">
      {breakdown.open && shownSlot && shownEntry && <>
        <PopoverHeading title={text.name(shownEntry.token, shownEntry.databaseIndex)} menu={breakdown} />
        <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={detailHour!} history={data.history} slots={detailSlots!} slot={shownSlot} entry={shownEntry}
          description={researchArt(artwork, shownEntry.token, shownEntry.databaseIndex)?.description} />
      </>}
    </Popover>
    {menu.menu}
  </section>;
}

function ResearchColumns({ lanes, slots, hour, moment, now, latest, text, dimmed, tip, untip, jump, onOpen, data }: {
  lanes: readonly ResearchLane[]; slots: readonly ResearchSlotHistory[]; hour: number; moment: boolean; now: ResearchSlotsAt | undefined;
  latest: { current: ResearchSlotsAt | undefined; inspect?: (slot: number) => SlotInspection | undefined }; text: ResearchText;
  dimmed: (token: string) => boolean; tip: Tip; untip: () => void; jump: Jump; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
  data: ResearchData;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  return <div className="research-scroll" ref={scroll}>
    <div className="research-lanes" style={{ '--research-slots': lanes.length } as CSSProperties}>
      {lanes.map(lane => <div className="research-lane-head" key={`head:${lane.slot.index}`}>Slot {lane.slot.index + 1}</div>)}
      {lanes.map(lane => <VirtualList key={`lane:${lane.slot.index}`} identity={data.history} items={lane.items} itemKey={item => item.key}
        estimate={32} className="research-lane" flowWhenFits style={{ padding: undefined, marginTop: `calc(${lane.offset} * var(--research-row))` }}
        label={`Slot ${lane.slot.index + 1}`} scrollRef={scroll} retainExpansion={false} isInteractive={laneInteractive}>
        {item => <LaneRow item={item} slots={slots} text={text} dimmed={dimmed} tip={tip} untip={untip} jump={jump} onOpen={onOpen} />}
      </VirtualList>)}
      {moment && <>
        <div className="research-now-label">Now · <span data-paint="date">{researchDate(now?.hour ?? hour)}</span></div>
        {lanes.map(lane => <div className="research-now-slot" key={`now:${lane.slot.index}`}>
          <NowCell lane={lane} history={data.history} now={now} latest={latest} text={text} tip={tip} untip={untip} jump={jump} onOpen={onOpen} /></div>)}
      </>}
    </div>
  </div>;
}

function ResearchList({ rows, text, looks, sort, density, narrow, setOption, menu, onOpen }: {
  rows: ResearchRow[]; text: ResearchText;
  looks: ResearchFolderLooks; sort: { column: string; direction: 'ascending' | 'descending' };
  density: 'comfortable' | 'compact'; narrow: NarrowTable; setOption: WidgetProps['setOption'];
  menu: (row: ResearchRow) => ReturnType<typeof jumpItem>[]; onOpen: (target: Breakdown, anchor: () => AnchorRect | null) => void;
}) {
  const pointer = useRef<{ x: number; y: number } | null>(null);
  const columns = useMemo<readonly TableColumn<ResearchRow>[]>(() => [
    { key: 'date', label: 'Date', header: 'Date', width: 'max-content', value: row => row.hour, renderMissing: true,
      render: row => <span className="research-list-date">{row.kind === 'gap' ? 'Not recorded' : researchItemDate(row)}</span> },
    { key: 'technology', label: 'Technology', header: 'Technology', width: 'minmax(var(--table-name-min), 1fr)',
      value: row => row.kind === 'gap' ? null : text.name(row.token, rowIndex(row)), renderMissing: true,
      render: row => row.kind === 'gap' ? <span className="research-list-gap">{researchDateRange(row.hour, row.lastHour)}</span>
        : <span className="research-list-tech"><Art icon={text.icon(row.token)} className="research-list-art" /><span>{text.name(row.token, rowIndex(row))}</span></span> },
    { key: 'folder', label: 'Folder', header: 'Folder', width: 'minmax(6em, 0.6fr)', priority: 2,
      value: row => row.kind === 'gap' ? null : researchFolderAt(looks, row.token, rowIndex(row))?.name ?? null, renderMissing: true,
      render: row => { const look = row.kind === 'gap' ? undefined : researchFolderAt(looks, row.token, rowIndex(row));
        return look ? <span className="research-list-folder"><i style={{ background: look.colour }} />{look.name}</span> : null; } },
    { key: 'slot', label: 'Slot', header: 'Slot', numeric: true, width: 'max-content', priority: 3, value: row => row.kind === 'gap' ? null : row.slot.index + 1, renderMissing: true,
      render: row => row.kind === 'gap' ? null : row.slot.index + 1 },
    { key: 'took', label: 'Took', header: 'Took', numeric: true, width: 'max-content', priority: 1,
      text: row => row.kind === 'gap' || row.tookHours === null ? null : researchDays(row.tookHours),
      value: row => row.kind === 'completed' ? row.tookHours : null, renderMissing: true,
      render: row => row.kind === 'gap' ? null : row.tookHours === null ? <span className="research-list-missing">—</span> : researchDays(row.tookHours) },
  ], [text, looks]);
  const tooltip = useCallback((row: ResearchRow) => row.kind === 'completed' ? completedTooltip(row, text) : gapTooltip(row), [text]);
  const click = (row: ResearchRow) => {
    if (row.kind !== 'completed') return;
    const at = pointer.current, focused = document.activeElement;
    pointer.current = null;
    const box: AnchorRect | null = at ? { left: at.x, right: at.x, top: at.y, bottom: at.y }
      : focused instanceof HTMLElement ? focused.getBoundingClientRect() : null;
    onOpen({ slotIndex: row.slot.index, entry: row.entry, hour: row.hour }, () => box);
  };
  const body: ReactNode = rows.length
    ? <Table rowRevision={row => [row, columns]} label="Research history" rows={rows} columns={columns} rowKey={row => row.key} sort={sort} setOption={setOption} narrow={narrow}
      cellBars={false} rankMarkers={false} density={density} muted={row => row.kind === 'gap'} onRowClick={click} menu={menu} tooltip={tooltip} />
    : <div className="dash-empty" role="status">No research in these folders by this date.</div>;
  return <div className="research-list" onPointerDownCapture={event => { pointer.current = { x: event.clientX, y: event.clientY }; }}>{body}</div>;
}
