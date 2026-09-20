import { useCallback, useId, useMemo, useState, useSyncExternalStore } from 'react';
import type { CSSProperties } from 'react';
import DateTimePicker from '../analytics/DateTimePicker';
import PlaybackSpeed from './PlaybackSpeed';
import { formatDateTime } from '../analytics/calendar';
import { recordingName } from '../api';
import { Icon } from '../Icons';
import type { BoundTrack, DashboardTemplate, WidgetSpec, WidgetTime } from './model';
import type { PlaybackController } from './playback';
import { widgetDefinition } from './definitions';
import { resolveWidgetCampaign } from './campaigns';
import { timelineGaps } from './timelineGaps';
import CountryPicker from './CountryPicker';
import type { DashboardClient } from './client';


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
  return <section className="header-playback" aria-label="Campaign and playback">
    <div className="header-time-section"><div className="header-time-controls">
    <div className={`header-date${draft != null ? ' has-date-draft' : ''}`}>
      <DateTimePicker key={referenceKey} label="Date" hideLabel value={draft ?? rawHour} disabled={!reference}
        otherValue={null} min={reference?.branch.firstHour ?? 0} max={reference?.branch.lastHour ?? 0} describedBy={helpId} onChange={setDraft} />
      {draft != null && reference && <button type="button" className="button small" onClick={() => { clock.seek(draft - reference.axisOffset); setDraft(null); }}>Go</button>}
    </div>
    <div className="header-transport" aria-label="Playback controls">
      <button type="button" disabled={!bounds || snapshot.hour <= bounds.firstHour} onClick={() => { setDraft(null); clock.seek(snapshot.hour - 1); }} aria-label="Previous hour" data-tooltip="Previous hour"><Icon name="chevron-left" /></button>
      <button type="button" className="header-play" disabled={!bounds || snapshot.hour >= bounds.lastHour} onClick={() => { setDraft(null); clock.setPlaying(!snapshot.playing); }} aria-label={snapshot.playing ? 'Pause playback' : 'Play recording'} data-tooltip={snapshot.playing ? 'Pause playback' : 'Play recording'}>
        <svg viewBox="0 0 24 24" aria-hidden="true">{snapshot.playing ? <path d="M7 5h4v14H7zm6 0h4v14h-4z" /> : <path d="m8 4 12 8-12 8z" />}</svg>
      </button>
      <button type="button" disabled={!bounds || snapshot.hour >= bounds.lastHour} onClick={() => { setDraft(null); clock.seek(snapshot.hour + 1); }} aria-label="Next hour" data-tooltip="Next hour"><Icon name="chevron-right" /></button>
    </div>
    <PlaybackSpeed value={snapshot.hoursPerSecond} onChange={(speed) => clock.setSpeed(speed)} />
    </div><div className="header-timeline-wrap"><input className="header-timeline" type="range" aria-label="Campaign timeline" disabled={!bounds || span <= 0}
      min={bounds?.firstHour ?? 0} max={bounds?.lastHour ?? 1} step={1} value={snapshot.hour}
      aria-valuetext={rawHour == null ? 'No recording selected' : `${formatDateTime(rawHour)}${currentGap ? `; reported recording gap: ${currentGap.reason}` : ''}`}
      data-tooltip={rawHour == null ? 'Open a recording to scrub the timeline' : formatDateTime(rawHour)}
      style={{ '--scrub-progress': `${progress}%` } as CSSProperties}
      onChange={(event) => { setDraft(null); clock.seek(Number(event.currentTarget.value)); }} />
      <div className="timeline-gap-markers">{gaps.map((gap, index) => <span key={index} className="timeline-gap" style={{ left: `${gap.left}%`, width: `${gap.width}%` }}
        data-tooltip={`Reported recording gap · ${formatDateTime(gap.firstHour)} to ${formatDateTime(gap.lastHour)} · ${gap.reason}`} />)}</div></div></div>
    <p className="sr-only" id={helpId}>Date browsing is a draft until Go. {alignment === 'anchors' ? 'The date uses the selected reference track; playback aligns by track anchors.' : 'Tracks share the game-calendar date.'} Only views following playback move with these controls.</p>
  </section>;
}

export function WidgetSettings({ widget, template, tracks, client, onApply, onRemove, onLive, defaultRoleId }: {
  widget: WidgetSpec; template: DashboardTemplate; tracks: BoundTrack[]; client: DashboardClient;
  onApply: (widget: WidgetSpec) => void; onRemove: () => void;
  onLive: (time: WidgetTime) => void; defaultRoleId: string | null;
}) {
  const [draft, setDraft] = useState(widget);
  const effective = resolveWidgetCampaign(draft, template, defaultRoleId);
  const reference = tracks.find(track => track.id === effective.trackIds[0]);
  const defaultCountryTag = reference?.selectedTag ?? reference?.source.metadata.countries.find(country => country.id === reference.source.metadata.playerCountryId)?.tag ?? null;
  const definition = widgetDefinition(widget.kind);
  const linked = widget.time.mode === 'follow';
  const error = !draft.title.trim() ? 'Give this view a title.' : null;
  return <form className="widget-settings-content" onSubmit={event => {
    event.preventDefault();
    if (!error) onApply({ ...draft, title: draft.title.trim() });
  }}>
    <button type="button" className="widget-live-toggle" aria-pressed={linked} disabled={!definition?.timeModes.includes('follow') || !definition.timeModes.includes('campaign')}
      onClick={() => { const time: WidgetTime = { mode: linked ? 'campaign' : 'follow' }; setDraft({ ...draft, time }); onLive(time); }}>
      <Icon name="link" /><span>Live playback</span><strong>{linked ? 'On' : 'Off'}</strong></button>
    <label className="field"><span>Title</span><input value={draft.title} maxLength={160} onChange={event => setDraft({ ...draft, title: event.currentTarget.value })} /></label>
    <label className="field"><span>Campaign</span><select aria-label="Widget campaign" value={draft.campaign === 'default' ? '__default' : template.tracks.find(track => track.id === draft.trackIds[0])?.roleId ?? '__missing'} onChange={event => {
      const roleId = event.currentTarget.value;
      const track = template.tracks.find(item => item.roleId === roleId);
      setDraft({ ...draft, campaign: roleId === '__default' ? 'default' : 'tracks', trackIds: track ? [track.id] : draft.trackIds,
        time: draft.time.mode === 'pin' ? { mode: definition?.defaultTime ?? 'campaign' } : draft.time });
    }}>
      <option value="__default">Default campaign</option>
      {template.roles.map(role => <option key={role.id} value={role.id}>{tracks.find(track => track.roleId === role.id) ? recordingName(tracks.find(track => track.roleId === role.id)!.source.path) : `${role.label} - unavailable`}</option>)}
    </select></label>
    {draft.kind !== 'placeholder' && <CountryPicker field key={reference?.source.id ?? 'unbound'} source={reference?.source} client={client}
      countries={reference?.source.metadata.countries ?? []} playerCountryId={reference?.source.metadata.playerCountryId ?? null}
      selectedTag={draft.countries.kind === 'tags' ? draft.countries.tags[0] ?? null : defaultCountryTag}
      onChange={tag => setDraft({ ...draft, countries: tag && tag !== defaultCountryTag ? { kind: 'tags', tags: [tag] } : { kind: 'selected' } })} />}
    {error && <p className="dash-inline-warning" role="alert">{error}</p>}
    <div className="widget-settings-actions"><button type="button" className="button quiet" aria-label="Delete widget" data-tooltip="Delete widget" onClick={onRemove}><Icon name="trash" /></button>
      <button type="submit" className="button primary" aria-label="Apply widget settings" data-tooltip="Apply" disabled={!!error}><Icon name="check" /></button></div>
  </form>;
}
