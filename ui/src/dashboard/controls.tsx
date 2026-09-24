import { useCallback, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { ComponentProps, CSSProperties, PointerEvent } from 'react';
import DateTimePicker from '../components/DateTimePicker';
import PlaybackSpeed from '../components/PlaybackSpeed';
import { formatDate, formatDateTime, gameDate } from './calendar';
import { Icon } from '../components/Icons';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model';
import { WIDGET_CONTENT_SCALES } from './model';
import { usePlaybackHour } from './playback';
import type { PlaybackController } from './playback';
import { widgetDefinition } from './definitions';
import { resolveWidgetCampaign } from './campaigns';
import { timelineGaps } from './timelineGaps';
import CountryPicker from '../components/CountryPicker';
import type { WidgetRuntime } from './definitions';
import CampaignPicker from './CampaignPicker';


/** The header's country picker, showing the flag the game drew at the playback hour.
 * Only this component follows the clock; while playing it asks at most every 250 ms. */
export function PlaybackCountryPicker({ clock, track, ...picker }: { clock: PlaybackController; track: BoundTrack | undefined } & Omit<ComponentProps<typeof CountryPicker>, 'flagAt'>) {
  const hour = usePlaybackHour(clock);
  const [sampled, setSampled] = useState(() => clock.getHour());
  const sent = useRef(0);
  useEffect(() => {
    const wait = clock.getSnapshot().playing ? Math.max(0, 250 - (performance.now() - sent.current)) : 0;
    const timer = setTimeout(() => { sent.current = performance.now(); setSampled(clock.getHour()); }, wait);
    return () => clearTimeout(timer);
  }, [hour, clock]);
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
  const edge = (hour: number) => { const date = gameDate(hour); return date ? formatDate(date) : ''; };
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
        <button type="button" className="icon-button" disabled={!bounds || snapshot.hour <= bounds.firstHour} onClick={() => { setDraft(null); clock.seek(snapshot.hour - 1); }} aria-label="Previous hour" data-tooltip="Previous hour"><Icon name="chevrons-left" /></button>
        <button type="button" className={`header-play${snapshot.playing ? ' is-playing' : ''}`} disabled={!bounds || snapshot.hour >= bounds.lastHour} onClick={() => { setDraft(null); clock.setPlaying(!snapshot.playing); }} aria-label={snapshot.playing ? 'Pause playback' : 'Play recording'} data-tooltip={snapshot.playing ? 'Pause' : 'Play'}>
          <Icon name={snapshot.playing ? 'pause' : 'play'} />
        </button>
        <button type="button" className="icon-button" disabled={!bounds || snapshot.hour >= bounds.lastHour} onClick={() => { setDraft(null); clock.seek(snapshot.hour + 1); }} aria-label="Next hour" data-tooltip="Next hour"><Icon name="chevrons-right" /></button>
      </div>
      <PlaybackSpeed value={snapshot.hoursPerSecond} onChange={(speed) => clock.setSpeed(speed)} />
    </section>
    <div className="header-timeline-wrap" data-empty={!bounds || span <= 0 || undefined}>
      <span className="timeline-edge">{bounds && reference ? edge(bounds.firstHour + reference.axisOffset) : ''}</span>
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
      <span className="timeline-edge">{bounds && reference ? edge(bounds.lastHour + reference.axisOffset) : ''}</span>
    </div>
    <p className="sr-only" id={helpId}>Date browsing is a draft until Go. {alignment === 'anchors' ? 'The date uses the selected reference track; playback aligns by track anchors.' : 'Tracks share the game-calendar date.'} Only views following playback move with these controls.</p>
  </>;
}

export function WidgetSettings({ widget, template, runtime, onChange }: {
  widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onChange: (widget: WidgetSpec, view?: boolean) => void;
}) {
  const { tracks, client, defaultRoleId } = runtime;
  // Keep incomplete title edits local; other controls always use current settings.
  const [title, setTitle] = useState(widget.title);
  useEffect(() => setTitle(widget.title), [widget.title]);
  const effective = resolveWidgetCampaign(widget, template, defaultRoleId);
  const reference = tracks.find(track => track.id === effective.trackIds[0]);
  const defaultCountryTag = reference?.selectedTag ?? reference?.source.metadata.countries.find(country => country.id === reference.source.metadata.playerCountryId)?.tag ?? null;
  const definition = widgetDefinition(widget.kind);
  const scale = widget.contentScale ?? 100;
  const step = (direction: 1 | -1) => {
    const index = WIDGET_CONTENT_SCALES.findIndex(value => value >= scale);
    const next = WIDGET_CONTENT_SCALES[Math.min(WIDGET_CONTENT_SCALES.length - 1, Math.max(0, (index < 0 ? WIDGET_CONTENT_SCALES.length - 1 : index) + direction))];
    if (next !== scale) onChange({ ...widget, contentScale: next }, true);
  };
  return <div className="widget-settings-content">
    <div className="setting-row"><span>Size<small>Make this widget's contents more compact or larger</small></span>
      <div className="size-stepper" role="group" aria-label="Widget size">
        <button type="button" className="icon-button small" aria-label="Smaller" disabled={scale <= WIDGET_CONTENT_SCALES[0]} onClick={() => step(-1)}><Icon name="minus" /></button>
        <button type="button" className="size-stepper-value" aria-label={`Size ${scale}%, reset to 100%`} data-tooltip="Reset to 100%" onClick={() => onChange({ ...widget, contentScale: 100 }, true)}>{scale}%</button>
        <button type="button" className="icon-button small" aria-label="Larger" disabled={scale >= WIDGET_CONTENT_SCALES.at(-1)!} onClick={() => step(1)}><Icon name="plus" /></button>
      </div></div>
    <label className="field"><span>Title</span><input value={title} maxLength={160} onBlur={() => setTitle(widget.title)} onChange={event => {
      const value = event.currentTarget.value;
      setTitle(value);
      if (value.trim()) onChange({ ...widget, title: value.trim() });
    }} /></label>
    {definition?.data !== false && <CampaignPicker widget={widget} template={template} runtime={runtime} />}
    {definition?.data !== false && <CountryPicker field key={reference?.source.id ?? 'unbound'} source={reference?.source} client={client}
      countries={reference?.source.metadata.countries ?? []} playerCountryId={reference?.source.metadata.playerCountryId ?? null}
      selectedTag={widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : defaultCountryTag}
      onChange={tag => onChange({ ...widget, countries: tag && tag !== defaultCountryTag ? { kind: 'tags', tags: [tag] } : { kind: 'selected' } })} />}
  </div>;
}
