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
import { CurrentFocus, FocusArt, FocusCompletion, isCompletedFocus } from './NationalFocusWidget';
import type { FocusListItem } from './NationalFocusWidget';
import { useFocusArtwork } from './useFocusArtwork';
import { FAILED_FACT } from '../../dashboard/countryFacts';
import { lastPage, loadPreparedFocusPage, preparedArtKey, loadPreparedFocusHour, canCarryPreparedFocus, loadPreparedFocusDetail, validateUnavailableFocus } from './preparedFocusData';
import type { FocusPreparedClient, PreparedFocusArt, PreparedFocusSnapshot } from './preparedFocusData';

import { usePreparedFocusParity } from './usePreparedFocusParity';

const NO_WANTED: readonly { tag: string; hour: number }[] = [];
type Snapshot = PreparedFocusSnapshot;
export default function PreparedFocusWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const requested = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = Math.floor(requested?.lastHour ?? 0);
  const live = widget.time.mode === 'follow';
  const moment = live || widget.time.mode === 'pin';
  const contentRevision = useContentRevision(runtime.client);
  const identity = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, contentRevision]);
  const [selection, setSelection] = useState<{ identity: string; page: bigint } | null>(null);
  const selected = selection?.identity === identity ? selection.page : null;
  const [answer, setAnswer] = useState<{ identity: string; client: typeof runtime.client; snapshot: Snapshot | null; pending: boolean; error: string | null; warnings: string[] } | null>(null);
  const presented = useRef<{ identity: string; client: typeof runtime.client; snapshot: Snapshot } | null>(null);
  if (presented.current && (presented.current.identity !== identity || presented.current.client !== runtime.client)) presented.current = null;
  const [retry, setRetry] = useState(0);
  const [opened, setOpened] = useState<{ identity: string; client: typeof runtime.client; slot: number; countryId: number; rows: Set<string> } | null>(null);
  useEffect(() => { setOpened(null); }, [identity, runtime.client]);
  const menu = useContextMenu('National focus actions');
  const inRange = !!track && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  useEffect(() => {
    if (!active || !track || !tag || !inRange) return;
    const controller = new AbortController();
    let current = true;
    const prior = presented.current;
    const crossesGap = (snapshot: Snapshot) => track.branch.gaps.some(gap => gap.firstHour <= Math.max(hour, snapshot.displayHour) && gap.lastHour >= Math.min(hour, snapshot.displayHour));
    setAnswer({ identity, client: runtime.client,
      snapshot: prior && canCarryPreparedFocus(prior.snapshot.hour, hour) && !crossesGap(prior.snapshot) ? prior.snapshot : null,
      pending: true, error: null, warnings: [] });
    const client = runtime.client as FocusPreparedClient;
    void (async () => {
      const row = await loadPreparedFocusHour(client, track.source, track.branch.id, tag, hour, controller.signal, retry > 0);
      if (!row) {
        const warnings = validateUnavailableFocus(await client.preparedQuery!(track.source, 'focus-unavailable', { branchId: track.branch.id, hour }, { signal: controller.signal }), hour);
        return { snapshot: null, warnings };
      }
      // Only an actual numeric hour establishes the tag's current native holder.
      if (current) {
        setOpened(previous => previous && (previous.identity !== identity || previous.client !== runtime.client || previous.slot !== row.slot || previous.countryId !== row.countryId) ? null : previous);
        if (prior && !canCarryPreparedFocus(prior.snapshot.hour, row)) presented.current = null;
        setAnswer({ identity, client: runtime.client,
          snapshot: prior && canCarryPreparedFocus(prior.snapshot.hour, row) && !crossesGap(prior.snapshot) ? prior.snapshot : null,
          pending: true, error: null, warnings: [] });
      }
      const page = selected == null ? live ? lastPage(row.completed) : 0n : selected > lastPage(row.completed) ? lastPage(row.completed) : selected;
      const data = await loadPreparedFocusPage(client, track.source, track.branch.id, tag, hour, row, page, controller.signal);
      return { snapshot: { ...data, hour: row, page }, warnings: [] };
    })().then(({ snapshot, warnings }) => {
      if (current) {
        presented.current = snapshot ? { identity, client: runtime.client, snapshot } : null;
        setAnswer({ identity, client: runtime.client, snapshot, pending: false, error: null, warnings });
      }
    }, error => {
      if (current) setAnswer({ identity, client: runtime.client, snapshot: null, pending: false, error: error instanceof Error ? error.message : String(error), warnings: [] });
    });
    return () => { current = false; controller.abort(); };
  }, [runtime.client, track, tag, hour, identity, selected, active, inRange, retry, live]);
  const shown = answer?.identity === identity && answer.client === runtime.client ? answer : null;
  const snapshot = shown?.snapshot;
  const detailEntries = useMemo(() => opened?.identity === identity && opened.client === runtime.client && opened.slot === snapshot?.hour.slot && opened.countryId === snapshot?.hour.countryId
    ? snapshot.order.filter(item => opened.rows.has(preparedArtKey(item.completedByHour ?? snapshot.hour.orderFirstHour, item.token))) : [], [opened, identity, runtime.client, snapshot]);
  const detailOwner = JSON.stringify([identity, snapshot?.hour.slot, snapshot?.hour.countryId, snapshot?.page.toString()]);
  const detailKeys = JSON.stringify(detailEntries.map(item => preparedArtKey(item.completedByHour ?? snapshot!.hour.orderFirstHour, item.token)));
  type DetailResult = { data: PreparedFocusArt | null; pending: boolean; error: string | null };
  const [preparedDetail, setPreparedDetail] = useState<{ owner: string; client: typeof runtime.client; rows: Map<string, DetailResult> } | null>(null);
  const detailResults = preparedDetail?.owner === detailOwner && preparedDetail.client === runtime.client ? preparedDetail.rows : undefined;
  useEffect(() => {
    if (!active || !track || !tag || !detailEntries.length || !snapshot) { setPreparedDetail(null); return; }
    const controller = new AbortController();
    let current = true;
    const wanted = new Map(detailEntries.map(entry => [preparedArtKey(entry.completedByHour ?? snapshot.hour.orderFirstHour, entry.token), entry]));
    const rows = new Map([...wanted.keys()].map(key => [key, detailResults?.get(key) ?? { data: null, pending: true, error: null }]));
    setPreparedDetail({ owner: detailOwner, client: runtime.client, rows });
    const results = new Map(rows);
    void Promise.all([...wanted].map(async ([key, entry]) => {
      if (!rows.get(key)!.pending) return;
      try {
        const data = await loadPreparedFocusDetail(runtime.client as FocusPreparedClient, track.source, track.branch.id, tag, snapshot.hour, entry, controller.signal);
        results.set(key, { data, pending: false, error: null });
      } catch (error) { results.set(key, { data: null, pending: false, error: error instanceof Error ? error.message : String(error) }); }
    })).then(() => { if (current) setPreparedDetail({ owner: detailOwner, client: runtime.client, rows: results }); });
    return () => { current = false; controller.abort(); };
  }, [runtime.client, track, tag, detailOwner, detailKeys, active]);
  const fallbackDetails = (key: string) => {
    const result = detailResults?.get(key);
    return result?.pending === false && result.data === null && result.error === null;
  };
  const detailsWanted = useMemo(() => detailEntries.filter(entry => fallbackDetails(preparedArtKey(entry.completedByHour ?? snapshot!.hour.orderFirstHour, entry.token)))
    .map(entry => ({ tag: entry.token, hour: entry.completedByHour ?? snapshot!.hour.orderFirstHour })), [detailEntries, snapshot, detailResults]);
  // A description focus art did not return keeps the dated oracle for opened rows.
  const details = useFocusArtwork(runtime.client, track, tag, NO_WANTED, active && inRange, detailsWanted);
  const detailRead = (key: string) => fallbackDetails(key) ? details.details?.get(key) : detailResults?.get(key)?.data;
  const detailPending = (key: string) => fallbackDetails(key) ? detailRead(key) === undefined : detailResults?.get(key)?.pending !== false;
  const descriptions = useRef<{ owner: string; client: typeof runtime.client; texts: Map<string, string> } | null>(null);
  if (descriptions.current?.owner !== detailOwner || descriptions.current.client !== runtime.client) descriptions.current = { owner: detailOwner, client: runtime.client, texts: new Map() };
  const warnings = useMemo(() => snapshot ? [...(shown?.warnings ?? []), ...snapshot.warnings, ...details.warnings,
    ...[...detailResults?.values() ?? []].flatMap(item => item.error ? [item.error] : item.data?.reason ? [item.data.reason] : []),
    ...[...details.details?.values() ?? []].flatMap(item => item && item !== FAILED_FACT && item.reason ? [item.reason] : []),
    ...[...snapshot.art.values()].flatMap(item => item.reason ? [item.reason] : [])] : shown?.warnings ?? [], [snapshot, details.warnings, details.details, detailResults, shown?.warnings]);
  useDiagnostics(onDiagnostics, warnings);
  const rows = useMemo<FocusListItem[]>(() => {
    const rows: FocusListItem[] = snapshot?.order.map(entry => ({ kind: 'completed', entry })) ?? [];
    if (!moment && snapshot?.hour.current && snapshot.page === lastPage(snapshot.hour.completed)) rows.push({ kind: 'unfinished', token: snapshot.hour.current });
    return rows;
  }, [snapshot, moment]);
  const list = useRef<HTMLDivElement>(null);
  const listIdentity = useMemo(() => ({}), [detailOwner, runtime.client]);
  const openedOrdinals = useMemo(() => detailEntries.map(entry => entry.ordinal), [detailEntries]);
  const paritySnapshot = import.meta.env.MODE === 'parity' ? useMemo(() => snapshot && !shown?.pending ? { ...snapshot,
    details: new Map(detailEntries.flatMap(entry => {
      const read = detailRead(preparedArtKey(entry.completedByHour ?? snapshot.hour.orderFirstHour, entry.token));
      return read && read !== FAILED_FACT ? [[entry.ordinal, read] as const] : [];
    })),
  } : undefined, [snapshot, shown?.pending, detailEntries, detailResults, details.details]) : undefined;
  const parityReady = !!shown && !shown.pending && detailEntries.every(entry => !detailPending(preparedArtKey(entry.completedByHour ?? snapshot!.hour.orderFirstHour, entry.token)));
  const parityRoot = import.meta.env.MODE === 'parity' ? usePreparedFocusParity(runtime.client as FocusPreparedClient, track?.source, track?.branch.id, tag, hour, moment, identity, active && inRange && parityReady, openedOrdinals, paritySnapshot) : undefined;
  if (!track || !tag) return <div className="dash-empty" role="status">Load a campaign and choose a country to see its national focuses.</div>;
  if (!inRange) return <div className="dash-empty" role="status">This time is outside the recording.</div>;
  if (shown?.error) return <div ref={parityRoot}><WidgetState kind="error" detail={shown.error} onRetry={() => setRetry(value => value + 1)} /></div>;
  if (!shown || shown.pending && !snapshot) return <div ref={parityRoot}><ListSkeleton label="Loading national focuses…" /></div>;
  if (!snapshot) return <div ref={parityRoot} className="dash-empty" role="status">No complete focus data recorded at this date.</div>;
  const row = snapshot.hour;
  const current = row.current ?? row.continuous;
  const currentArt = current ? snapshot.art.get(preparedArtKey(snapshot.displayHour, current)) : undefined;
  const finalPage = lastPage(row.completed);
  const changePage = (page: bigint) => { setSelection({ identity, page }); setOpened(null); };
  return <section ref={parityRoot} className="national-focus prepared-focus" aria-label="National focus" aria-busy={shown.pending}>
    {headerSlot && createPortal(<span className="pill focus-header-count">{row.completed.rows} completed</span>, headerSlot)}
    {moment && <CurrentFocus order={{ completed: [], current: row.current, continuous: row.continuous, paused: row.paused }} art={currentArt?.artwork}
      state={{ progress: BigInt(row.progressRaw), dailyGain: row.dailyGainRaw === null ? null : BigInt(row.dailyGainRaw) }} live={live} reason={currentArt?.reason ?? undefined} pending={shown.pending} />}
    {finalPage > 0n && <nav className="focus-page-controls" aria-label="Completion pages">
      <button type="button" disabled={snapshot.page === 0n || shown.pending} onClick={() => changePage(0n)}>First</button>
      <button type="button" disabled={snapshot.page === 0n || shown.pending} onClick={() => changePage(snapshot.page - 1n)}>Previous</button>
      <span role="status">Page {String(snapshot.page + 1n)} of {BigInt(row.completed.count) === 0n ? '1' : row.completed.count}</span>
      <button type="button" disabled={snapshot.page >= finalPage || shown.pending} onClick={() => changePage(snapshot.page + 1n)}>Next</button>
      <button type="button" disabled={snapshot.page >= finalPage || shown.pending} onClick={() => changePage(finalPage)}>Last</button>
    </nav>}
    <div className="focus-scroll" ref={list}>
      {rows.length ? <VirtualList identity={listIdentity} retainExpansion={false} items={rows} itemKey={item => item.kind === 'completed'
        ? `${item.entry.ordinal}:${item.entry.completedByHour ?? row.orderFirstHour}` : `unfinished:${item.token}`} estimate={62} label="Completed focuses in order" className="focus-list" scrollRef={list}
        isInteractive={isCompletedFocus} rowProps={item => item.kind === 'unfinished' ? { className: 'focus-entry is-unfinished' }
          : ({ className: `focus-entry${live && BigInt(item.entry.ordinal) === BigInt(row.completed.rows) - 1n ? ' is-newest' : ''}`, ...rowJumpProps(runtime, track.id, item.entry.completedByHour, menu.open) })}>{item => {
        if (item.kind === 'unfinished') return <div className="focus-row">
          <span className="focus-number" aria-hidden="true">{String(BigInt(row.completed.rows) + 1n).padStart(2, '0')}</span>
          <FocusArt art={currentArt?.artwork} size="small" />
          <span className="focus-label"><strong data-tooltip={contentTitleHint(item.token, currentArt?.reason ?? undefined)}>{contentTitle(currentArt?.artwork.name, item.token, 'Focus name unavailable', shown.pending)}</strong>
            <span>{row.paused ? 'Paused' : 'In progress'}</span></span>
        </div>;
        const entry = item.entry, key = preparedArtKey(entry.completedByHour ?? row.orderFirstHour, entry.token);
        const art = snapshot.art.get(key), read = detailRead(key);
        const description = read && read !== FAILED_FACT ? read.artwork.description ?? art?.artwork.description : art?.artwork.description;
        if (description) descriptions.current!.texts.set(key, description);
        return <FocusCompletion entry={entry} art={art?.artwork} reason={art?.reason ?? undefined} pending={shown.pending}
          description={description ?? (detailPending(key) ? descriptions.current!.texts.get(key) : null)} open={detailEntries.some(item => item.ordinal === entry.ordinal)}
          onToggle={open => setOpened(previous => {
            const wanted = previous?.identity === identity && previous.client === runtime.client && previous.slot === row.slot && previous.countryId === row.countryId ? previous.rows : new Set<string>();
            if (wanted.has(key) === open) return previous;
            const rows = new Set(wanted); if (open) rows.add(key); else rows.delete(key);
            return { identity, client: runtime.client, slot: row.slot, countryId: row.countryId, rows };
          })} />;
      }}</VirtualList> : <p className="focus-empty">No completed focuses recorded at this point.</p>}
    </div>
    {menu.menu}
  </section>;
}
