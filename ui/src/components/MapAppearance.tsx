import { useEffect, useRef, useState } from 'react';
import { errorMessage } from '../shell/api';
import { Icon } from './Icons';
import VideoControls, { useVideoSettings } from '../shell/VideoSettings';
import './mapControls.css';
import type { CampaignMapSettings, DashboardClient, DashboardSource, MapGraphics, MapSettingsReply } from '../dashboard/client';

const ANTI_ALIASING = 'Anti-aliasing';
const TEXTURE_QUALITY = 'Texture quality';
const graphics: [Exclude<keyof MapGraphics, "samples" | "textureQuality">, string][] = [
  ['fullResolution', 'Full map resolution'],
  ['dayNight', 'Day/night cycle'],
  ['natoCounters', 'NATO counters'],
  ['showAlliedPlans', 'Allied battle plans'],
  ['highGfxShaders', 'High-quality shaders'], ['waterReflections', 'Water reflections'], ['trees', 'Trees'], ['shadows', 'Shadows'], ['rivers', 'Rivers'], ['citySprawl', 'City sprawl'],
  ['buildings', '3D buildings'], ['units', '3D units'], ['weather', 'Weather effects'], ['sky', 'Sky'],
];
/** Every option row's label, in order; settings search reveals rows by these. */
export const MAP_OPTION_LABELS = [ANTI_ALIASING, TEXTURE_QUALITY, ...graphics.map(([, label]) => label)];

export default function MapAppearance({ source, client, global = false, viewId, userFolder }: { source: DashboardSource | null; client: DashboardClient; global?: boolean; viewId?: string; userFolder?: string }) {
  const video = useVideoSettings(global, userFolder);
  const target = global ? { scope: 'global' as const } : viewId ? { scope: 'widget' as const, id: viewId } : { scope: 'campaign' as const };
  const available = global || !!source;
  const parentLabel = global ? 'the game settings' : viewId ? 'campaign settings' : 'global defaults';
  const scopeLabel = global ? 'Graphics' : viewId ? 'Widget graphics' : 'Campaign graphics';
  const [reply, setReply] = useState<MapSettingsReply | null>(null);
  const [draft, setDraft] = useState<CampaignMapSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [retry, setRetry] = useState(0);
  const [query, setQuery] = useState('');
  useEffect(() => {
    const controller = new AbortController();
    setError(null); setReply(null); setDraft(null);
    if (!available) return;
    if (!client.mapSettings) { setError('Map settings are unavailable in this version.'); return; }
    void client.mapSettings(source?.id ?? '', source?.revision ?? '', null, controller.signal, target).then(value => {
      if (!controller.signal.aborted) { setReply(value); setDraft(value.settings); }
    }).catch(cause => { if (!controller.signal.aborted) setError(errorMessage(cause)); });
    return () => controller.abort();
  }, [client, source?.id, source?.revision, global, viewId, retry]);
  const saveRequest = useRef<AbortController | null>(null);
  useEffect(() => () => saveRequest.current?.abort(), []);
  const mapDirty = !!reply && !!draft && JSON.stringify(reply.settings) !== JSON.stringify(draft);
  const dirty = mapDirty || (global && video.dirty);
  const selected = draft?.visualMods ?? reply?.inheritedSettings.visualMods ?? reply?.recordedMods ?? [];
  const mods = [...(reply?.mods ?? []), ...selected.filter(id => !reply?.mods.some(mod => mod.id === id)).map(id => ({ id, name: id.replace(/^mod\//, ''), reason: 'No longer installed' }))]
    .filter(mod => mod.name.toLowerCase().includes(query.trim().toLowerCase()));
  const save = async () => {
    if (!available || !dirty) return;
    saveRequest.current?.abort();
    const saveController = new AbortController(); saveRequest.current = saveController;
    setBusy(true);
    try {
      if (mapDirty && draft && client.mapSettings) {
        const value = await client.mapSettings(source?.id ?? '', source?.revision ?? '', draft, saveController.signal, target);
        if (!saveController.signal.aborted) { setReply(value); setDraft(value.settings); setError(null); }
      }
      if (global && video.dirty && !saveController.signal.aborted) await video.save();
    } catch (cause) { if (!saveController.signal.aborted) setError(errorMessage(cause)); }
    finally { if (!saveController.signal.aborted) setBusy(false); }
  };
  return <div className="map-menu-section map-appearance">
    <div className="map-appearance-heading"><strong>{scopeLabel}</strong>{global && <small>Global defaults</small>}<span className="folder-info" tabIndex={0} role="img" aria-label="About graphics defaults" data-tooltip={global ? 'Defaults for all campaign maps. Campaigns and widgets can override them.' : viewId ? 'Changes apply only to this widget for this campaign.' : 'Changes apply to this campaign and widgets that inherit its settings.'}><Icon name="info" /></span></div>
    {!available && <p className="map-menu-note" role="status">Load a campaign to configure its map graphics and visual mods.</p>}
    {draft && <small className="pill graphics-inheritance">{draft.useGameGraphics ? `Using ${parentLabel}` : 'Custom graphics'}</small>}
    <div className="map-graphics-options">
    <div className="map-graphics-fields">
    <label className="map-option-row"><span>{ANTI_ALIASING}</span><select aria-label={ANTI_ALIASING} disabled={!draft || busy} value={draft?.graphics.samples ?? ""} onChange={event => { if (draft) setDraft({ ...draft, useGameGraphics: false, graphics: { ...draft.graphics, samples: Number(event.currentTarget.value) } }); }}>
      {!draft && <option value="">{!available ? "No campaign" : error ? "Unavailable" : "Loading..."}</option>}
      {[...new Set([...(reply?.sampleCounts ?? []), ...(draft ? [draft.graphics.samples] : [])])].sort((a, b) => a - b).map(count => <option key={count} value={count} disabled={!!reply && !reply.sampleCounts.includes(count)}>{count === 1 ? 'Off' : `${count}x MSAA`}{reply && !reply.sampleCounts.includes(count) ? ' (unavailable)' : ''}</option>)}
    </select></label>
    <label className="map-option-row"><span>{TEXTURE_QUALITY}</span><select aria-label={TEXTURE_QUALITY} disabled={!draft || busy} value={draft?.graphics.textureQuality ?? ""} onChange={event => { if (draft) setDraft({ ...draft, useGameGraphics: false, graphics: { ...draft.graphics, textureQuality: Number(event.currentTarget.value) } }); }}>
      {!draft && <option value="">{!available ? "No campaign" : error ? "Unavailable" : "Loading..."}</option>}<option value={0}>High</option><option value={1}>Medium</option><option value={2}>Low</option>
    </select></label>
    {global && <VideoControls video={video} busy={busy} />}
    </div>
    <div className="map-graphics-grid">{graphics.map(([key, label]) => {
      const value = key === 'natoCounters' ? draft?.graphics.natoCounters ?? reply?.inheritedSettings.graphics.natoCounters : draft?.graphics[key];
      return <label className="map-option-row" key={key}><span>{label}</span><input type="checkbox" role="switch" disabled={!draft || busy} checked={value ?? false} aria-checked={value ?? "mixed"}
        onChange={event => { if (draft) setDraft({ ...draft, useGameGraphics: false, graphics: { ...draft.graphics, [key]: event.currentTarget.checked } }); }} /></label>;
    })}</div>
    </div>
    <button type="button" className="map-menu-action graphics-reset" disabled={busy || !(draft && reply && (!global || reply.gameGraphics)) && !(global && video.canReset)} onClick={() => { if (draft && reply && (!global || reply.gameGraphics)) setDraft({ ...draft, useGameGraphics: true, graphics: reply.inheritedSettings.graphics }); if (global) video.reset(); }}>{global ? 'Revert to the game settings' : `Use ${parentLabel}`}</button>
    {global && reply?.gameGraphicsError && <p className="map-menu-note">Game graphics settings unavailable: {reply.gameGraphicsError}</p>}
    {global && video.gameError && <p className="map-menu-note">Display defaults unavailable: {video.gameError}</p>}
    {global && video.error && <p className="map-menu-error" role="alert">{video.error} <button type="button" className="button quiet" onClick={video.retry}>Retry</button></p>}
    <details className="map-visual-mods"><summary>Visual mods{reply && <span>{global && draft?.visualMods === null ? 'Recorded selection' : `${selected.length} enabled`}</span>}</summary>
      {!reply && <p className="map-menu-note" role="status">{!available ? 'Load a campaign to see installed visual mods.' : error ? 'Installed mods are unavailable until the settings connection is restored.' : 'Reading installed mods...'}</p>}
      {reply && reply.mods.length > 0 && <input className="map-mod-search" aria-label="Search installed visual mods" placeholder="Search installed mods..." value={query} onChange={event => setQuery(event.currentTarget.value)} />}
      <div className="map-mod-list">{mods.map(mod => <label key={mod.id} className="map-mod-row">
        <input type="checkbox" checked={selected.includes(mod.id)} disabled={busy || (!!mod.reason && !selected.includes(mod.id)) || !draft} onChange={event => {
          if (draft) setDraft({ ...draft, visualMods: event.currentTarget.checked ? [...selected, mod.id] : selected.filter(id => id !== mod.id) });
        }} /><span>{mod.name}{mod.reason && <small>{mod.reason}</small>}</span>
      </label>)}</div>
      {reply && mods.length === 0 && <p className="map-menu-note">{query ? 'No matching installed mods.' : 'No installed mods found.'}</p>}
      {selected.length > 1 && <div className="map-mod-order"><small>Applied in this order; later mods override earlier ones.</small>{selected.map((id, index) => <div key={id}><span>{reply?.mods.find(mod => mod.id === id)?.name ?? id}</span><button type="button" className="dash-icon-button" disabled={busy || index === 0} aria-label={`Move ${reply?.mods.find(mod => mod.id === id)?.name ?? id} earlier`} onClick={() => {
        if (!draft || index === 0) return; const next = [...selected]; [next[index - 1], next[index]] = [next[index], next[index - 1]]; setDraft({ ...draft, visualMods: next });
      }}>↑</button></div>)}</div>}
      {draft?.visualMods !== null && draft && <button type="button" className="map-menu-action" disabled={busy} onClick={() => setDraft({ ...draft, visualMods: null })}>{global ? 'Use recorded mod selection' : `Use ${parentLabel} for mods`}</button>}
    </details>
    {available && !reply && !error && <p className="map-menu-note" role="status">Reading map settings…</p>}
    {error && <div className="map-menu-error" role="alert">{error}{!reply && <button type="button" className="button quiet" onClick={() => setRetry(value => value + 1)}>Retry</button>}</div>}
    {dirty && <div className="map-appearance-actions"><button type="button" className="button quiet" disabled={busy} onClick={() => { setDraft(reply?.settings ?? null); if (global) video.cancel(); if (reply) setError(null); }}>Cancel</button><button type="button" className="button" disabled={busy} onClick={() => void save()}>{busy ? 'Applying…' : 'Apply'}</button></div>}
  </div>;
}
