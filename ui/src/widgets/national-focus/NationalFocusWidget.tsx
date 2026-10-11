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
import type { Artwork, FocusOrder, FocusReply } from '../../generated/dashboard';
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

const NO_WARNINGS: string[] = [];
const days = (value: number) => `${enTwoDecimals.format(value)} ${value === 1 ? 'day' : 'days'}`;

function FocusArt({ art, size }: { art: Artwork | undefined; size: 'large' | 'small' }) {
  return <span className={`focus-art is-${size}`}>{art?.icon ? <img src={art.icon} alt="" /> : <ArtFallback icon="target" />}</span>;
}

/** Live or pinned: the focus being worked on at that hour, with the game's progress line along its lower edge. */
function CurrentFocus({ order, art, state, live, reason, pending }: { order: FocusOrder; art: Artwork | undefined; state: FocusProgressState | null; live: boolean; reason?: string; pending?: boolean }) {
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

export default function NationalFocusWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
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
  const detailsWanted = useMemo(() => opened?.identity === detailsIdentity && opened.client === runtime.client
    ? wanted.filter(item => opened.rows.has(nameKey(item.hour, item.tag))) : [], [opened, detailsIdentity, runtime.client, wanted]);
  const detailsKeys = useMemo(() => new Set(detailsWanted.map(item => nameKey(item.hour, item.tag))), [detailsWanted]);
  const dated = useFocusArtwork(runtime.client, track, tag, wanted, active && inRange === true, detailsWanted, data?.staticText);
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
  const count = order?.completed.length ?? 0;
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
  // The whole-campaign view lists the focus left unfinished at the end instead of a live card.
  const unfinished = moment ? null : order.current;
  const currentArt = at(order.current ?? order.continuous ?? '', currentHour);
  return <section key={detailsIdentity} className="national-focus" aria-label="National focus">
    {headerSlot && createPortal(<span className="pill focus-header-count">{order.completed.length} completed</span>, headerSlot)}
    {moment && <CurrentFocus order={order} art={currentArt.art} state={state} live={live} reason={currentArt.reason} pending={currentArt.pending} />}
    <div className="focus-scroll" ref={list}>
      {order.completed.length || unfinished ? <ol className="focus-list" aria-label="Completed focuses in order">{order.completed.map(entry => {
        const readHour = entry.completedByHour ?? readingHour!, key = nameKey(readHour, entry.token);
        const { art, reason, pending } = at(entry.token, readHour);
        const date = entry.completedByHour == null ? null : gameDate(entry.completedByHour);
        return <li key={entry.ordinal} className={`focus-entry${entry.ordinal === newest ? ' is-newest' : ''}`}
          {...rowJumpProps(runtime, track.id, entry.completedByHour, menu.open)}>
          <details className="focus-details" open={detailsKeys.has(key)} onToggle={event => toggleDetails(key, event.currentTarget.open)}>
            <summary className="focus-row">
              <span className="focus-number" aria-label={`Focus ${entry.ordinal + 1}`}>{String(entry.ordinal + 1).padStart(2, '0')}</span>
              <FocusArt art={art} size="small" />
              <span className="focus-label"><strong data-tooltip={contentTitleHint(entry.token, reason)}>{contentTitle(art?.name, entry.token, 'Focus name unavailable', pending)}</strong>
                <span className="focus-date" data-tooltip={entry.completedByHour == null ? 'Completion date unknown.' : 'Complete by this date.'}>
                  {date == null ? 'Date not recorded' : `By ${formatDate(date)}`}
                </span>
              </span>
              <svg className="focus-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
            </summary>
            <div className="focus-description">
              <div className="focus-duration">{art?.baseDurationDays != null ? <>Base duration <strong>{days(art.baseDurationDays)}</strong></> : 'Duration unavailable'}</div>
              {art?.description && <p>{art.description}</p>}
            </div>
          </details>
        </li>;
      })}
        {unfinished && <li className="focus-entry is-unfinished">
          <div className="focus-row">
            <span className="focus-number" aria-hidden="true">{String(order.completed.length + 1).padStart(2, '0')}</span>
            <FocusArt art={currentArt.art} size="small" />
            <span className="focus-label"><strong data-tooltip={contentTitleHint(unfinished, currentArt.reason)}>{contentTitle(currentArt.art?.name, unfinished, 'Focus name unavailable', currentArt.pending)}</strong>
              <span>{order.paused ? 'Paused' : 'In progress'}</span></span>
          </div>
        </li>}
      </ol> : <p className="focus-empty">No completed focuses recorded at this point.</p>}
    </div>
    {menu.menu}
  </section>;
}
