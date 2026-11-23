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
import { desktopAvailable } from '../shell/api';
import { Popover, usePopover } from '../components/Popover';
import { menuKeys } from '../components/ContextMenu';
import { commitZone } from '../shell/commitCount';
import { gapText } from '../shell/recordingNotice';

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

function MapMuteButton() {
  const { muted, error, set, retry } = useMapMuted();
  if (!desktopAvailable) return null;
  if (muted == null && !error) return <span className="header-mute-slot" aria-hidden="true" />;
  if (muted == null) return <button type="button" className="icon-button header-mute is-unavailable" onClick={retry}
    aria-label="Map audio unavailable. Try again" data-detail={error ?? undefined} data-tooltip="Map audio unavailable · Try again">
    <Icon name="volume" /><span className="header-mute-alert" aria-hidden="true" /></button>;
  const label = muted ? 'Unmute map audio' : 'Mute map audio';
  return <button type="button" className="icon-button header-mute" aria-pressed={muted} onClick={() => set(!muted)}
    aria-label={label} aria-description={error ? 'Couldn’t change map audio.' : undefined} data-detail={error ?? undefined}
    data-tooltip={error ? `${label} · Couldn’t change map audio` : label}><Icon name={muted ? 'volume-off' : 'volume'} /></button>;
}

export function LayoutMenu({ layouts, activeId, disabled, onChoose, onEdit }: {
  layouts: readonly { id: string; name: string }[]; activeId: string; disabled: boolean; onChoose: (id: string) => void; onEdit: () => void;
}) {
  const id = useId();
  const menu = usePopover();
  const active = layouts.find(layout => layout.id === activeId);
  const name = active?.name ?? 'Layout';
  return <>
    <button ref={menu.trigger} type="button" className="header-layout-trigger" popoverTarget={id} aria-haspopup="menu" aria-expanded={menu.open}
      aria-label={`Layout: ${name}`} data-tooltip-overflow={name}>
      <Icon name="layers" /><span className="header-layout-name" data-overflow-label>{name}</span><Icon name="chevron-down" className="header-layout-chevron" /></button>
    <Popover menu={menu} id={id} role="menu" className="header-layout-menu" aria-label="Layout" onKeyDownCapture={menuKeys}>
      <div className="group-label" id={`${id}-label`}>Saved layouts</div>
      <div role="group" className="header-layout-list" aria-labelledby={`${id}-label`} data-tooltip-side="left">
        {layouts.map(layout => <button key={layout.id} type="button" role="menuitemradio" className="menu-item" disabled={disabled}
          aria-checked={layout.id === activeId} data-popover-autofocus={layout.id === activeId || undefined} data-tooltip-overflow={layout.name}
          onClick={() => { menu.close(); if (layout.id !== activeId) onChoose(layout.id); }}>
          <span className="menu-item-icon-slot" aria-hidden="true" />
          <span className="menu-item-label" data-overflow-label>{layout.name}</span>{layout.id === activeId && <Icon name="check" />}</button>)}
      </div>
      <hr />
      <button type="button" role="menuitem" className="menu-item" data-popover-autofocus={!active || undefined} onClick={() => { menu.close(); onEdit(); }}>
        <Icon name="edit" /><span className="menu-item-label">Edit layout</span></button>
    </Popover>
  </>;
}

export function PlaybackStrip({ clock, tracks, alignment, defaultRoleId }: {
  clock: PlaybackController; tracks: BoundTrack[]; alignment: DashboardTemplate['alignment']; defaultRoleId: string | null;
}) {
  const controlsKey = useCallback(() => {
    const snapshot = clock.getSnapshot();
    return `${snapshot.playing}:${snapshot.hoursPerSecond}:${snapshot.bounds?.firstHour}:${snapshot.bounds?.lastHour}:${snapshot.playing ? '' : snapshot.hour}`;
  }, [clock]);
  useSyncExternalStore(clock.subscribe, controlsKey, controlsKey);
  const snapshot = clock.getSnapshot();
  const [dateDraft, setDateDraft] = useState<{ reference: string; hour: number | null } | null>(null);
  const [hover, setHover] = useState<{ left: number; hour: number; flipped: boolean } | null>(null);
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
  const axisOffset = reference?.axisOffset;
  const liveDate = useMemo(() => axisOffset == null ? undefined
    : { read: () => clock.getHour() + axisOffset, subscribe: clock.subscribeFrame, settled: clock.subscribe }, [clock, axisOffset]);
  const timeline = useRef<HTMLInputElement>(null);
  const previous = useRef<HTMLButtonElement>(null);
  const play = useRef<HTMLButtonElement>(null);
  const next = useRef<HTMLButtonElement>(null);
  const fill = useRef<HTMLSpanElement>(null);
  const knob = useRef<HTMLSpanElement>(null);
  const painted = useRef('');
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
  const settle = useRef((_anchor?: boolean) => {});
  settle.current = (anchor = false) => {
    const { hour, bounds, playing } = clock.getSnapshot();
    if (previous.current) previous.current.disabled = !bounds || hour <= bounds.firstHour;
    if (play.current) play.current.disabled = !bounds || hour >= bounds.lastHour;
    if (next.current) next.current.disabled = !bounds || hour >= bounds.lastHour;
    const input = timeline.current;
    if (!input || playing && !anchor && document.activeElement !== input) return;
    const key = `${hour}:${bounds?.firstHour}:${bounds?.lastHour}:${referenceKey}`;
    if (key === painted.current) return;
    painted.current = key;
    input.value = String(hour);
    const raw = reference ? hour + reference.axisOffset : null;
    const gap = raw == null ? undefined : reference?.branch.gaps?.find(item => item.firstHour <= raw && item.lastHour >= raw);
    input.setAttribute('aria-valuetext', raw == null ? 'No campaign' : `${formatDateTime(raw)}${gap ? '; not recorded' : ''}`);
  };
  useLayoutEffect(() => { painted.current = ''; settle.current(true); place.current(); });
  useLayoutEffect(() => {
    const stops = [clock.subscribeFrame(() => settle.current()), clock.subscribe(() => { settle.current(true); place.current(); })];
    return () => { for (const stop of stops) stop(); for (const animation of moving.current) animation.cancel(); };
  }, [clock]);
  const pointer = useRef<{ pressed: boolean; hour: number | null }>({ pressed: false, hour: null });
  const pointerAt = (event: PointerEvent<HTMLInputElement>) => {
    if (!bounds || span <= 0) return null;
    const at = timelineHourAt(event.clientX, event.currentTarget.getBoundingClientRect(), bounds.firstHour, bounds.lastHour);
    pointer.current.hour = at.hour;
    return at;
  };
  const trackHover = (event: PointerEvent<HTMLInputElement>) => {
    const at = pointerAt(event);
    if (at && reference) setHover({ left: at.left, hour: at.hour + reference.axisOffset, flipped: at.left > event.currentTarget.clientWidth - 140 });
  };
  const press = (event: PointerEvent<HTMLInputElement>) => { pointer.current.pressed = true; pointerAt(event); };
  const release = () => { pointer.current.pressed = false; };
  return <>
    <section className="header-playback" aria-label="Playback" data-empty={!reference || undefined}>
      <div className={`header-date${draft != null ? ' has-date-draft' : ''}`}>
        <DateTimePicker key={referenceKey} label="Date" hideLabel value={draft ?? rawHour} live={draft == null ? liveDate : undefined} disabled={!reference}
          otherValue={null} min={reference?.branch.firstHour ?? 0} max={reference?.branch.lastHour ?? 0} describedBy={helpId} onChange={setDraft}
          commitLabel="Go to date" onCommit={hour => { if (reference) clock.seek(hour - reference.axisOffset); setDraft(null); }} onCancel={() => setDraft(null)} />
      </div>
      <div className="header-transport" aria-label="Playback controls">
        <button type="button" ref={previous} className="icon-button" onClick={() => { setDraft(null); clock.step(-1); }} aria-label="Previous hour" data-tooltip="Previous hour"><Icon name="chevrons-left" /></button>
        <button type="button" ref={play} className={`header-play${snapshot.playing ? ' is-playing' : ''}`} onClick={() => { setDraft(null); clock.setPlaying(!clock.getSnapshot().playing); }} aria-label={snapshot.playing ? 'Pause' : 'Play'} data-tooltip={snapshot.playing ? 'Pause' : 'Play'}>
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
          data-tooltip={gapText(gap)} />)}</div>
        {hover && <span className={`timeline-hover${hover.flipped ? ' is-flipped' : ''}`} style={{ left: hover.left }} aria-hidden="true">{formatDateTime(hover.hour)}</span>}
      </div>
    </div>
    <p className="sr-only" id={helpId}>A date picked in the calendar is a draft until Go to date, or Enter on a day. {alignment === 'anchors' ? 'The date uses the selected reference track; playback aligns by track anchors.' : 'Tracks share the game-calendar date.'} Only views following playback move with these controls.</p>
  </>;
}

commitZone(PlaybackStrip, 'playback strip');
commitZone(PlaybackCountryPicker, 'country picker');
