import { useCallback, useId, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { ComponentProps, PointerEvent } from 'react';
import DateTimePicker from '../components/DateTimePicker';
import PlaybackSpeed from '../components/PlaybackSpeed';
import { formatDateTime } from './calendar';
import { Icon } from '../components/Icons';
import type { BoundTrack, DashboardTemplate } from './model';
import type { PlaybackController } from './playback';
import { useAtHead, useIdentityHour } from './atHead.ts';
import { timelineGaps, timelineHourAt } from './timelineGaps';
import CountryPicker from '../components/CountryPicker';
import { countryNameAt, useSlotTags } from './countrySlots.ts';
import { useMapMuted } from '../shell/muteSetting';
import { commitZone } from '../shell/commitCount';

/** The header's country picker, showing the flag the game drew at the playback hour. The flag is read
 * at the hour it last changed (`useIdentityHour`), so the picker renders only when it does. The chosen country is its
 * slot (`selectedSlot`), named by the tag it goes by at the playback hour. */
export function PlaybackCountryPicker({ clock, track, selectedSlot, ...picker }: { clock: PlaybackController; track: BoundTrack | undefined; selectedSlot: number | null }
  & Omit<ComponentProps<typeof CountryPicker>, 'flagAt'>) {
  const first = track?.branch.firstHour, last = track?.branch.lastHour, offset = track?.axisOffset;
  const trackHour = useCallback((hour: number) => first === undefined || last === undefined || offset === undefined
    || hour + offset < first || hour + offset > last ? null : hour + offset, [first, last, offset]);
  const slots = useSlotTags(picker.client, track, !!track && selectedSlot !== null);
  const shown = useAtHead(clock, !!track && selectedSlot !== null, hour => {
    const at = trackHour(hour);
    return at === null ? null : countryNameAt(slots, selectedSlot, at);
  });
  const selectedTag = selectedSlot === null ? picker.selectedTag : shown ?? picker.selectedTag;
  const tags = useMemo(() => selectedTag ? [selectedTag] : [], [selectedTag]);
  const raw = useIdentityHour(picker.client, track, tags, clock, !!track, trackHour);
  const flagAt = useMemo(() => track && raw != null ? { branchId: track.branch.id, hour: raw } : null, [track?.branch.id, raw]);
  return <CountryPicker {...picker} selectedTag={selectedTag} flagAt={flagAt} />;
}

/** Mutes or restores the native maps' audio; only the focused map sounds. */
function MapMuteButton() {
  const { muted, error, set } = useMapMuted();
  if (muted == null && !error) return null;
  const label = muted ? 'Unmute map audio' : 'Mute map audio';
  return <button type="button" className="icon-button" aria-pressed={!!muted} onClick={() => set(!muted)}
    aria-label={label} data-tooltip={error ?? label}><Icon name={muted ? 'volume-off' : 'volume'} /></button>;
}

export function PlaybackStrip({ clock, tracks, alignment, defaultRoleId }: {
  clock: PlaybackController; tracks: BoundTrack[]; alignment: DashboardTemplate['alignment']; defaultRoleId: string | null;
}) {
  // React renders the strip on anchors only; the playhead, the date and the hour-bound controls are
  // painted from the clock's frames (`place` and `settle` below), and the calendar takes the anchor's hour.
  const controlsKey = useCallback(() => {
    const snapshot = clock.getSnapshot();
    return `${snapshot.playing}:${snapshot.hoursPerSecond}:${snapshot.bounds?.firstHour}:${snapshot.bounds?.lastHour}:${snapshot.playing ? '' : snapshot.hour}`;
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
  const gaps = useMemo(() => reference && bounds ? timelineGaps(reference.branch.gaps ?? [], reference.axisOffset, bounds.firstHour, bounds.lastHour) : [], [reference, bounds]);
  // The date follows the clock's frames until a draft replaces it.
  const axisOffset = reference?.axisOffset;
  const liveDate = useMemo(() => axisOffset == null ? undefined
    : { read: () => clock.getHour() + axisOffset, subscribe: clock.subscribeFrame }, [clock, axisOffset]);
  const timeline = useRef<HTMLInputElement>(null);
  const previous = useRef<HTMLButtonElement>(null);
  const play = useRef<HTMLButtonElement>(null);
  const next = useRef<HTMLButtonElement>(null);
  const fill = useRef<HTMLSpanElement>(null);
  const knob = useRef<HTMLSpanElement>(null);
  const painted = useRef('');
  // The playhead moves as the clock does, linearly between anchors: one compositor animation per
  // anchor carries it to the end at the playing speed, so no frame touches the page for it.
  const placed = useRef('');
  const moving = useRef<Animation[]>([]);
  const place = useRef(() => {});
  place.current = () => {
    const { playing, hoursPerSecond, bounds, seekSequence, position } = clock.getSnapshot();
    const key = `${playing}:${hoursPerSecond}:${seekSequence}:${bounds?.firstHour}:${bounds?.lastHour}:${playing ? '' : position}`;
    if (key === placed.current || !fill.current || !knob.current) return;
    placed.current = key;
    for (const animation of moving.current) animation.cancel();
    moving.current = [];
    const span = bounds ? bounds.lastHour - bounds.firstHour : 0;
    const at = clock.positionAt();
    const from = bounds && span > 0 ? Math.min(1, Math.max(0, (at - bounds.firstHour) / span)) : 0;
    const frames = (progress: number) => [{ transform: `scaleX(${progress})` }, { transform: `translateX(${(progress - 1) * 100}%)` }] as const;
    const [fillFrom, knobFrom] = frames(from);
    fill.current.style.transform = fillFrom.transform;
    knob.current.style.transform = knobFrom.transform;
    if (!playing || !bounds || span <= 0 || from >= 1 || typeof fill.current.animate !== 'function') return;
    const timing: KeyframeAnimationOptions = { duration: (bounds.lastHour - at) / hoursPerSecond * 1000, easing: 'linear', fill: 'forwards' };
    const [fillTo, knobTo] = frames(1);
    moving.current = [fill.current.animate([fillFrom, fillTo], timing), knob.current.animate([knobFrom, knobTo], timing)];
  };
  // The slider's value, its spoken date and the hour-bound buttons follow the clock's frames.
  const settle = useRef(() => {});
  settle.current = () => {
    const { hour, bounds } = clock.getSnapshot();
    const key = `${hour}:${bounds?.firstHour}:${bounds?.lastHour}:${referenceKey}`;
    if (key === painted.current) return;
    painted.current = key;
    if (previous.current) previous.current.disabled = !bounds || hour <= bounds.firstHour;
    if (play.current) play.current.disabled = !bounds || hour >= bounds.lastHour;
    if (next.current) next.current.disabled = !bounds || hour >= bounds.lastHour;
    const input = timeline.current;
    if (!input) return;
    input.value = String(hour);
    const raw = reference ? hour + reference.axisOffset : null;
    const gap = raw == null ? undefined : reference?.branch.gaps?.find(item => item.firstHour <= raw && item.lastHour >= raw);
    input.setAttribute('aria-valuetext', raw == null ? 'No recording selected' : `${formatDateTime(raw)}${gap ? `; reported recording gap: ${gap.reason}` : ''}`);
  };
  // After every render, since React leaves these to the painters, and on the clock's frames and anchors.
  useLayoutEffect(() => { painted.current = ''; settle.current(); place.current(); });
  useLayoutEffect(() => {
    const stops = [clock.subscribeFrame(() => settle.current()), clock.subscribe(() => place.current())];
    return () => { for (const stop of stops) stop(); for (const animation of moving.current) animation.cancel(); };
  }, [clock]);
  // A pressed pointer seeks to the hour under it, the hour its tooltip names; the keyboard to the range's value.
  const pointer = useRef<{ pressed: boolean; hour: number | null }>({ pressed: false, hour: null });
  const pointerAt = (event: PointerEvent<HTMLInputElement>) => {
    if (!bounds || span <= 0) return null;
    const at = timelineHourAt(event.clientX, event.currentTarget.getBoundingClientRect(), bounds.firstHour, bounds.lastHour);
    pointer.current.hour = at.hour;
    return at;
  };
  const trackHover = (event: PointerEvent<HTMLInputElement>) => {
    const at = pointerAt(event);
    if (at && reference) setHover({ left: at.left, hour: at.hour + reference.axisOffset });
  };
  const press = (event: PointerEvent<HTMLInputElement>) => { pointer.current.pressed = true; pointerAt(event); };
  const release = () => { pointer.current.pressed = false; };
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
          onPointerMove={trackHover} onPointerLeave={() => setHover(null)} onPointerDown={press} onPointerUp={release} onPointerCancel={release}
          onChange={(event) => { setDraft(null); clock.seek(pointer.current.pressed && pointer.current.hour !== null ? pointer.current.hour : Number(event.currentTarget.value)); }} />
        <div className="timeline-progress" aria-hidden="true"><span ref={fill} className="timeline-fill" /><span ref={knob} className="timeline-knob" /></div>
        <div className="timeline-gap-markers">{gaps.map((gap, index) => <span key={index} className="timeline-gap" style={{ left: `${gap.left}%`, width: `${gap.width}%` }}
          data-tooltip={`Reported recording gap · ${formatDateTime(gap.firstHour)} to ${formatDateTime(gap.lastHour)} · ${gap.reason}`} />)}</div>
        {hover && <span className="timeline-hover" style={{ left: hover.left }} aria-hidden="true">{formatDateTime(hover.hour)}</span>}
      </div>
    </div>
    <p className="sr-only" id={helpId}>Date browsing is a draft until Go. {alignment === 'anchors' ? 'The date uses the selected reference track; playback aligns by track anchors.' : 'Tracks share the game-calendar date.'} Only views following playback move with these controls.</p>
  </>;
}

commitZone(PlaybackStrip, 'playback strip');
commitZone(PlaybackCountryPicker, 'country picker');
