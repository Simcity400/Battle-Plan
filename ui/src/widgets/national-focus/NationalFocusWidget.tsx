import { useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { formatDate, gameDate } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { Artwork, FocusOrder, FocusReply } from '../../generated/dashboard';
import { focusAt, focusCompletion, focusProgressAt, focusRequest, validateFocusReply } from './focusData';
import type { FocusProgressState } from './focusData';
import ListSkeleton from '../../components/Skeleton';
import { Icon } from '../../components/Icons';
import './nationalFocus.css';

const NO_WARNINGS: string[] = [];
const days = (value: number) => `${value.toLocaleString('en', { maximumFractionDigits: 2 })} ${value === 1 ? 'day' : 'days'}`;

function FocusArt({ art, size }: { art: Artwork | undefined; size: 'large' | 'small' }) {
  return <span className={`focus-art is-${size}`}>{art?.icon ? <img src={art.icon} alt="" /> : <span className="focus-missing-art" data-tooltip="Game artwork unavailable in the recorded content"><Icon name="target" /></span>}</span>;
}

/** Live or pinned: the focus being worked on at that hour, with the game's progress line along its lower edge. */
function CurrentFocus({ order, art, state, live }: { order: FocusOrder; art: Artwork | undefined; state: FocusProgressState | null; live: boolean }) {
  const token = order.current ?? order.continuous;
  const continuous = !order.current && !!order.continuous;
  const completion = order.current && state && art?.baseDurationDays != null ? focusCompletion(state, art.baseDurationDays) : null;
  if (!token) {
    const banked = state && state.progress > 0n ? Number(state.progress / 100_000n) : 0;
    return <div className="focus-now is-idle" role="status">
      <span className="focus-art is-large"><span className="focus-missing-art"><Icon name="pause" /></span></span>
      <span className="focus-now-text"><strong>No national focus selected</strong>
        <span>{banked > 0 ? `${days(banked)} of progress saved for the next focus` : 'The country is not working on a national focus.'}</span></span>
    </div>;
  }
  const status = continuous ? 'Continuous focus' : order.paused ? 'Paused' : 'In progress';
  return <div className={`focus-now${order.paused ? ' is-paused' : ''}`} aria-live="polite">
    <FocusArt art={art} size="large" />
    <span className="focus-now-text"><strong>{art?.name ?? token}</strong>
      <span className="focus-now-status">{live && !continuous && !order.paused && <span className="live-dot" />}{status}
        {completion && <span className="focus-now-days" data-tooltip="Recorded work toward this focus's base duration">· Day {Math.min(completion.elapsedDays + 1, Math.ceil(completion.totalDays))} of {completion.totalDays.toLocaleString('en', { maximumFractionDigits: 2 })}</span>}</span></span>
    {completion && <span className="focus-now-left" data-tooltip={completion.daysLeft == null ? 'The daily rate is not known yet: it needs one recorded day of progress on this focus.' : 'At the last recorded daily rate. Focus speed changes can shift it.'}>
      {completion.daysLeft == null ? <><strong>{Math.round(completion.fraction * 100)}%</strong><small>complete</small></>
        : <><strong>{completion.daysLeft}</strong><small>{completion.daysLeft === 1 ? 'day left' : 'days left'}</small></>}
    </span>}
    {completion && <span className="focus-progress" role="progressbar" aria-label={`${art?.name ?? token} progress`} aria-valuemin={0} aria-valuemax={100}
      aria-valuenow={Math.round(completion.fraction * 100)} style={{ '--focus-progress': completion.fraction } as CSSProperties}><span /></span>}
  </div>;
}

export default function NationalFocusWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
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
  useEffect(() => {
    onDiagnostics(data?.warnings ?? NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  const artwork = useMemo(() => new Map(data?.artwork.map(art => [art.token, art])), [data]);
  const order = data && hour != null ? focusAt(data.history, hour) : null;
  const state = data && hour != null && moment ? focusProgressAt(data.history, hour) : null;
  const list = useRef<HTMLDivElement>(null);
  const count = order?.completed.length ?? 0;
  const seen = useRef(count);
  // While following playback, keep the newest completion in view as it lands.
  useLayoutEffect(() => {
    if (live && count > seen.current) list.current?.scrollTo({ top: list.current.scrollHeight, behavior: document.documentElement.dataset.motion === 'reduced' ? 'auto' : 'smooth' });
    seen.current = count;
  }, [live, count]);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="target" /></span><p>Load a campaign and choose a country to see its national focuses.</p></div>;
  if (!inRange) return <div className="dash-empty" role="status">This time is outside the country’s recorded branch.</div>;
  if (error) return <div className="dash-empty" role="alert"><p>{error}</p><button className="button small" onClick={retry}>Retry</button></div>;
  if (!data) return <ListSkeleton label="Loading national focuses…" />;
  if (!order) return <div className="dash-empty" role="status">No complete focus data recorded at this date.</div>;
  const newest = live ? order.completed.at(-1)?.ordinal : undefined;
  // The whole-campaign view lists the focus left unfinished at the end instead of a live card.
  const unfinished = moment ? null : order.current;
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="national-focus" aria-label="National focus">
    {headerSlot && createPortal(<span className="pill focus-header-count">{order.completed.length} completed</span>, headerSlot)}
    {moment && <CurrentFocus order={order} art={artwork.get(order.current ?? order.continuous ?? '')} state={state} live={live} />}
    <div className="focus-scroll" ref={list}>
      {order.completed.length || unfinished ? <ol className="focus-list" aria-label="Completed focuses in order">{order.completed.map(entry => {
        const art = artwork.get(entry.token);
        const date = entry.completedByHour == null ? null : gameDate(entry.completedByHour);
        return <li key={entry.ordinal} className={`focus-entry${entry.ordinal === newest ? ' is-newest' : ''}`}>
          <details className="focus-details">
            <summary className="focus-row">
              <span className="focus-number" aria-label={`Focus ${entry.ordinal + 1}`}>{String(entry.ordinal + 1).padStart(2, '0')}</span>
              <FocusArt art={art} size="small" />
              <span className="focus-label"><strong>{art?.name ?? entry.token}</strong>
                <span className="focus-date" data-tooltip={entry.completedByHour == null ? 'This focus was already complete when readable recording history began, or its completion could not be dated.' : 'First recorded observation of this completion; the exact completion time is not stored.'}>
                  {date == null ? 'Date not recorded' : `By ${formatDate(date)}`}
                </span>
              </span>
              <svg className="focus-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
            </summary>
            <div className="focus-description">
              <div className="focus-duration">{art?.baseDurationDays != null ? <>Base duration <strong>{days(art.baseDurationDays)}</strong></> : 'Duration unavailable'}</div>
              <p>{art?.description ?? 'Description unavailable in the recorded game localisation.'}</p>
            </div>
          </details>
        </li>;
      })}
        {unfinished && <li className="focus-entry is-unfinished">
          <div className="focus-row">
            <span className="focus-number" aria-hidden="true">{String(order.completed.length + 1).padStart(2, '0')}</span>
            <FocusArt art={artwork.get(unfinished)} size="small" />
            <span className="focus-label"><strong>{artwork.get(unfinished)?.name ?? unfinished}</strong>
              <span>{order.paused ? 'Paused when the recording ended' : 'Still in progress when the recording ended'}</span></span>
            <span className="pill">Unfinished</span>
          </div>
        </li>}
      </ol> : <p className="focus-empty">No completed focuses recorded at this point.</p>}
    </div>
  </section>;
}
