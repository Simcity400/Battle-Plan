import { useEffect, useRef, useState } from 'react';
import { Select } from './Select';
import { desktopAvailable, errorMessage, mapAction } from '../shell/api';
import { Icon } from './Icons';
import CountryPicker from './CountryPicker';
import VideoControls, { useVideoSettings } from '../shell/VideoSettings';
import './mapControls.css';
import type { CampaignMapSettings, DashboardClient, DashboardSource, MapGraphics, MapSettingsReply } from '../dashboard/client';
import type { CounterFilter, MapAudio } from '../generated/dashboard';

const ANTI_ALIASING = 'Anti-aliasing';
const TEXTURE_QUALITY = 'Texture quality';
const COUNTER_FILTER = 'Counters shown for';
type Switch = Exclude<keyof MapGraphics, 'samples' | 'textureQuality'>;
type Item = { key: Switch; label: string; parts?: [Switch, string][] };
/** Quality choices that change how the map looks rather than what it shows. */
const QUALITY: [Switch, string][] = [
  ['fullResolution', 'Full map resolution'], ['highGfxShaders', 'High-quality shaders'], ['waterReflections', 'Water reflections'],
  ['shadows', 'Shadows'], ['dayNight', 'Day/night cycle'], ['natoCounters', 'NATO counters'],
];
/** Every show/hide switch: one main switch per item, its parts under Advanced. */
const GROUPS: { title: string; items: Item[] }[] = [
  { title: 'World', items: [
    { key: 'water', label: 'Water' },
    { key: 'borders', label: 'Borders', parts: [['landBorders', 'Land borders'], ['seaBorders', 'Sea borders'], ['demilitarizedZones', 'Demilitarised zones'], ['straitCrossings', 'Strait crossings']] },
    { key: 'rivers', label: 'Rivers' },
    { key: 'trees', label: 'Trees' },
    { key: 'railways', label: 'Railways', parts: [['rails', 'Rails'], ['railwayBridges', 'Railway bridges'], ['supplyTrains', 'Supply trains']] },
    { key: 'citySprawl', label: 'City sprawl' },
    { key: 'buildings', label: '3D buildings' },
    { key: 'mapTable', label: 'Map table' },
    { key: 'nuclearStrikes', label: 'Nuclear strikes' },
    { key: 'bombardmentExplosions', label: 'Bombardment explosions' },
    { key: 'missileExplosions', label: 'Missile explosions' },
    { key: 'mapText', label: 'Floating map text', parts: [['textFakeDivisions', 'Fake divisions revealed'], ['textRailwayGuns', 'Railway guns overrun'], ['textBombardments', 'Shore bombardments'],
      ['textGenerals', 'Generals captured'], ['textParadrops', 'Paradrop landings'], ['textResistance', 'Resistance activity'], ['textBuildings', 'Building damage']] },
    { key: 'weather', label: 'Weather effects' },
    { key: 'sky', label: 'Sky' },
    { key: 'countryNames', label: 'Country names' },
    { key: 'cityNames', label: 'City names' },
  ] },
  { title: 'Units', items: [
    { key: 'units', label: '3D units', parts: [['unitFigures', 'Unit figures'], ['aircraft', 'Aircraft']] },
    { key: 'counters', label: 'Counters and icons', parts: [['landCounters', 'Division counters'], ['railwayGunCounters', 'Railway-gun counters'], ['navalCounters', 'Naval counters'],
      ['portCounters', 'Ships in port'], ['baseIcons', 'Base icons'], ['theatrePins', 'Theatre pins'], ['landCombatMarkers', 'Land combat markers'], ['navalMarkers', 'Naval combat markers'], ['raidIcons', 'Raid icons']] },
    { key: 'movementArrows', label: 'Selected unit arrows' },
  ] },
  { title: 'Plans', items: [
    { key: 'battlePlans', label: 'Battle plans', parts: [['frontLines', 'Front lines'], ['offensiveLines', 'Offensive lines'], ['plannedMovement', 'Planned movement'],
      ['navalInvasions', 'Naval invasions'], ['paradrops', 'Paradrops'], ['raidPaths', 'Raid paths'], ['planLabels', 'Plan labels']] },
    { key: 'showAlliedPlans', label: 'Allied battle plans' },
  ] },
  { title: 'Interface', items: [{ key: 'tooltips', label: 'Tooltips' }] },
];
/** Aircraft by mission, under the 3D units and aircraft switches. */
const AIR_MISSIONS: [Switch, string][] = [['airPatrols', 'Patrol flights'], ['airReconnaissance', 'Reconnaissance flights'], ['airSupport', 'Close air support flights'],
  ['airBombing', 'Bombing flights'], ['airCombat', 'Air combat'], ['airKamikaze', 'Kamikaze strikes'], ['airSupply', 'Air supply flights'], ['airMissiles', 'Missiles']];
const AUDIO: [keyof MapAudio, string][] = [['master', 'Master volume'], ['ambient', 'Ambient volume'], ['units', 'Unit volume'], ['combat', 'Combat volume'], ['voice', 'Voice volume'], ['music', 'Music volume']];
/** Every option row's label, in order; settings search reveals rows by these. */
export const MAP_OPTION_LABELS = [ANTI_ALIASING, TEXTURE_QUALITY, ...QUALITY.map(([, label]) => label),
  ...GROUPS.flatMap(group => group.items.flatMap(item => [item.label, ...(item.parts ?? []).map(([, label]) => label)])), ...AIR_MISSIONS.map(([, label]) => label),
  COUNTER_FILTER, ...AUDIO.map(([, label]) => label)];

/** The stored settings with every choice inheriting, in the same shape, so reverting to what is
 * already stored is not a change. Visual mods are kept: they have their own control. */
export const inheritingAll = (settings: CampaignMapSettings): CampaignMapSettings => ({
  graphics: Object.fromEntries(Object.keys(settings.graphics).map(key => [key, null])) as CampaignMapSettings['graphics'],
  audio: Object.fromEntries(Object.keys(settings.audio).map(key => [key, null])) as CampaignMapSettings['audio'],
  counters: null, visualMods: settings.visualMods,
});
const isSet = (value: unknown) => value !== null && value !== undefined;
const overridden = (settings: CampaignMapSettings) => Object.values(settings.graphics).some(isSet) || Object.values(settings.audio).some(isSet) || settings.counters !== null;
const filterLabel = (filter: CounterFilter) => filter.kind === 'all' ? 'All countries' : filter.kind === 'players' ? 'Players only' : `${filter.tags.length} chosen`;

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
  // Reverting is offered only while something differs from what it reverts to.
  const graphicsCustom = !!draft && !!reply && overridden(draft);
  const videoCustom = global && video.canReset && !video.atGame;
  const inherited = reply?.inheritedSettings;
  const enabled = !!draft && !busy;
  const setGraphics = (key: keyof MapGraphics, value: unknown) => { if (draft) setDraft({ ...draft, graphics: { ...draft.graphics, [key]: value } }); };
  const setAudio = (key: keyof MapAudio, value: number | null) => { if (draft) setDraft({ ...draft, audio: { ...draft.audio, [key]: value } }); };
  const reset = (label: string, onClick: () => void) => <button type="button" className="icon-button small map-option-inherit" disabled={!enabled}
    aria-label={`${label}: use ${parentLabel}`} data-tooltip={`Use ${parentLabel}`} onClick={onClick}><Icon name="refresh" /></button>;
  const switchRow = (key: Switch, label: string, part = false) => {
    const own = draft?.graphics[key];
    const value = isSet(own) ? own as boolean : inherited?.graphics[key] as boolean | null | undefined;
    return <div className={`map-option-row${part ? ' map-option-part' : ''}`} key={key} data-overridden={isSet(own) || undefined}>
      <span>{label}</span>
      <span className="map-option-control">
        {isSet(own) && reset(label, () => setGraphics(key, null))}
        <input type="checkbox" role="switch" aria-label={label} disabled={!enabled} checked={value ?? false} aria-checked={value ?? 'mixed'}
          onChange={event => setGraphics(key, event.currentTarget.checked)} />
      </span>
    </div>;
  };
  const selectRow = (key: 'samples' | 'textureQuality', label: string, choices: { value: string; label: string; disabled?: boolean }[]) => {
    const own = draft?.graphics[key];
    const parent = inherited?.graphics[key];
    const describe = (value: number | undefined) => choices.find(choice => choice.value === String(value))?.label ?? '';
    return <div className="map-option-row" data-overridden={isSet(own) || undefined}><span>{label}</span>
      <Select label={label} disabled={!enabled} value={isSet(own) ? String(own) : ''} onChange={value => setGraphics(key, value === '' ? null : Number(value))}
        options={[{ value: '', label: !draft ? (!available ? 'No campaign' : error ? 'Unavailable' : 'Loading...') : `Inherit (${describe(parent)})` }, ...choices]} /></div>;
  };
  const samples = [...new Set([...(reply?.sampleCounts ?? []), ...(isSet(draft?.graphics.samples) ? [draft!.graphics.samples!] : [])])].sort((a, b) => a - b)
    .map(count => ({ value: String(count), label: `${count === 1 ? 'Off' : `${count}x MSAA`}${reply && !reply.sampleCounts.includes(count) ? ' (unavailable)' : ''}`, disabled: !!reply && !reply.sampleCounts.includes(count) }));
  const filter = draft?.counters ?? null;
  const shownFilter = filter ?? inherited?.counters ?? { kind: 'all' as const };
  const chosen = shownFilter.kind === 'countries' ? shownFilter.tags : [];
  // Any installed mod can be shown; one no longer installed keeps its place by name.
  const selected = draft?.visualMods ?? reply?.inheritedSettings.visualMods ?? [];
  const mods = [...(reply?.mods ?? []), ...selected.filter(choice => !reply?.mods.some(mod => mod.id === choice.id)).map(choice => ({ ...choice, problem: 'No longer installed' }))]
    .filter(mod => mod.name.toLowerCase().includes(query.trim().toLowerCase()));
  const recorded = new Set(reply?.recordedMods.map(mod => mod.recordedId));
  const refreshMods = async () => {
    if (!client.mapSettings) return;
    setBusy(true);
    try {
      const value = await client.mapSettings(source?.id ?? '', source?.revision ?? '', null, new AbortController().signal, target); setReply(value); if (!mapDirty) setDraft(value.settings);
      // Open maps whose visual mods were installed, updated or removed reload where they are.
      if (desktopAvailable) await mapAction({ kind: 'refreshVisualMods' });
    }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  };
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
    <div className="map-appearance-heading"><strong>{scopeLabel}</strong>{global && <small>Global defaults</small>}<span className="folder-info" tabIndex={0} role="img" aria-label="About graphics defaults" data-tooltip={global ? 'Defaults for all campaign maps. Each setting follows the game until you change it; campaigns and widgets can override each one.' : viewId ? 'Changes apply only to this widget for this campaign. Settings you leave follow the campaign.' : 'Changes apply to this campaign and widgets that follow it. Settings you leave follow the global defaults.'}><Icon name="info" /></span></div>
    {!available && <p className="map-menu-note" role="status">Load a campaign to configure its map graphics and visual mods.</p>}
    {draft && <small className="pill graphics-inheritance">{graphicsCustom ? 'Some settings changed here' : `Using ${parentLabel}`}</small>}
    <div className="map-graphics-options">
      <div className="map-graphics-fields">
        {selectRow('samples', ANTI_ALIASING, samples)}
        {selectRow('textureQuality', TEXTURE_QUALITY, [{ value: '0', label: 'High' }, { value: '1', label: 'Medium' }, { value: '2', label: 'Low' }])}
        {global && <VideoControls video={video} busy={busy} />}
      </div>
      <div className="map-graphics-grid">{QUALITY.map(([key, label]) => switchRow(key, label))}</div>
      {GROUPS.map(group => <div className="map-graphics-group" key={group.title}>
        <small className="map-graphics-group-title">{group.title}</small>
        <div className="map-graphics-grid">{group.items.map(item => switchRow(item.key, item.label))}</div>
      </div>)}
      <div className="map-option-row" data-overridden={filter !== null || undefined}><span>{COUNTER_FILTER}</span>
        <span className="map-option-control">
          <Select label={COUNTER_FILTER} disabled={!enabled} value={filter?.kind ?? ''}
            onChange={value => { if (draft) setDraft({ ...draft, counters: value === '' ? null : value === 'countries' ? { kind: 'countries', tags: chosen } : { kind: value as 'all' | 'players' } }); }}
            options={[{ value: '', label: inherited ? `Inherit (${filterLabel(inherited.counters)})` : 'Inherit' }, { value: 'all', label: 'All countries' }, { value: 'players', label: 'Players only' },
              ...(source ? [{ value: 'countries', label: 'Chosen countries' }] : [])]} />
        </span>
      </div>
      {source && shownFilter.kind === 'countries' && <div className="map-counter-countries">
        <CountryPicker source={source} client={client} countries={source.metadata.countries} playerCountryId={source.metadata.playerCountryId ?? null} selectedTag={null} field chosen={chosen}
          addLabel={`${chosen.length} ${chosen.length === 1 ? 'country' : 'countries'}: add or remove`} onChange={tag => {
            if (!tag || !draft) return;
            setDraft({ ...draft, counters: { kind: 'countries', tags: chosen.includes(tag) ? chosen.filter(item => item !== tag) : [...chosen, tag] } });
          }} />
      </div>}
      <details className="map-visual-mods map-advanced"><summary>Advanced</summary>
        <p className="map-menu-note">Each part shows only while its main switch is on.</p>
        {GROUPS.flatMap(group => group.items).filter(item => item.parts).map(item => <div className="map-graphics-group" key={item.key}>
          <small className="map-graphics-group-title">{item.label}</small>
          <div className="map-graphics-grid">{item.parts!.map(([key, label]) => switchRow(key, label, true))}</div>
        </div>)}
        <div className="map-graphics-group">
          <small className="map-graphics-group-title">Aircraft missions</small>
          <div className="map-graphics-grid">{AIR_MISSIONS.map(([key, label]) => switchRow(key, label, true))}</div>
        </div>
      </details>
      <div className="map-graphics-group">
        <small className="map-graphics-group-title">Sound <span className="folder-info" tabIndex={0} role="img" aria-label="About map sound" data-tooltip="Only the map you are using plays sound and music, at these volumes."><Icon name="info" /></span></small>
        {AUDIO.map(([key, label]) => {
          const own = draft?.audio[key];
          const value = isSet(own) ? own as number : inherited?.audio[key];
          return <div className="map-option-row" key={key} data-overridden={isSet(own) || undefined}><span>{label}</span>
            <span className="map-option-control">
              {isSet(own) && reset(label, () => setAudio(key, null))}
              <input type="range" min={0} max={100} step={1} aria-label={label} disabled={!enabled} value={value ?? 0} onChange={event => setAudio(key, Number(event.currentTarget.value))} />
              <output className="map-volume">{isSet(value) ? Math.round(value!) : ''}</output>
            </span>
          </div>;
        })}
      </div>
    </div>
    {(graphicsCustom || videoCustom) && <button type="button" className="map-menu-action graphics-reset" disabled={busy}
      onClick={() => { if (graphicsCustom) setDraft(inheritingAll(draft!)); if (videoCustom) video.reset(); }}>
      {global ? 'Revert to the game settings' : `Use ${parentLabel}`}</button>}
    {global && reply?.gameSettingsError && <p className="map-menu-note">Game settings unavailable: {reply.gameSettingsError}</p>}
    {global && video.gameError && <p className="map-menu-note">Display defaults unavailable: {video.gameError}</p>}
    {global && video.error && <p className="map-menu-error" role="alert">{video.error} <button type="button" className="button quiet" onClick={video.retry}>Retry</button></p>}
    <details className="map-visual-mods"><summary>Visual mods{reply && <span>{selected.length ? `${selected.length} shown` : 'None'}</span>}</summary>
      <p className="map-menu-note">Show any installed mod's artwork over {global ? 'every campaign\'s maps and widgets' : viewId ? 'this map only' : 'this recording\'s maps and widgets'}. Widgets show its icons, flags, portraits, windows, fonts and their text; map geometry, everything recorded and analytics labels stay as recorded. Some combinations may look wrong.</p>
      {reply && reply.recordedMods.length > 0 && <p className="map-menu-note">Recorded with {reply.recordedMods.map(mod => mod.name).join(', ')}.</p>}
      {!reply && <p className="map-menu-note" role="status">{!available ? 'Load a campaign to see installed visual mods.' : error ? 'Installed mods are unavailable until the settings connection is restored.' : 'Reading installed mods...'}</p>}
      {reply && <div className="map-mod-tools">{reply.mods.length > 0 && <input className="map-mod-search" aria-label="Search installed visual mods" placeholder="Search installed mods..." value={query} onChange={event => setQuery(event.currentTarget.value)} />}
        <button type="button" className="button quiet" disabled={busy} onClick={() => void refreshMods()}>Refresh</button></div>}
      <div className="map-mod-list">{mods.map(mod => <label key={mod.id} className="map-mod-row">
        <input type="checkbox" checked={selected.some(choice => choice.id === mod.id)} disabled={busy || !draft} onChange={event => {
          if (draft) setDraft({ ...draft, visualMods: event.currentTarget.checked ? [...selected, { id: mod.id, name: mod.name }] : selected.filter(choice => choice.id !== mod.id) });
        }} /><span>{mod.name}{'recordedId' in mod && recorded.has(mod.recordedId) && <small>In the recording</small>}{mod.problem && <small>{mod.problem}</small>}</span>
      </label>)}</div>
      {reply && mods.length === 0 && <p className="map-menu-note">{query ? 'No matching installed mods.' : 'No installed mods found.'}</p>}
      {reply && !mapDirty && reply.appliedMods.length > 1 && <div className="map-mod-order"><small>Shown in the game's load order; later mods cover earlier ones.</small>{reply.appliedMods.map(mod => <div key={mod.id}><span>{mod.name}</span></div>)}</div>}
      {reply && !mapDirty && reply.appliedNotes.map(note => <p key={note} className="map-menu-note">{note}</p>)}
      {draft?.visualMods !== null && draft && <button type="button" className="map-menu-action" disabled={busy} onClick={() => setDraft({ ...draft, visualMods: null })}>{global ? 'Show no visual mods' : `Use ${parentLabel} for mods`}</button>}
    </details>
    {available && !reply && !error && <p className="map-menu-note" role="status">Reading map settings…</p>}
    {error && <div className="map-menu-error" role="alert">{error}{!reply && <button type="button" className="button quiet" onClick={() => setRetry(value => value + 1)}>Retry</button>}</div>}
    {dirty && <div className="map-appearance-actions"><button type="button" className="button quiet" disabled={busy} onClick={() => { setDraft(reply?.settings ?? null); if (global) video.cancel(); if (reply) setError(null); }}>Cancel</button><button type="button" className="button" disabled={busy} onClick={() => void save()}>{busy ? 'Applying…' : 'Apply'}</button></div>}
  </div>;
}
