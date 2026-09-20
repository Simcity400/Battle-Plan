import { useCallback, useId, useMemo, useState, useSyncExternalStore } from 'react';
import type { CSSProperties } from 'react';
import DateTimePicker from '../analytics/DateTimePicker';
import PlaybackSpeed from './PlaybackSpeed';
import { formatDateTime } from '../analytics/calendar';
import { metrics, parseRange } from '../analytics/model';
import { recordingName } from '../api';
import { Icon } from '../Icons';
import { pinTime, resolveWindow } from './model';
import type { BoundTrack, DashboardTemplate, WidgetSpec, WidgetTime } from './model';
import type { PlaybackController } from './playback';
import { widgetDefinition } from './definitions';
import { resolveWidgetCampaign } from './campaigns';
import { timelineGaps } from './timelineGaps';

export const timeLabels: Record<WidgetTime['mode'], string> = {
  follow: 'Follow playback', campaign: 'Whole campaign', range: 'Selected period', pin: 'Pinned',
};

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

export function WidgetSettings({ widget, template, tracks, clock, onApply, onRemove, onLive, defaultRoleId }: {
  widget: WidgetSpec; template: DashboardTemplate; tracks: BoundTrack[]; clock: PlaybackController; onApply: (widget: WidgetSpec) => void; onRemove: () => void;
  onLive: (time: WidgetTime) => void; defaultRoleId: string | null;
}) {
  const [draft, setDraft] = useState(widget);
  const effective = resolveWidgetCampaign(draft, template, defaultRoleId);
  const reference = tracks.find((track) => track.id === effective.trackIds[0]);
  const initial = reference ? resolveWindow(resolveWidgetCampaign(widget, template, defaultRoleId), reference, tracks, clock.getHour()) : null;
  const [from, setFrom] = useState<number | null>(initial?.firstHour ?? reference?.branch.firstHour ?? null);
  const [to, setTo] = useState<number | null>(initial?.lastHour ?? reference?.branch.lastHour ?? null);
  const [tagText, setTagText] = useState(widget.countries.kind === 'tags' ? widget.countries.tags.join(', ') : '');
  const help = useId();
  const range = reference ? parseRange(String(from ?? ''), String(to ?? ''), reference.branch) : null;
  const rangeError = draft.time.mode === 'range' && (!range || typeof range === 'string') ? typeof range === 'string' ? range : 'Connect the first track before choosing a period.' : null;
  const tags = tagText.split(/[\s,]+/).map((tag) => tag.trim()).filter(Boolean);
  const error = !draft.title.trim() ? 'Give this view a title.' : rangeError ?? (draft.countries.kind === 'tags' && !tags.length ? 'Choose at least one country tag.' : null);
  const definition = widgetDefinition(widget.kind);
  const linked = widget.time.mode === 'follow';
  return <form className="widget-settings-content" onSubmit={(event) => {
    event.preventDefault();
    if (error) return;
    let time = draft.time;
    if (time.mode === 'range' && reference && range && typeof range !== 'string') time = { mode: 'range', firstOffset: range.firstHour - reference.branch.firstHour, lastOffset: range.lastHour - reference.branch.firstHour };
    onApply({ ...draft, ...(time.mode === 'pin' ? { campaign: 'tracks', trackIds: effective.trackIds.length ? effective.trackIds : draft.trackIds } : {}), title: draft.title.trim(), time,
      countries: draft.countries.kind === 'tags' ? { kind: 'tags', tags: [...new Set(tags)] } : draft.countries });
  }}>
    <button type="button" className="widget-live-toggle" aria-pressed={linked} disabled={!definition?.timeModes.includes('follow') || !definition.timeModes.includes('campaign')}
      onClick={() => { const time: WidgetTime = { mode: linked ? 'campaign' : 'follow' }; setDraft({ ...draft, time }); onLive(time); }}>
      <Icon name="link" /><span>Live playback</span><strong>{linked ? 'On' : 'Off'}</strong></button>
    <label className="field"><span>Title</span><input value={draft.title} maxLength={160} onChange={(event) => setDraft({ ...draft, title: event.currentTarget.value })} /></label>
    <label className="field"><span>Campaign</span><select aria-label="Widget campaign" value={draft.campaign === 'default' ? '__default' : template.tracks.find((track) => track.id === draft.trackIds[0])?.roleId ?? '__missing'} onChange={(event) => {
      const roleId = event.currentTarget.value;
      const track = template.tracks.find((item) => item.roleId === roleId);
      setDraft({ ...draft, campaign: roleId === '__default' ? 'default' : 'tracks', trackIds: track ? [track.id] : draft.trackIds,
        time: draft.time.mode === 'pin' ? { mode: definition?.defaultTime ?? 'campaign' } : draft.time });
    }}>
      <option value="__default">Default campaign</option>
      {template.roles.map((role) => <option key={role.id} value={role.id}>{tracks.find((track) => track.roleId === role.id) ? recordingName(tracks.find((track) => track.roleId === role.id)!.source.path) : `${role.label} - unavailable`}</option>)}
    </select></label>
    {draft.kind !== 'placeholder' && <details className="widget-advanced"><summary>More options</summary><div>
    {draft.kind !== 'placeholder' && <fieldset className="dash-fieldset"><legend>{draft.kind === 'map' ? 'Map track' : 'Compare tracks'}</legend>
      {template.tracks.map((track) => <label className="dash-check" key={track.id}>
        <input type={draft.kind === 'map' ? 'radio' : 'checkbox'} name="widget-tracks" checked={effective.trackIds.includes(track.id)} onChange={(event) => {
          const trackIds = draft.kind === 'map' ? [track.id] : event.currentTarget.checked ? [...effective.trackIds, track.id] : effective.trackIds.filter((id) => id !== track.id);
          if (trackIds.length) setDraft({ ...draft, campaign: 'tracks', trackIds });
        }} /><span>{track.label}<small>{tracks.some((item) => item.id === track.id) ? template.roles.find((role) => role.id === track.roleId)?.label : 'Source, branch or anchor unavailable'}</small></span>
      </label>)}
    </fieldset>}
    {draft.kind !== 'map' && draft.kind !== 'placeholder' && <>
      <label className="field"><span>Countries</span><select value={draft.countries.kind} onChange={(event) => setDraft({ ...draft, countries: event.currentTarget.value === 'tags'
        ? { kind: 'tags', tags } : { kind: event.currentTarget.value as 'selected' | 'all' } })}>
        <option value="selected">Selected country from each source</option><option value="all">All observed countries</option><option value="tags">Choose country tags</option>
      </select></label>
      {draft.countries.kind === 'tags' && <label className="field"><span>Country tags, separated by commas</span><input value={tagText} placeholder="GER, ENG, SOV" maxLength={2048} onChange={(event) => setTagText(event.currentTarget.value.toUpperCase())} /></label>}
      <p className="dash-help">Country selections rebind when a recording changes. Missing countries remain explicitly unavailable.</p>
    </>}
    {draft.kind === 'history' && <label className="field"><span>Metric</span><select value={draft.metric} onChange={(event) => setDraft({ ...draft, metric: event.currentTarget.value as WidgetSpec['metric'] })}>
      {metrics.map((metric) => <option key={metric.key} value={metric.key}>{metric.label}</option>)}
    </select></label>}
    {draft.kind !== 'placeholder' && <><label className="field"><span>Time mode</span><select value={draft.time.mode} onChange={(event) => {
      const mode = event.currentTarget.value as WidgetTime['mode'];
      const candidate = effective.time.mode === 'range' && reference && range && typeof range !== 'string'
        ? { ...effective, time: { mode: 'range' as const, firstOffset: range.firstHour - reference.branch.firstHour, lastOffset: range.lastHour - reference.branch.firstHour } }
        : effective;
      const time: WidgetTime = mode === 'pin' ? pinTime(candidate, tracks, clock.getHour())
        : mode === 'range' ? { mode, firstOffset: 0, lastOffset: reference ? reference.branch.lastHour - reference.branch.firstHour : 0 } : { mode };
      if (mode === 'range' && reference) { setFrom(reference.branch.firstHour); setTo(reference.branch.lastHour); }
      setDraft({ ...draft, time });
    }}>{Object.entries(timeLabels).filter(([mode]) => widgetDefinition(draft.kind)?.timeModes.includes(mode as WidgetTime['mode']) || mode === draft.time.mode)
      .map(([mode, label]) => <option key={mode} value={mode}>{label}</option>)}</select></label>
    <p className="dash-help">{draft.time.mode === 'follow' ? draft.kind === 'history' ? 'Shows the trailing seven days ending at shared playback, with exact hourly buckets.' : 'Shows the state at shared playback.'
      : draft.time.mode === 'campaign' ? draft.kind === 'history' ? 'Keeps the whole branch in view while playback continues.' : 'Shows the state at the branch’s final hour.'
        : draft.time.mode === 'pin' ? 'Keeps the currently resolved window for each selected track. Changing playback will not change its data.'
          : draft.kind === 'history' ? 'Uses this fixed period. Date browsing does not issue queries.' : 'Shows the state at the ending hour of this fixed period.'}</p>
    {draft.time.mode === 'range' && <div className="dash-range-fields">
      <DateTimePicker label="From" value={from} otherValue={to} min={reference?.branch.firstHour ?? 0} max={reference?.branch.lastHour ?? 0}
        disabled={!reference} onChange={setFrom} invalid={!!rangeError} describedBy={help} />
      <DateTimePicker label="To" value={to} otherValue={from} min={reference?.branch.firstHour ?? 0} max={reference?.branch.lastHour ?? 0}
        disabled={!reference} onChange={setTo} invalid={!!rangeError} describedBy={help} />
      <p id={help} className="dash-help">Dates use {reference?.label ?? 'the first selected track'}. Both endpoints are inclusive.</p>
    </div>}
    {draft.time.mode === 'pin' && <button type="button" className="button small" disabled={!reference} onClick={() => setDraft({ ...draft, time: pinTime({ ...effective, time: { mode: 'follow' } }, tracks, clock.getHour()) })}>Pin current playback time</button>}
    </>}
    </div></details>}
    {error && <p className="dash-inline-warning" role="alert">{error}</p>}
    <div className="widget-settings-actions"><button type="button" className="button quiet" onClick={onRemove}>Remove widget</button>
      <button type="submit" className="button primary" disabled={!!error}>Apply</button></div>
  </form>;
}
