import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { ComponentProps, PointerEvent } from 'react';
import DateTimePicker from '../components/DateTimePicker';
import PlaybackSpeed from '../components/PlaybackSpeed';
import { formatDateTime } from './calendar';
import { Icon } from '../components/Icons';
import type { BoundTrack, DashboardTemplate } from './model';
import { usePlaybackHour } from './playback';
import type { PlaybackController } from './playback';
import type { DashboardClient } from './client';
import { newId } from '../shell/ids';
import { validatePreparationReply, preparationActive } from '../shell/preparation/contract';
import type { PreparedCoverage } from '../shell/preparation/contract';
import { PAUSED_POLL_MS } from '../shell/preparation/controller';
import { timelineGaps, timelinePrepared } from './timelineGaps';
import CountryPicker from '../components/CountryPicker';
import { useMapMuted } from '../shell/muteSetting';

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

/** Mutes or restores the native maps' audio; only the focused map sounds. */
function MapMuteButton() {
  const { muted, error, set } = useMapMuted();
  if (muted == null && !error) return null;
  const label = muted ? 'Unmute map audio' : 'Mute map audio';
  return <button type="button" className="icon-button" aria-pressed={!!muted} onClick={() => set(!muted)}
    aria-label={label} data-tooltip={error ?? label}><Icon name={muted ? 'volume-off' : 'volume'} /></button>;
}

export function PlaybackStrip({ clock, tracks, alignment, defaultRoleId, client }: {
  client: DashboardClient; clock: PlaybackController; tracks: BoundTrack[]; alignment: DashboardTemplate['alignment']; defaultRoleId: string | null;
}) {
  // React renders the strip on anchors only; the playhead, the date and the hour-bound controls are
  // painted from the clock's frames (`paint` below), and the calendar takes the paced hour.
  const controlsKey = useCallback(() => {
    const snapshot = clock.getSnapshot();
    return `${snapshot.playing}:${snapshot.hoursPerSecond}:${snapshot.bounds?.firstHour}:${snapshot.bounds?.lastHour}`;
  }, [clock]);
  useSyncExternalStore(clock.subscribe, controlsKey, controlsKey);
  const pacedHour = usePlaybackHour(clock);
  const snapshot = clock.getSnapshot();
  const [dateDraft, setDateDraft] = useState<{ reference: string; hour: number | null } | null>(null);
  const [hover, setHover] = useState<{ left: number; hour: number } | null>(null);
  const helpId = useId();
  const reference = tracks.find((track) => track.roleId === defaultRoleId);
  const referenceKey = JSON.stringify(reference && [reference.id, reference.source.id, reference.source.revision,
    reference.branch.id, reference.branch.firstHour, reference.branch.lastHour, reference.axisOffset]);
  const [prepared, setPrepared] = useState<{ owner: string; coverage: PreparedCoverage | null }>();
  useEffect(() => {
    if (!reference || !client.preparationStatus) return;
    const controller = new AbortController(); let timer: ReturnType<typeof setTimeout> | undefined;
    // Preparation can change without notifications, so even terminal states need slow polling.
    const poll = async () => {
      const request = { sourceId: reference.source.id, revision: reference.source.revision, requestId: newId() };
      let wait = PAUSED_POLL_MS;
      try {
        const status = validatePreparationReply(await client.preparationStatus!(request, controller.signal), request);
        if (controller.signal.aborted) return;
        setPrepared({ owner: referenceKey, coverage: status.coverage?.[reference.branch.id] ?? null });
        if (preparationActive(status)) wait = 500;
      } catch {
        // Coverage keeps the last valid status until another arrives.
        if (controller.signal.aborted) return;
      }
      timer = setTimeout(() => { void poll(); }, wait);
    };
    void poll(); return () => { controller.abort(); clearTimeout(timer); };
  }, [client, referenceKey]);
  // Without a reference track the key is undefined too, so an unset state must not match it.
  const coverage = prepared && prepared.owner === referenceKey ? prepared.coverage : null;
  const draft = dateDraft && dateDraft.reference === referenceKey ? dateDraft.hour : null;
  const setDraft = (hour: number | null) => setDateDraft({ reference: referenceKey, hour });
  const rawHour = reference ? pacedHour + reference.axisOffset : null;
  const bounds = snapshot.bounds;
  const span = bounds ? bounds.lastHour - bounds.firstHour : 0;
  const gaps = useMemo(() => reference && bounds ? timelineGaps(reference.branch.gaps ?? [], reference.axisOffset, bounds.firstHour, bounds.lastHour) : [], [reference, bounds]);
  const exact = useMemo(() => reference && bounds ? timelinePrepared(coverage, reference.branch.gaps ?? [], reference.axisOffset,
    bounds.firstHour, bounds.lastHour) : [], [reference, bounds, coverage]);
  // The date follows the clock's frames until a draft replaces it.
  const axisOffset = reference?.axisOffset;
  const liveDate = useMemo(() => axisOffset == null ? undefined
    : { read: () => clock.getHour() + axisOffset, subscribe: clock.subscribeFrame }, [clock, axisOffset]);
  const timeline = useRef<HTMLInputElement>(null);
  const previous = useRef<HTMLButtonElement>(null);
  const play = useRef<HTMLButtonElement>(null);
  const next = useRef<HTMLButtonElement>(null);
  const painted = useRef('');
  const paint = useRef(() => {});
  paint.current = () => {
    const { hour, bounds } = clock.getSnapshot();
    const key = `${hour}:${bounds?.firstHour}:${bounds?.lastHour}:${referenceKey}`;
    if (key === painted.current) return;
    painted.current = key;
    const span = bounds ? bounds.lastHour - bounds.firstHour : 0;
    if (previous.current) previous.current.disabled = !bounds || hour <= bounds.firstHour;
    if (play.current) play.current.disabled = !bounds || hour >= bounds.lastHour;
    if (next.current) next.current.disabled = !bounds || hour >= bounds.lastHour;
    const input = timeline.current;
    if (!input) return;
    input.value = String(hour);
    input.style.setProperty('--scrub-progress', `${bounds && span > 0 ? 100 * (hour - bounds.firstHour) / span : 0}%`);
    const raw = reference ? hour + reference.axisOffset : null;
    const gap = raw == null ? undefined : reference?.branch.gaps?.find(item => item.firstHour <= raw && item.lastHour >= raw);
    input.setAttribute('aria-valuetext', raw == null ? 'No recording selected' : `${formatDateTime(raw)}${gap ? `; reported recording gap: ${gap.reason}` : ''}`);
  };
  // After every render, since React leaves these to the painter, and on every clock frame.
  useLayoutEffect(() => { painted.current = ''; paint.current(); });
  useLayoutEffect(() => clock.subscribeFrame(() => paint.current()), [clock]);
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
        <DateTimePicker key={referenceKey} label="Date" hideLabel value={draft ?? rawHour} live={draft == null ? liveDate : undefined} disabled={!reference}
          otherValue={null} min={reference?.branch.firstHour ?? 0} max={reference?.branch.lastHour ?? 0} describedBy={helpId} onChange={setDraft} />
        {draft != null && reference && <button type="button" className="button primary small" onClick={() => { clock.seek(draft - reference.axisOffset); setDraft(null); }}>Go</button>}
      </div>
      <div className="header-transport" aria-label="Playback controls">
        <button type="button" ref={previous} className="icon-button" onClick={() => { setDraft(null); clock.step(-1); }} aria-label="Previous hour" data-tooltip="Previous hour"><Icon name="chevrons-left" /></button>
        <button type="button" ref={play} className={`header-play${snapshot.playing ? ' is-playing' : ''}`} onClick={() => { setDraft(null); clock.setPlaying(!clock.getSnapshot().playing); }} aria-label={snapshot.playing ? 'Pause playback' : 'Play recording'} data-tooltip={snapshot.playing ? 'Pause' : 'Play'}>
          <Icon name={snapshot.playing ? 'pause' : 'play'} />
        </button>
        <button type="button" ref={next} className="icon-button" onClick={() => { setDraft(null); clock.step(1); }} aria-label="Next hour" data-tooltip="Next hour"><Icon name="chevrons-right" /></button>
      </div>
      <PlaybackSpeed value={snapshot.hoursPerSecond} onChange={(speed) => clock.setSpeed(speed)} />
      <MapMuteButton />
    </section>
    <div className="header-timeline-wrap" data-empty={!bounds || span <= 0 || undefined}>
      <div className="timeline-track">
        <input ref={timeline} className="header-timeline" type="range" aria-label="Campaign timeline" disabled={!bounds || span <= 0}
          min={bounds?.firstHour ?? 0} max={bounds?.lastHour ?? 1} step={1}
          onPointerMove={trackHover} onPointerLeave={() => setHover(null)}
          onChange={(event) => { setDraft(null); clock.seek(Number(event.currentTarget.value)); }} />
        <div className="timeline-prepared-markers" aria-label="Exact prepared hours">{exact.map((span, index) =>
          <span key={index} className="timeline-prepared" style={{ left: `${span.left}%`, width: `${span.width}%` }}
            data-tooltip={`Exact prepared hours · ${formatDateTime(span.firstHour)} to ${formatDateTime(span.lastHour)}`} />)}</div>
        <div className="timeline-gap-markers">{gaps.map((gap, index) => <span key={index} className="timeline-gap" style={{ left: `${gap.left}%`, width: `${gap.width}%` }}
          data-tooltip={`Reported recording gap · ${formatDateTime(gap.firstHour)} to ${formatDateTime(gap.lastHour)} · ${gap.reason}`} />)}</div>
        {hover && <span className="timeline-hover" style={{ left: hover.left }} aria-hidden="true">{formatDateTime(hover.hour)}</span>}
      </div>
    </div>
    <p className="sr-only" id={helpId}>Date browsing is a draft until Go. {alignment === 'anchors' ? 'The date uses the selected reference track; playback aligns by track anchors.' : 'Tracks share the game-calendar date.'} Only views following playback move with these controls.</p>
  </>;
}
