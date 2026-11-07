import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { useContextMenu } from '../../components/ContextMenu';
import { rowJumpProps } from '../../charts/jump';
import { VirtualList } from '../../components/VirtualList';
import WidgetState from '../../components/WidgetState';
import ListSkeleton from '../../components/Skeleton';
import { CurrentFocus, FocusArt, FocusCompletion, FocusFailureNotices, isCompletedFocus } from './NationalFocusWidget.tsx';
import type { FocusListItem } from './NationalFocusWidget';
import { useFocusArtwork } from './useFocusArtwork';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { inFrame } from '../../dashboard/frameUpdates';
import { countWork, measuredWidget } from '../../dashboard/widgetPerf';
import { nameKey, isFailedFact } from '../../dashboard/countryFacts.ts';
import type { FocusEntry } from '../../generated/dashboard';
import { lastPage, loadPreparedFocusPage, preparedArtKey, validatePreparedHour, canCarryPreparedFocus, loadPreparedFocusDetail, validateUnavailableFocus } from './preparedFocusData';
import { activeFocus } from './focusData';
import type { FocusPageReads, FocusPreparedClient, PreparedFocusArt, PreparedFocusSnapshot } from './preparedFocusData';

import { usePreparedFocusParity } from './usePreparedFocusParity';

const NO_WANTED: readonly { tag: string; hour: number }[] = [];
type Snapshot = PreparedFocusSnapshot;
/** Repeated completions may share artwork, but never disclosure state. */
const disclosureKey = (ordinal: number, artKey: string) => JSON.stringify([ordinal, artKey]);
export default function PreparedFocusWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const requested = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(requested?.lastHour ?? 0);
  const live = widget.time.mode === 'follow';
  const moment = live || widget.time.mode === 'pin';
  const contentRevision = useContentRevision(runtime.client);
  const identity = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, contentRevision]);
  const [selection, setSelection] = useState<{ identity: string; page: bigint } | null>(null);
  const selected = selection?.identity === identity ? selection.page : null;
  const [answer, setAnswer] = useState<{ identity: string; client: typeof runtime.client; hour: number; snapshot: Snapshot | null; pending: boolean; error: Failure | null; warnings: string[] } | null>(null);
  const presented = useRef<{ identity: string; client: typeof runtime.client; snapshot: Snapshot } | null>(null);
  if (presented.current && (presented.current.identity !== identity || presented.current.client !== runtime.client)) presented.current = null;
  const [retry, setRetry] = useState(0);
  const [opened, setOpened] = useState<{ identity: string; client: typeof runtime.client; slot: number; countryId: number; rows: Set<string> } | null>(null);
  useEffect(() => { setOpened(null); }, [identity, runtime.client]);
  const menu = useContextMenu('National focus actions');
  const inRange = !!track && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  // Playback moves the hour without cancelling: the running read lands, then the newest hour is read.
  const lane = JSON.stringify([identity, selected === null ? null : String(selected), live, retry]);
  const flight = useRef<{ lane: string; hour: number; running: boolean; shown: boolean; controller: AbortController } | null>(null);
  const wanted = useRef(hour); wanted.current = hour;
  const measured = measuredWidget();
  const [landed, setLanded] = useState(0);
  // What the last page read, which any hour of this country, recording and content reuses; a retry reads afresh.
  const pageReads = useRef<{ owner: string; client: typeof runtime.client; reads: FocusPageReads } | null>(null);
  useEffect(() => {
    if (!active || !track || !tag || !inRange) return;
    // A read that has shown its hour gives way to a moved one, even while it still prepares.
    if (flight.current?.lane === lane && flight.current.running && (!flight.current.shown || flight.current.hour === hour)) return;
    flight.current?.controller.abort();
    const controller = new AbortController();
    const mine = { lane, hour, running: true, shown: false, controller };
    flight.current = mine;
    const prior = presented.current;
    const crossesGap = (snapshot: Snapshot) => track.branch.gaps.some(gap => gap.firstHour <= Math.max(hour, snapshot.displayHour) && gap.lastHour >= Math.min(hour, snapshot.displayHour));
    let generation = 0, lastRow = '', presentation: Promise<void> = Promise.resolve();
    // Every answer of this read renders with the next tick while playing, in the order set.
    const render = (update: () => void, own?: number) => inFrame(() => {
      if (!controller.signal.aborted && (own === undefined || own === generation)) update();
    });
    render(() => setAnswer({ identity, client: runtime.client, hour,
      snapshot: prior && canCarryPreparedFocus(prior.snapshot.hour, hour) && !crossesGap(prior.snapshot) ? prior.snapshot : null,
      pending: true, error: null, warnings: [] }));
    const client = runtime.client as FocusPreparedClient;
    const readsOwner = JSON.stringify([identity, retry]);
    const landedHour = () => { mine.shown = true; if (wanted.current !== hour) setLanded(value => value + 1); };
    countWork(measured, 'read');
    const show = async (row: ReturnType<typeof validatePreparedHour>, own: number) => {
      if (!row) {
        const warnings = validateUnavailableFocus(await client.preparedQuery!(track.source, 'focus-unavailable', { branchId: track.branch.id, hour }, { signal: controller.signal }), hour);
        if (!controller.signal.aborted && own === generation) render(() => {
          countWork(measured, 'reply');
          setAnswer({ identity, client: runtime.client, hour, snapshot: null, pending: false, error: null, warnings }); landedHour();
        }, own);
        return;
      }
      if (!controller.signal.aborted && prior && !canCarryPreparedFocus(prior.snapshot.hour, row)) {
        presented.current = null;
        render(() => setAnswer({ identity, client: runtime.client, hour, snapshot: null, pending: true, error: null, warnings: [] }), own);
      }
      const page = selected == null ? live ? lastPage(row.completed) : 0n : selected > lastPage(row.completed) ? lastPage(row.completed) : selected;
      const kept = pageReads.current?.owner === readsOwner && pageReads.current.client === runtime.client ? pageReads.current.reads : undefined;
      const { reads, ...data } = await loadPreparedFocusPage(client, track.source, track.branch.id, tag, hour, row, page, controller.signal, kept);
      if (controller.signal.aborted || own !== generation) return;
      pageReads.current = { owner: readsOwner, client: runtime.client, reads };
      const snapshot = { ...data, hour: row, page };
      presented.current = { identity, client: runtime.client, snapshot };
      render(() => {
        countWork(measured, 'reply');
        setOpened(previous => previous && (previous.identity !== identity || previous.client !== runtime.client || previous.slot !== row.slot || previous.countryId !== row.countryId) ? null : previous);
        setAnswer({ identity, client: runtime.client, hour, snapshot, pending: false, error: null, warnings: [] });
        landedHour();
      }, own);
    };
    // A failed seek must not reuse another hour's page and progress.
    const failed = (error: unknown, own = generation) => {
      if (controller.signal.aborted || own !== generation) return;
      render(() => {
        setAnswer(previous => ({ identity, client: runtime.client, hour,
          snapshot: previous?.snapshot && previous.snapshot.hour.at === hour ? previous.snapshot : null, pending: false, error: asFailure(error), warnings: [] }));
        landedHour();
      }, own);
    };
    // The final reply waits for the presentation an equal partial reply started.
    const present = (value: unknown): Promise<void> => {
      let row: ReturnType<typeof validatePreparedHour>;
      try { row = validatePreparedHour(value, hour); } catch (error) { failed(error); return presentation; }
      const key = JSON.stringify(row);
      if (lastRow === key) return presentation;
      lastRow = key;
      const own = ++generation;
      presentation = show(row, own).catch(error => failed(error, own));
      return presentation;
    };
    void preparedSeriesQuery(client, track.source, 'focus-hour', { branchId: track.branch.id, tag, hour }, controller.signal, retry > 0,
      undefined, undefined, value => { void present(value); }).then(present).catch(error => failed(error)).finally(() => {
      mine.running = false;
      if (!controller.signal.aborted && wanted.current !== hour) render(() => setLanded(value => value + 1));
    });
  }, [runtime.client, track, tag, hour, identity, selected, active, inRange, retry, live, landed]);
  // Leaving this lane, the screen or the recording cancels its read.
  useEffect(() => () => { flight.current?.controller.abort(); flight.current = null; }, [lane, active, inRange, runtime.client, track]);
  // An answer for another hour is still pending for this one, keeps its snapshot where it may carry
  // to this hour, or while following playback forward, where it is the answer of its own hour until
  // the next lands; and keeps its failure to itself.
  const held = answer?.identity === identity && answer.client === runtime.client ? answer : null;
  const following = live && runtime.clock.getSnapshot().playing;
  const carries = (snapshot: Snapshot) => (canCarryPreparedFocus(snapshot.hour, hour) || following && hour > snapshot.displayHour)
    && !(track?.branch.gaps ?? []).some(gap => gap.firstHour <= Math.max(hour, snapshot.displayHour) && gap.lastHour >= Math.min(hour, snapshot.displayHour));
  const shown = !held ? null : held.hour === hour ? held : { ...held, pending: true, error: null,
    snapshot: held.snapshot && carries(held.snapshot) ? held.snapshot : null };
  const snapshot = shown?.snapshot;
  const detailEntries = useMemo(() => opened?.identity === identity && opened.client === runtime.client && opened.slot === snapshot?.hour.slot && opened.countryId === snapshot?.hour.countryId
    ? snapshot.order.filter(item => opened.rows.has(disclosureKey(item.ordinal, preparedArtKey('national', item.completedByHour ?? snapshot.hour.orderFirstHour ?? snapshot.hour.at, item.token)))) : [], [opened, identity, runtime.client, snapshot]);
  const detailOwner = JSON.stringify([identity, snapshot?.hour.slot, snapshot?.hour.countryId, snapshot?.page.toString()]);
  const detailKeys = JSON.stringify(detailEntries.map(item => preparedArtKey('national', item.completedByHour ?? snapshot!.hour.orderFirstHour ?? snapshot!.hour.at, item.token)));
  type DetailResult = { data: PreparedFocusArt | null; pending: boolean; error: Failure | null };
  const [preparedDetail, setPreparedDetail] = useState<{ owner: string; client: typeof runtime.client; rows: Map<string, DetailResult> } | null>(null);
  const detailResults = preparedDetail?.owner === detailOwner && preparedDetail.client === runtime.client ? preparedDetail.rows : undefined;
  useEffect(() => {
    if (!active || !track || !tag || !detailEntries.length || !snapshot) { setPreparedDetail(null); return; }
    const controller = new AbortController();
    let current = true;
    const wanted = new Map(detailEntries.map(entry => [preparedArtKey('national', entry.completedByHour ?? snapshot.hour.orderFirstHour ?? snapshot.hour.at, entry.token), entry]));
    const rows = new Map([...wanted.keys()].map(key => [key, detailResults?.get(key) ?? { data: null, pending: true, error: null }]));
    setPreparedDetail({ owner: detailOwner, client: runtime.client, rows });
    const results = new Map(rows);
    void Promise.all([...wanted].map(async ([key, entry]) => {
      if (!rows.get(key)!.pending) return;
      try {
        const data = await loadPreparedFocusDetail(runtime.client as FocusPreparedClient, track.source, track.branch.id, tag, snapshot.hour, entry, controller.signal);
        results.set(key, { data, pending: false, error: null });
      } catch (error) { results.set(key, { data: null, pending: false, error: asFailure(error) }); }
    })).then(() => { if (current) setPreparedDetail({ owner: detailOwner, client: runtime.client, rows: results }); });
    return () => { current = false; controller.abort(); };
  }, [runtime.client, track, tag, detailOwner, detailKeys, active]);
  const fallbackDetails = (key: string) => {
    const result = detailResults?.get(key);
    return result?.pending === false && result.data === null && result.error === null;
  };
  const detailsWanted = useMemo(() => detailEntries.filter(entry => fallbackDetails(preparedArtKey('national', entry.completedByHour ?? snapshot!.hour.orderFirstHour ?? snapshot!.hour.at, entry.token)))
    .map(entry => ({ tag: entry.token, hour: entry.completedByHour ?? snapshot!.hour.orderFirstHour ?? snapshot!.hour.at })), [detailEntries, snapshot, detailResults]);
  // A description focus art did not return keeps the dated oracle for opened rows.
  const details = useFocusArtwork(runtime.client, track, tag, NO_WANTED, active && inRange, detailsWanted);
  // The dated fallback keys a completion's description by its hour and token alone, as completions are national.
  const detailRead = (key: string, entry: FocusEntry) => fallbackDetails(key)
    ? details.details?.get(nameKey(entry.completedByHour ?? snapshot!.hour.orderFirstHour ?? snapshot!.hour.at, entry.token)) : detailResults?.get(key)?.data;
  const detailPending = (key: string, entry: FocusEntry) => fallbackDetails(key) ? detailRead(key, entry) === undefined : detailResults?.get(key)?.pending !== false;
  const descriptions = useRef<{ owner: string; client: typeof runtime.client; texts: Map<string, string> } | null>(null);
  if (descriptions.current?.owner !== detailOwner || descriptions.current.client !== runtime.client) descriptions.current = { owner: detailOwner, client: runtime.client, texts: new Map() };
  const warnings = useMemo(() => snapshot ? [...(shown?.warnings ?? []), ...snapshot.warnings, ...details.warnings,
    ...[...detailResults?.values() ?? []].flatMap(item => item.error ? [item.error.message] : item.data?.reason ? [item.data.reason] : []),
    ...[...details.details?.values() ?? []].flatMap(item => item && !isFailedFact(item) && item.reason ? [item.reason] : []),
    ...[...snapshot.art.values()].flatMap(item => item.reason ? [item.reason] : [])] : shown?.warnings ?? [], [snapshot, details.warnings, details.details, detailResults, shown?.warnings]);
  useDiagnostics(onDiagnostics, warnings);
  const rows = useMemo<FocusListItem[]>(() => {
    const rows: FocusListItem[] = snapshot?.order.map(entry => ({ kind: 'completed', entry })) ?? [];
    if (!moment && snapshot?.hour.current && snapshot.page === lastPage(snapshot.hour.completed)) rows.push({ kind: 'unfinished', token: snapshot.hour.current });
    return rows;
  }, [snapshot, moment]);
  const list = useRef<HTMLDivElement>(null);
  const listIdentity = useMemo(() => ({}), [detailOwner, runtime.client]);
  // Following playback must reveal new completions even when they start a new page.
  const newest = live && snapshot && snapshot.page === lastPage(snapshot.hour.completed) ? snapshot.hour.completed.rows : null;
  const seen = useRef<string | null>(null);
  useEffect(() => {
    const before = seen.current, element = list.current;
    seen.current = newest;
    if (newest === null || !element || BigInt(newest) <= BigInt(before ?? '0')) return;
    element.scrollTo({ top: element.scrollHeight, behavior: document.documentElement.dataset.motion === 'reduced' ? 'auto' : 'smooth' });
  }, [newest]);
  const openedOrdinals = useMemo(() => detailEntries.map(entry => entry.ordinal), [detailEntries]);
  const paritySnapshot = import.meta.env.MODE === 'parity' ? useMemo(() => snapshot && !shown?.pending ? { ...snapshot,
    details: new Map(detailEntries.flatMap(entry => {
      const read = detailRead(preparedArtKey('national', entry.completedByHour ?? snapshot.hour.orderFirstHour ?? snapshot.hour.at, entry.token), entry);
      return read && !isFailedFact(read) ? [[entry.ordinal, read] as const] : [];
    })),
  } : undefined, [snapshot, shown?.pending, detailEntries, detailResults, details.details]) : undefined;
  const parityReady = !!shown && !shown.pending && detailEntries.every(entry => !detailPending(preparedArtKey('national', entry.completedByHour ?? snapshot!.hour.orderFirstHour ?? snapshot!.hour.at, entry.token), entry));
  const parityRoot = import.meta.env.MODE === 'parity' ? usePreparedFocusParity(runtime.client as FocusPreparedClient, track?.source, track?.branch.id, tag, hour, moment, identity, active && inRange && parityReady, openedOrdinals, paritySnapshot) : undefined;
  if (!track || !tag) return <div className="dash-empty" role="status">Load a campaign and choose a country to see its national focuses.</div>;
  if (!inRange) return <div className="dash-empty" role="status">This time is outside the recording.</div>;
  const failure = shown?.error ? <WidgetState kind="error" failure={shown.error} onRetry={() => setRetry(value => value + 1)} /> : null;
  if (failure && !snapshot) return <div ref={parityRoot}>{failure}</div>;
  if (!shown || shown.pending && !snapshot) return <div ref={parityRoot}><ListSkeleton label="Loading national focuses…" /></div>;
  if (!snapshot) return <div ref={parityRoot} className="dash-empty" role="status">No complete focus data recorded at this date.</div>;
  const row = snapshot.hour;
  const current = activeFocus(row);
  const currentArt = current?.token ? snapshot.art.get(preparedArtKey(current.database, snapshot.displayHour, current.token)) : undefined;
  const finalPage = lastPage(row.completed);
  const changePage = (page: bigint) => { setSelection({ identity, page }); setOpened(null); };
  return <section ref={parityRoot} className="national-focus prepared-focus" aria-label="National focus" aria-busy={shown.pending}>
    {headerSlot && createPortal(<span className="pill focus-header-count">{row.completed.rows} completed</span>, headerSlot)}
    {failure}
    {moment && <CurrentFocus order={{ completed: [], current: row.current, continuous: row.continuous, paused: row.paused }} art={currentArt?.artwork}
      state={{ progress: BigInt(row.progressRaw), dailyGain: row.dailyGainRaw === null ? null : BigInt(row.dailyGainRaw) }} live={live} reason={currentArt?.reason ?? undefined} failure={currentArt?.failure} titleFailure={currentArt?.titleFailure} pending={active && shown.pending && !currentArt} />}
    {finalPage > 0n && <nav className="focus-page-controls" aria-label="Completion pages">
      <button type="button" disabled={snapshot.page === 0n || shown.pending} onClick={() => changePage(0n)}>First</button>
      <button type="button" disabled={snapshot.page === 0n || shown.pending} onClick={() => changePage(snapshot.page - 1n)}>Previous</button>
      <span role="status">Page {String(snapshot.page + 1n)} of {BigInt(row.completed.count) === 0n ? '1' : row.completed.count}</span>
      <button type="button" disabled={snapshot.page >= finalPage || shown.pending} onClick={() => changePage(snapshot.page + 1n)}>Next</button>
      <button type="button" disabled={snapshot.page >= finalPage || shown.pending} onClick={() => changePage(finalPage)}>Last</button>
    </nav>}
    <div className="focus-scroll" ref={list}>
      {rows.length ? <VirtualList identity={listIdentity} retainExpansion={false} items={rows} itemKey={item => item.kind === 'completed'
        ? `${item.entry.ordinal}:${item.entry.completedByHour ?? row.orderFirstHour ?? row.at}` : `unfinished:${item.token}`} estimate={62} label="Completed focuses in order" className="focus-list" scrollRef={list}
        isInteractive={isCompletedFocus} rowProps={item => item.kind === 'unfinished' ? { className: 'focus-entry is-unfinished' }
          : ({ className: `focus-entry${live && BigInt(item.entry.ordinal) === BigInt(row.completed.rows) - 1n ? ' is-newest' : ''}`, ...rowJumpProps(runtime, track.id, item.entry.completedByHour, menu.open) })}>{item => {
        if (item.kind === 'unfinished') return <div className="focus-row">
          <span className="focus-number" aria-hidden="true">{String(BigInt(row.completed.rows) + 1n).padStart(2, '0')}</span>
          <FocusArt art={currentArt?.artwork} size="small" />
          <span className="focus-label"><strong data-detail={currentArt?.failure?.message} data-tooltip={contentTitleHint(item.token, currentArt?.reason ?? undefined)}>{contentTitle(currentArt?.titleFailure ?? currentArt?.artwork.name, item.token, 'Focus name unavailable', active && shown.pending && !currentArt)}</strong>
            <span>{row.paused ? 'Paused' : 'In progress'}</span></span>
          {currentArt?.failure && <FocusFailureNotices errors={[currentArt.failure]} />}
        </div>;
        const entry = item.entry, key = preparedArtKey('national', entry.completedByHour ?? row.orderFirstHour ?? row.at, entry.token), disclosure = disclosureKey(entry.ordinal, key);
        const art = snapshot.art.get(key), read = detailRead(key, entry);
        const description = read && !isFailedFact(read) ? read.artwork.description ?? art?.artwork.description : art?.artwork.description;
        if (description) descriptions.current!.texts.set(key, description);
        return <FocusCompletion datePreparing={row.preparing?.includes('completedByHour')} entry={entry} art={art?.artwork} reason={art?.reason ?? undefined} failure={art?.failure} titleFailure={art?.titleFailure} failures={[isFailedFact(read) ? read : undefined, detailResults?.get(key)?.error ?? undefined].filter((error): error is Failure => error !== undefined)} pending={active && shown.pending && !art}
          description={description ?? (detailPending(key, entry) ? descriptions.current!.texts.get(key) : null)} open={detailEntries.some(item => item.ordinal === entry.ordinal)}
          onToggle={open => setOpened(previous => {
            const wanted = previous?.identity === identity && previous.client === runtime.client && previous.slot === row.slot && previous.countryId === row.countryId ? previous.rows : new Set<string>();
            if (wanted.has(disclosure) === open) return previous;
            const rows = new Set(wanted); if (open) rows.add(disclosure); else rows.delete(disclosure);
            return { identity, client: runtime.client, slot: row.slot, countryId: row.countryId, rows };
          })} />;
      }}</VirtualList> : <p className="focus-empty">No completed focuses recorded at this point.</p>}
    </div>
    {row.preparing?.length ? <p className="chart-line-key" role="status">Completion dates and acquisition context are still being prepared.</p> : null}
    {menu.menu}
  </section>;
}
