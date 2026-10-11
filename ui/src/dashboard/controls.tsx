import { useCallback, useEffect, useId, useMemo, useState, useSyncExternalStore } from 'react';
import type { ComponentProps, CSSProperties, PointerEvent } from 'react';
import DateTimePicker from '../components/DateTimePicker';
import PlaybackSpeed from '../components/PlaybackSpeed';
import { formatDateTime } from './calendar';
import { Icon } from '../components/Icons';
import type { BoundTrack, DashboardTemplate } from './model';
import type { PlaybackController } from './playback';
import { timelineGaps } from './timelineGaps';
import CountryPicker from '../components/CountryPicker';

/** The header's country picker, showing the flag the game drew at the playback hour.
 * Only this component follows the clock; while playing it asks at most every 250 ms,
 * and the hours between those samples render nothing. */
export function PlaybackCountryPicker({ clock, track, ...picker }: { clock: PlaybackController; track: BoundTrack | undefined } & Omit<ComponentProps<typeof CountryPicker>, 'flagAt'>) {
  const [sampled, setSampled] = useState(() => clock.getHour());
  useEffect(() => {
    let hour = clock.getHour(), sent = 0, timer: ReturnType<typeof setTimeout> | undefined;
    const sample = () => {
      clearTimeout(timer);
      const wait = clock.getSnapshot().playing ? Math.max(0, 250 - (performance.now() - sent)) : 0;
      timer = setTimeout(() => { sent = performance.now(); setSampled(clock.getHour()); }, wait);
    };
    sample();
    const stop = clock.subscribeHour(() => { if (clock.getHour() !== hour) { hour = clock.getHour(); sample(); } });
    return () => { stop(); clearTimeout(timer); };
  }, [clock]);
  const raw = track ? sampled + track.axisOffset : null;
  const flagAt = useMemo(() => track && raw != null && raw >= track.branch.firstHour && raw <= track.branch.lastHour
    ? { branchId: track.branch.id, hour: raw } : null, [track?.branch.id, track?.branch.firstHour, track?.branch.lastHour, raw]);
  return <CountryPicker {...picker} flagAt={flagAt} />;
}

export function PlaybackStrip({ clock, tracks, alignment, defaultRoleId }: {
  clock: PlaybackController; tracks: BoundTrack[]; alignment: DashboardTemplate['alignment']; defaultRoleId: string | null;
}) {
  const controlsKey = useCallback(() => {
    const snapshot = clock.getSnapshot();
    return `${snapshot.hour}:${snapshot.playing}:${snapshot.hoursPerSecond}:${snapshot.bounds?.firstHour}:${snapshot.bounds?.lastHour}`;
  }, [clock]);
  useSyncExternalStore(clock.subscribe, controlsKey, controlsKey);
  const snapshot = clock.getSnapshot();
  const [dateDraft, setDateDraft] = useState<{ reference: string; hour: number | null } | null>(null);
  const [hover, setHover] = useState<{ left: number; hour: number } | null>(null);
  const helpId = useId();
  const reference = tracks.find((track) => track.roleId === defaultRoleId);
  const referenceKey = JSON.stringify(reference && [reference.id, reference.source.id, reference.source.revision,
    reference.branch.id, reference.branch.firstHour, reference.branch.lastHour, reference.axisOffset]);
  const draft = dateDraft && dateDraft.reference === referenceKey ? dateDraft.hour : null;
  const setDraft = (hour: number | null) => setDateDraft({ reference: referenceKey, hour });
  const rawHour = reference ? snapshot.hour + reference.axisOffset : null;
  const bounds = snapshot.bounds;
  const span = bounds ? bounds.lastHour - bounds.firstHour : 0;
  const progress = bounds && span > 0 ? 100 * (snapshot.hour - bounds.firstHour) / span : 0;
  const gaps = useMemo(() => reference && bounds ? timelineGaps(reference.branch.gaps ?? [], reference.axisOffset, bounds.firstHour, bounds.lastHour) : [], [reference, bounds]);
  const currentGap = reference?.branch.gaps?.find(gap => rawHour != null && gap.firstHour <= rawHour && gap.lastHour >= rawHour);
  const trackHover = (event: PointerEvent<HTMLInputElement>) => {
    if (!bounds || span <= 0 || !reference) return;
    // The range maps values across the track minus the thumb.
    const rect = event.currentTarget.getBoundingClientRect();
    const fraction = Math.min(1, Math.max(0, (event.clientX - rect.left - 7) / Math.max(1, rect.width - 14)));
    setHover({ left: 7 + fraction * (rect.width - 14), hour: Math.round(bounds.firstHour + fraction * span) + reference.axisOffset });
  };
  return <>
    <section className="header-playback" aria-label="Playback">
      <div className={`header-date${draft != null ? ' has-date-draft' : ''}`}>
        <DateTimePicker key={referenceKey} label="Date" hideLabel value={draft ?? rawHour} disabled={!reference}
          otherValue={null} min={reference?.branch.firstHour ?? 0} max={reference?.branch.lastHour ?? 0} describedBy={helpId} onChange={setDraft} />
        {draft != null && reference && <button type="button" className="button primary small" onClick={() => { clock.seek(draft - reference.axisOffset); setDraft(null); }}>Go</button>}
      </div>
      <div className="header-transport" aria-label="Playback controls">
        <button type="button" className="icon-button" disabled={!bounds || snapshot.hour <= bounds.firstHour} onClick={() => { setDraft(null); clock.step(-1); }} aria-label="Previous hour" data-tooltip="Previous hour"><Icon name="chevrons-left" /></button>
        <button type="button" className={`header-play${snapshot.playing ? ' is-playing' : ''}`} disabled={!bounds || snapshot.hour >= bounds.lastHour} onClick={() => { setDraft(null); clock.setPlaying(!snapshot.playing); }} aria-label={snapshot.playing ? 'Pause playback' : 'Play recording'} data-tooltip={snapshot.playing ? 'Pause' : 'Play'}>
          <Icon name={snapshot.playing ? 'pause' : 'play'} />
        </button>
        <button type="button" className="icon-button" disabled={!bounds || snapshot.hour >= bounds.lastHour} onClick={() => { setDraft(null); clock.step(1); }} aria-label="Next hour" data-tooltip="Next hour"><Icon name="chevrons-right" /></button>
      </div>
      <PlaybackSpeed value={snapshot.hoursPerSecond} onChange={(speed) => clock.setSpeed(speed)} />
    </section>
    <div className="header-timeline-wrap" data-empty={!bounds || span <= 0 || undefined}>
      <div className="timeline-track">
        <input className="header-timeline" type="range" aria-label="Campaign timeline" disabled={!bounds || span <= 0}
          min={bounds?.firstHour ?? 0} max={bounds?.lastHour ?? 1} step={1} value={snapshot.hour}
          aria-valuetext={rawHour == null ? 'No recording selected' : `${formatDateTime(rawHour)}${currentGap ? `; reported recording gap: ${currentGap.reason}` : ''}`}
          style={{ '--scrub-progress': `${progress}%` } as CSSProperties}
          onPointerMove={trackHover} onPointerLeave={() => setHover(null)}
          onChange={(event) => { setDraft(null); clock.seek(Number(event.currentTarget.value)); }} />
        <div className="timeline-gap-markers">{gaps.map((gap, index) => <span key={index} className="timeline-gap" style={{ left: `${gap.left}%`, width: `${gap.width}%` }}
          data-tooltip={`Reported recording gap · ${formatDateTime(gap.firstHour)} to ${formatDateTime(gap.lastHour)} · ${gap.reason}`} />)}</div>
        {hover && <span className="timeline-hover" style={{ left: hover.left }} aria-hidden="true">{formatDateTime(hover.hour)}</span>}
      </div>
    </div>
    <p className="sr-only" id={helpId}>Date browsing is a draft until Go. {alignment === 'anchors' ? 'The date uses the selected reference track; playback aligns by track anchors.' : 'Tracks share the game-calendar date.'} Only views following playback move with these controls.</p>
  </>;
}
