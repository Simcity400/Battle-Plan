import { VirtualList } from '../../components/VirtualList';
import ArtFallback from '../../components/ArtFallback';
import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { formatDate, gameDate } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { focusParityModel } from './focusParityModel';
import { useParityRows } from '../../dashboard/parityRows';
import type { Artwork, FocusEntry, FocusOrder, FocusReply } from '../../generated/dashboard';
import { FAILED_FACT, nameKey } from '../../dashboard/countryFacts';
import { useFocusArtwork } from './useFocusArtwork';
import { focusSegmentAt, focusCompletion, focusProgressAt, focusRequest, validateFocusReply } from './focusData';
import type { FocusProgressState } from './focusData';
import WidgetState from '../../components/WidgetState';
import ListSkeleton from '../../components/Skeleton';
import { Icon } from '../../components/Icons';
import { useContextMenu } from '../../components/ContextMenu';
import { rowJumpProps } from '../../charts/jump';
import { enTwoDecimals } from '../../charts/numberFormat';
import './nationalFocus.css';
import PreparedFocusWidget from './PreparedFocusWidget';
import type { FocusPreparedClient } from './preparedFocusData';

const NO_WARNINGS: string[] = [];
const days = (value: number) => `${enTwoDecimals.format(value)} ${value === 1 ? 'day' : 'days'}`;
export type FocusListItem = { kind: 'completed'; entry: FocusEntry } | { kind: 'unfinished'; token: string };
export const isCompletedFocus = (item: FocusListItem) => item.kind === 'completed';
const disclosureKey = (entry: FocusEntry, readHour: number) => JSON.stringify([entry.ordinal, nameKey(readHour, entry.token)]);

export function FocusArt({ art, size }: { art: Artwork | undefined; size: 'large' | 'small' }) {
  return <span className={`focus-art is-${size}`}>{art?.icon ? <img src={art.icon} alt="" /> : <ArtFallback icon="target" />}</span>;
}

export function FocusCompletion({ entry, art, reason, pending, datePreparing, description, open, onToggle }: {
  entry: FocusEntry; art?: Artwork; reason?: string; pending?: boolean; datePreparing?: boolean; description?: string | null;
  open: boolean; onToggle: (open: boolean) => void;
}) {
  const date = entry.completedByHour == null ? null : gameDate(entry.completedByHour);
  return <details className="focus-details" open={open} onToggle={event => onToggle(event.currentTarget.open)}>
    <summary className="focus-row">
      <span className="focus-number" aria-label={`Focus ${entry.ordinal + 1}`}>{String(entry.ordinal + 1).padStart(2, '0')}</span>
      <FocusArt art={art} size="small" />
      <span className="focus-label"><strong data-tooltip={contentTitleHint(entry.token, reason)}>{contentTitle(art?.name, entry.token, 'Focus name unavailable', pending)}</strong>
        <span className="focus-date" data-tooltip={datePreparing ? 'Completion date is still being prepared.' : date == null ? 'Completion date unknown.'
          : 'Completed by this date. Focuses are read once a game day, so it may have finished a day or so earlier.'}>
          {datePreparing ? 'Still being prepared' : date == null ? 'Date not recorded' : formatDate(date)}
        </span>
      </span>
      <svg className="focus-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
    </summary>
    <div className="focus-description">
      <div className="focus-duration">{art?.baseDurationDays != null ? <>Base duration <strong>{days(art.baseDurationDays)}</strong></> : 'Duration unavailable'}</div>
      {description && <p>{description}</p>}
    </div>
  </details>;
}

/** Live or pinned: the focus being worked on at that hour, with the game's progress line along its lower edge. */
export function CurrentFocus({ order, art, state, live, reason, pending }: { order: FocusOrder; art: Artwork | undefined; state: FocusProgressState | null; live: boolean; reason?: string; pending?: boolean }) {
  const token = order.current ?? order.continuous;
  const continuous = !order.current && !!order.continuous;
  const completion = order.current && state && art?.baseDurationDays != null ? focusCompletion(state, art.baseDurationDays) : null;
  if (!token) {
    const banked = state && state.progress > 0n ? Number(state.progress / 100_000n) : 0;
    return <div className="focus-now is-idle" role="status">
      <span className="focus-art is-large"><span className="focus-missing-art"><Icon name="pause" /></span></span>
      <span className="focus-now-text"><strong>No national focus selected</strong>
        <span>{banked > 0 ? `${days(banked)} of progress saved for the next focus` : null}</span></span>
    </div>;
  }
  const status = continuous ? 'Continuous focus' : order.paused ? 'Paused' : 'In progress';
  const name = contentTitle(art?.name, token, 'Focus name unavailable', pending);
  return <div className={`focus-now${order.paused ? ' is-paused' : ''}`} aria-live="polite">
    <FocusArt art={art} size="large" />
    <span className="focus-now-text"><strong data-tooltip={contentTitleHint(token, reason)}>{name}</strong>
      <span className="focus-now-status">{live && !continuous && !order.paused && <span className="live-dot" />}{status}
        {completion && <span className="focus-now-days">· Day {Math.min(completion.elapsedDays + 1, Math.ceil(completion.totalDays))} of {enTwoDecimals.format(completion.totalDays)}</span>}</span></span>
    {completion && <span className="focus-now-left" data-tooltip={completion.daysLeft == null ? undefined : 'Estimated at current focus speed.'}>
      {completion.daysLeft == null ? <><strong>{Math.round(completion.fraction * 100)}%</strong><small>complete</small></>
        : <><strong>{completion.daysLeft}</strong><small>{completion.daysLeft === 1 ? 'day left' : 'days left'}</small></>}
    </span>}
    {completion && <span className="focus-progress" role="progressbar" aria-label={`${name} progress`} aria-valuemin={0} aria-valuemax={100}
      aria-valuenow={Math.round(completion.fraction * 100)} style={{ '--focus-progress': completion.fraction } as CSSProperties}><span /></span>}
  </div>;
}

export function LegacyNationalFocusWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const menu = useContextMenu('National focus actions');
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const live = widget.time.mode === 'follow';
  // A pinned moment shows the focus in progress then, like Live but held still.
  const moment = live || widget.time.mode === 'pin';
  const inRange = track && hour != null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const request = track && tag ? focusRequest(track.source.id, track.source.revision, { branchId: track.branch.id, tag }) : null;
  const { data, error, retry } = useProduct<FocusReply>(runtime.client, request, active, validateFocusReply);
  const contentRevision = useContentRevision(runtime.client);
  const detailsIdentity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, contentRevision]) : '';
  const [opened, setOpened] = useState<{ identity: string; client: typeof runtime.client; rows: Set<string> } | null>(null);
  // Only explicitly static text may be shown before the dated reply arrives.
  const metadata = useMemo(() => {
    const text = new Map(data?.staticText?.map(item => [item.token, item]));
    return new Map(data?.artwork.map(({ token, baseDurationDays }) => [token,
      { token, baseDurationDays, name: text.get(token)?.name ?? null, description: text.get(token)?.description ?? null, icon: null } satisfies Artwork]));
  }, [data]);
  const segment = data && hour != null ? focusSegmentAt(data.history, hour) : null;
  const order = segment?.order ?? null;
  const state = data && hour != null && moment ? focusProgressAt(data.history, hour) : null;
  const currentHour = Math.floor(hour ?? 0);
  const readingHour = segment?.firstHour;
  const wanted = useMemo(() => {
    const rows = order?.completed.map(entry => ({ tag: entry.token, hour: entry.completedByHour ?? readingHour! })) ?? [];
    // Follow scrolls to the newest completion; schedule that end of the list first.
    // Presentation keeps the recorded ordinal order in every time mode.
    if (live) rows.reverse();
    const current = order?.current ?? order?.continuous;
    if (current && hour != null) {
      const item = { tag: current, hour: currentHour };
      if (moment) rows.unshift(item);
      else rows.push(item);
    }
    return rows.filter(row => Number.isInteger(row.hour));
  }, [order, readingHour, hour, currentHour, moment, live]);
  // Repeated completions may share dated text, but their disclosure state must stay separate.
  const openRows = opened?.identity === detailsIdentity && opened.client === runtime.client ? opened.rows : null;
  const detailsWanted = useMemo(() => {
    const rows = openRows ? order?.completed.flatMap(entry => {
      const hour = entry.completedByHour ?? readingHour!;
      return Number.isInteger(hour) && openRows.has(disclosureKey(entry, hour)) ? [{ tag: entry.token, hour }] : [];
    }) ?? [] : [];
    // Follow schedules the newest end of the list first, as `wanted` does.
    return live ? rows.reverse() : rows;
  }, [openRows, order, readingHour, live]);
  const detailsKeys = useMemo(() => new Set(detailsWanted.map(item => nameKey(item.hour, item.tag))), [detailsWanted]);
  const dated = useFocusArtwork(runtime.client, track, tag, wanted, active && inRange === true, detailsWanted, data?.staticText);
  // Descriptions rows have shown, used while their read has no answer: closing releases the demand, yet the row
  // closes with its text, and reopening shows the same reading at once. Another identity or client starts empty.
  const shown = useRef<{ identity: string; client: typeof runtime.client; texts: Map<string, string> } | null>(null);
  if (shown.current?.identity !== detailsIdentity || shown.current.client !== runtime.client) {
    shown.current = { identity: detailsIdentity, client: runtime.client, texts: new Map() };
  }
  const shownTexts = shown.current.texts;
  const toggleDetails = (key: string, open: boolean) => setOpened(previous => {
    const rows = previous?.identity === detailsIdentity && previous.client === runtime.client ? previous.rows : new Set<string>();
    if (rows.has(key) === open && previous?.identity === detailsIdentity && previous.client === runtime.client) return previous;
    const next = new Set(rows);
    if (open) next.add(key); else next.delete(key);
    return { identity: detailsIdentity, client: runtime.client, rows: next };
  });
  const at = (token: string, readHour: number) => {
    const item = dated.read.get(nameKey(readHour, token));
    const fallback = metadata.get(token);
    return item && item !== FAILED_FACT ? { art: { ...item.artwork, description: item.artwork.description ?? fallback?.description ?? null },
      reason: [item.titlePending ? 'Loading focus name\u2026' : item.titleReason, item.reason].filter(Boolean).join('; ') || undefined, pending: item.titlePending ?? false }
      : { art: fallback, pending: item === undefined, reason: item === undefined ? 'Loading focus name\u2026'
        : item === FAILED_FACT ? 'Focus name request failed.' : 'Focus name was not recorded at this hour.' };
  };
  const unavailable = useMemo(() => {
    const reasons = new Map<string, number>();
    for (const item of dated.read.values()) {
      const reason = item === FAILED_FACT ? 'Game artwork request failed.'
        : item === null ? 'Game artwork was not recorded at this hour.' : [item.titleReason, item.reason, item.imageReason].filter(Boolean).join('; ');
      if (reason) reasons.set(reason, (reasons.get(reason) ?? 0) + 1);
    }
    for (const item of dated.details?.values() ?? []) {
      const reason = item === FAILED_FACT ? 'Focus description request failed.'
        : item === null ? 'Focus description was not recorded at this hour.' : item.reason;
      if (reason) reasons.set(reason, (reasons.get(reason) ?? 0) + 1);
    }
    return [...reasons].map(([reason, count]) => ({ reason, count }));
  }, [dated.read, dated.details]);
  const diagnostics = useMemo(() => [...(data?.warnings ?? NO_WARNINGS), ...dated.warnings,
    ...unavailable.map(item => `${item.reason} (${item.count})`)], [data, dated.warnings, unavailable]);
  useDiagnostics(onDiagnostics, diagnostics);
  const list = useRef<HTMLDivElement>(null);
  const listIdentity = useMemo(() => ({}), [detailsIdentity, runtime.client]);
  const count = order?.completed.length ?? 0;
  if (import.meta.env.MODE === 'parity') {
    useParityRows(list, () => focusParityModel(order, state, readingHour, currentHour, moment, data?.staticText ?? [],
      (token, date) => {
        const value = at(token, date);
        return { artwork: value.art ?? null, pending: value.pending, reason: value.reason ?? null,
          descriptionDemanded: detailsKeys.has(nameKey(date, token)) };
      }));
  }
  const listItems = useMemo<FocusListItem[]>(() => {
    const items: FocusListItem[] = order?.completed.map(entry => ({ kind: 'completed', entry })) ?? [];
    if (!moment && order?.current) items.push({ kind: 'unfinished', token: order.current });
    return items;
  }, [order, moment]);
  const seen = useRef(count);
  // While following playback, keep the newest completion in view as it lands.
  useLayoutEffect(() => {
    if (live && count > seen.current) list.current?.scrollTo({ top: list.current.scrollHeight, behavior: document.documentElement.dataset.motion === 'reduced' ? 'auto' : 'smooth' });
    seen.current = count;
  }, [live, count]);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="target" /></span><p>Load a campaign and choose a country to see its national focuses.</p></div>;
  if (!inRange) return <div className="dash-empty" role="status">This time is outside the recording.</div>;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <ListSkeleton label="Loading national focuses…" />;
  if (!order) return <div className="dash-empty" role="status">No complete focus data recorded at this date.</div>;
  const newest = live ? order.completed.at(-1)?.ordinal : undefined;
  const currentArt = at(order.current ?? order.continuous ?? '', currentHour);
  return <section key={detailsIdentity} className="national-focus" aria-label="National focus">
    {headerSlot && createPortal(<span className="pill focus-header-count">{order.completed.length} completed</span>, headerSlot)}
    {moment && <CurrentFocus order={order} art={currentArt.art} state={state} live={live} reason={currentArt.reason} pending={currentArt.pending} />}
    <div className="focus-scroll" ref={list}>
      {listItems.length ? <VirtualList identity={listIdentity} retainExpansion={false} items={listItems} itemKey={item => item.kind === 'completed'
        ? `${item.entry.ordinal}:${item.entry.completedByHour ?? readingHour}` : `unfinished:${item.token}`} estimate={62} label="Completed focuses in order" className="focus-list" scrollRef={list}
        isInteractive={isCompletedFocus} rowProps={item => item.kind === 'unfinished' ? { className: 'focus-entry is-unfinished' }
          : ({ className: `focus-entry${item.entry.ordinal === newest ? ' is-newest' : ''}`, ...rowJumpProps(runtime, track.id, item.entry.completedByHour, menu.open) })}>{item => {
        // The unfinished focus is the final noninteractive ordered row.
        if (item.kind === 'unfinished') return <div className="focus-row">
          <span className="focus-number" aria-hidden="true">{String(order.completed.length + 1).padStart(2, '0')}</span>
          <FocusArt art={currentArt.art} size="small" />
          <span className="focus-label"><strong data-tooltip={contentTitleHint(item.token, currentArt.reason)}>{contentTitle(currentArt.art?.name, item.token, 'Focus name unavailable', currentArt.pending)}</strong>
            <span>{order.paused ? 'Paused' : 'In progress'}</span></span>
        </div>;
        const entry = item.entry;
        const readHour = entry.completedByHour ?? readingHour!, key = nameKey(readHour, entry.token), disclosure = disclosureKey(entry, readHour);
        const { art, reason, pending } = at(entry.token, readHour);
        if (art?.description) shownTexts.set(key, art.description);
        const description = art?.description ?? (dated.details?.get(key) === undefined ? shownTexts.get(key) : undefined);
        return <FocusCompletion entry={entry} art={art} reason={reason} pending={pending} description={description}
          open={!!openRows?.has(disclosure)} onToggle={open => toggleDetails(disclosure, open)} />;
      }}</VirtualList> : <p className="focus-empty">No completed focuses recorded at this point.</p>}
    </div>
    {menu.menu}
  </section>;
}

/** The historical product remains available for clients without prepared queries. */
export default function NationalFocusWidget(props: WidgetProps) {
  return (props.runtime.client as FocusPreparedClient).preparedQuery
    ? <PreparedFocusWidget {...props} /> : <LegacyNationalFocusWidget {...props} />;
}
