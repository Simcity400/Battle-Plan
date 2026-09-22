import { useEffect, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { formatDate, gameDate } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { FocusReply } from '../../generated/dashboard';
import { focusAt, focusRequest, validateFocusReply } from './focusData';
import './nationalFocus.css';

const NO_WARNINGS: string[] = [];

export default function NationalFocusWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : track?.selectedTag
    ?? track?.source.metadata.countries.find(country => country.id === track.source.metadata.playerCountryId)?.tag;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const inRange = track && hour != null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const request = track && tag ? focusRequest(track.source.id, track.source.revision, { branchId: track.branch.id, tag }) : null;
  const { data, error, retry } = useProduct<FocusReply>(runtime.client, request, active, validateFocusReply);
  useEffect(() => {
    onDiagnostics(data?.warnings ?? NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  const artwork = useMemo(() => new Map(data?.artwork.map(art => [art.token, art])), [data]);
  const order = data && hour != null ? focusAt(data.history, hour) : null;
  if (!track || !tag) return <div className="dash-empty" role="status">Load a campaign and choose a country to see its national focuses.</div>;
  if (!inRange) return <div className="dash-empty" role="status">This time is outside the country’s recorded branch.</div>;
  if (error) return <div className="dash-empty" role="alert"><p>{error}</p><button className="button small" onClick={retry}>Retry</button></div>;
  if (!data) return <div className="dash-empty" role="status">Loading national focuses…</div>;
  if (!order) return <div className="dash-empty" role="status">No complete focus data recorded at this date.</div>;
  const current = order.current ?? order.continuous;
  function focus(token: string, ordinal: number | null, completedByHour: number | null = null) {
    const art = artwork.get(token);
    const date = completedByHour == null ? null : gameDate(completedByHour);
    return <li key={ordinal ?? 'current'} className={ordinal == null ? 'focus-entry focus-current' : 'focus-entry'}>
      <details className="focus-details">
        <summary className="focus-row">
          <span className="focus-number" aria-label={ordinal == null ? 'Current focus' : `Focus ${ordinal + 1}`}>{ordinal == null ? '→' : String(ordinal + 1).padStart(2, '0')}</span>
          <span className="focus-art">{art?.icon ? <img src={art.icon} alt="" width="66" height="64" /> : <span className="focus-missing-art" aria-label="Game artwork unavailable">◇</span>}</span>
          <span className="focus-label"><strong>{art?.name ?? token}</strong>
            {ordinal == null ? <span>{order!.paused ? 'Paused' : order!.current ? 'In progress' : 'Continuous focus'}</span>
              : <span className="focus-date" data-tooltip={completedByHour == null ? 'This focus was already complete when readable recording history began, or its completion could not be dated.' : 'First recorded observation of this completion; the exact completion time is not stored.'}>
                {date == null ? 'Date not recorded' : `By ${formatDate(date)}`}
              </span>}
          </span>
          <svg className="focus-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
        </summary>
        <div className="focus-description">
          <div className="focus-duration">{ordinal == null && order!.continuous === token && !order!.current
            ? 'Continuous focus'
            : art?.baseDurationDays != null ? <>Base duration <strong>{art.baseDurationDays.toLocaleString('en', { maximumFractionDigits: 2 })} {art.baseDurationDays === 1 ? 'day' : 'days'}</strong></>
              : 'Duration unavailable'}</div>
          <p>{art?.description ?? 'Description unavailable in the recorded game localisation.'}</p>
        </div>
      </details>
    </li>;
  }
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="national-focus" aria-label="National focus order">
    {headerSlot && createPortal(<span className="focus-header-count"> - {order.completed.length} completed focuses</span>, headerSlot)}
    <div className="focus-scroll">
      {order.completed.length ? <ol className="focus-list" aria-label="Focus order">{order.completed.map(entry => focus(entry.token, entry.ordinal, entry.completedByHour))}</ol> : <p className="focus-empty">No completed focuses recorded at this point.</p>}
      {current && <ol className="focus-list focus-active-list" aria-label="Current national focus">{focus(current, null)}</ol>}
    </div>
  </section>;
}
