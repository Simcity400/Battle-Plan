import { Fragment, useEffect, useId, useRef, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { Select } from './Select';
import { desktopAvailable, errorMessage, mapAction } from '../shell/api';
import { Icon } from './Icons';
import CountryPicker from './CountryPicker';
import { ConfirmStep } from './ContextMenu';
import { FailureLine } from './FailureLine';
import VideoControls, { useVideoSettings } from '../shell/VideoSettings';
import { ConfirmRow, SettingsBlock } from '../shell/SettingsBlock';
import { counterCountries, currentCampaignSettings, saveCampaignSettings, setCounterCountries } from '../dashboard/campaignSettings';
import './mapControls.css';
import type { CampaignMapSettings, DashboardClient, DashboardSource, MapGraphics, MapSettingsReply } from '../dashboard/client';
import type { CounterFilter, MapAudio } from '../generated/dashboard';
import { writeSlot } from '../shell/pendingWrites';

const ANTI_ALIASING = 'Anti-aliasing';
const TEXTURE_QUALITY = 'Texture quality';
const COUNTER_FILTER = 'Counters shown for';
type Switch = Exclude<keyof MapGraphics, 'samples' | 'textureQuality'>;
const MAP_SAMPLE_COUNTS = [1, 2, 4, 8];
type Item = { key: Switch; label: string; parts?: [Switch, string][] };
const QUALITY: [Switch, string][] = [
  ['fullResolution', 'Full map resolution'], ['highGfxShaders', 'High-quality shaders'], ['waterReflections', 'Water reflections'],
  ['shadows', 'Shadows'], ['dayNight', 'Day/night cycle'],
];
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
    { key: 'weather', label: 'Weather effects' },
    { key: 'sky', label: 'Sky' },
  ] },
  { title: 'Labels and tooltips', items: [
    { key: 'countryNames', label: 'Country names' },
    { key: 'cityNames', label: 'City names' },
    { key: 'mapText', label: 'Floating map text', parts: [['textFakeDivisions', 'Fake divisions revealed'], ['textRailwayGuns', 'Railway guns overrun'], ['textBombardments', 'Shore bombardments'],
      ['textGenerals', 'Generals captured'], ['textParadrops', 'Paradrop landings'], ['textResistance', 'Resistance activity'], ['textBuildings', 'Building damage']] },
    { key: 'tooltips', label: 'Tooltips' },
  ] },
  { title: 'Units', items: [
    { key: 'units', label: '3D units', parts: [['unitFigures', 'Unit figures'], ['aircraft', 'Aircraft']] },
    { key: 'counters', label: 'Counters and icons', parts: [['landCounters', 'Division counters'], ['railwayGunCounters', 'Railway-gun counters'], ['navalCounters', 'Naval counters'],
      ['portCounters', 'Ships in port'], ['baseIcons', 'Base icons'], ['theatrePins', 'Theatre pins'], ['landCombatMarkers', 'Land combat markers'], ['navalMarkers', 'Naval combat markers'], ['raidIcons', 'Raid icons']] },
    { key: 'natoCounters', label: 'NATO counters' },
    { key: 'movementArrows', label: 'Selected unit arrows' },
  ] },
  { title: 'Plans', items: [
    { key: 'battlePlans', label: 'Battle plans', parts: [['frontLines', 'Front lines'], ['offensiveLines', 'Offensive lines'], ['plannedMovement', 'Planned movement'],
      ['navalInvasions', 'Naval invasions'], ['paradrops', 'Paradrops'], ['raidPaths', 'Raid paths'], ['planLabels', 'Plan labels']] },
    { key: 'showAlliedPlans', label: 'Allied battle plans' },
  ] },
];
const AIR_MISSIONS: [Switch, string][] = [['airPatrols', 'Patrol flights'], ['airReconnaissance', 'Reconnaissance flights'], ['airSupport', 'Close air support flights'],
  ['airBombing', 'Bombing flights'], ['airCombat', 'Air combat'], ['airKamikaze', 'Kamikaze strikes'], ['airSupply', 'Air supply flights'], ['airMissiles', 'Missiles']];
const AUDIO: [keyof MapAudio, string][] = [['master', 'Master volume'], ['ambient', 'Ambient volume'], ['units', 'Unit volume'], ['combat', 'Combat volume'], ['interface', 'Interface volume'], ['voice', 'Voice volume'], ['music', 'Music volume']];
export const MAP_SOUND_LABELS = AUDIO.map(([, label]) => label);
type Place = { section: string; label: string };
const PLACES = new Map<string, Place>([
  ['graphics.samples', { section: 'display', label: ANTI_ALIASING }], ['graphics.textureQuality', { section: 'display', label: TEXTURE_QUALITY }],
  ...QUALITY.map(([key, label]): [string, Place] => [`graphics.${key}`, { section: 'quality', label }]),
  ...GROUPS.flatMap(group => group.items.flatMap((item): [string, Place][] => [[`graphics.${item.key}`, { section: group.title, label: item.label }],
    ...(item.parts ?? []).map(([key, label]): [string, Place] => [`graphics.${key}`, { section: 'advanced', label }])])),
  ...AIR_MISSIONS.map(([key, label]): [string, Place] => [`graphics.${key}`, { section: 'advanced', label }]),
  ['counters', { section: 'counters', label: COUNTER_FILTER }],
  ...AUDIO.map(([key, label]): [string, Place] => [`audio.${key}`, { section: 'sound', label }]),
  ['visualMods', { section: 'mods', label: 'Visual mods' }],
]);
type MapFailure = { text: string; detail?: string; retry?: () => void; section?: string; write?: boolean };
export const MAP_OPTION_LABELS = [ANTI_ALIASING, TEXTURE_QUALITY, ...QUALITY.map(([, label]) => label),
  ...GROUPS.flatMap(group => group.items.flatMap(item => [item.label, ...(item.parts ?? []).map(([, label]) => label)])), ...AIR_MISSIONS.map(([, label]) => label),
  COUNTER_FILTER, ...MAP_SOUND_LABELS];
const SOUND_NOTE = 'Only the map you’re using plays sound and music, at these volumes.';
const MODS_HELP = 'A mod can supply icons, flags, portraits, windows, fonts and interface text. Analytics labels and everything recorded stay as recorded. Some combinations of mods may look wrong.';

export const inheritingAll = (settings: CampaignMapSettings): CampaignMapSettings => ({
  graphics: Object.fromEntries(Object.keys(settings.graphics).map(key => [key, null])) as CampaignMapSettings['graphics'],
  audio: Object.fromEntries(Object.keys(settings.audio).map(key => [key, null])) as CampaignMapSettings['audio'],
  counters: null, visualMods: settings.visualMods,
});
const isSet = (value: unknown) => value !== null && value !== undefined;
const overridden = (settings: CampaignMapSettings) => Object.values(settings.graphics).some(isSet) || Object.values(settings.audio).some(isSet) || settings.counters !== null;
const applied = (setting: string, value: unknown) => ({ 'data-map-setting': setting, 'data-map-value': JSON.stringify(value ?? null) });
const sentence = (label: string) => /^[A-Z][a-z]/.test(label) ? label[0].toLowerCase() + label.slice(1) : label;
const named = (names: string[]) => names.length <= 3 ? names.length > 1 ? `${names.slice(0, -1).join(', ')} and ${names.at(-1)}` : names[0] ?? ''
  : `${names.slice(0, 2).join(', ')} and ${names.length - 2} others`;
const differs = (first: unknown, second: unknown) => JSON.stringify(first ?? null) !== JSON.stringify(second ?? null);
const changedKeys = (prefix: 'graphics' | 'audio', saved: Record<string, unknown>, next: Record<string, unknown>) =>
  [...new Set([...Object.keys(saved), ...Object.keys(next)])].filter(key => differs(saved[key], next[key])).map(key => `${prefix}.${key}`);
const unsavedSettings = (saved: CampaignMapSettings, next: CampaignMapSettings) => [
  ...changedKeys('graphics', saved.graphics, next.graphics), ...changedKeys('audio', saved.audio, next.audio),
  ...(differs(saved.counters, next.counters) ? ['counters'] : []), ...(differs(saved.visualMods, next.visualMods) ? ['visualMods'] : []),
];
const filterLabel = (filter: CounterFilter) => filter.kind === 'all' ? 'All countries' : filter.kind === 'players' ? 'Players only' : `${filter.tags.length} chosen`;

function Volume({ label, value, disabled, onCommit }: { label: string; value: number | null | undefined; disabled: boolean; onCommit: (value: number) => void }) {
  const [moving, setMoving] = useState<number | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const commit = useRef(onCommit);
  commit.current = onCommit;
  useEffect(() => {
    const element = input.current;
    if (!element) return;
    const done = () => { setMoving(null); commit.current(Number(element.value)); };
    element.addEventListener('change', done);
    return () => element.removeEventListener('change', done);
  }, []);
  const known = moving !== null || isSet(value);
  const shown = moving ?? value ?? 0;
  return <span className={`map-volume-control${known ? '' : ' is-unknown'}`}>
    <output className="map-volume" aria-label={known ? undefined : `${label} not known yet`}>{known ? Math.round(shown) : '—'}</output>
    <input ref={input} type="range" className="slider" min={0} max={100} step={1} aria-label={label} aria-hidden={known ? undefined : true} disabled={disabled || !known} value={shown}
      style={{ '--fill': `${shown}%` } as CSSProperties} onChange={event => setMoving(Number(event.currentTarget.value))} />
  </span>;
}

export default function MapAppearance({ source, client, global = false, viewId, userFolder }: { source: DashboardSource | null; client: DashboardClient; global?: boolean; viewId?: string; userFolder?: string }) {
  const video = useVideoSettings(global, userFolder);
  const target = global ? { scope: 'global' as const } : viewId ? { scope: 'widget' as const, id: viewId } : { scope: 'campaign' as const };
  const available = global || !!source;
  const parentLabel = global ? 'the game settings' : viewId ? 'campaign settings' : 'global defaults';
  const scopeLabel = viewId ? 'Widget graphics' : 'Campaign graphics';
  const rowId = useId();
  const [reply, setReply] = useState<MapSettingsReply | null>(null);
  const [draft, setDraft] = useState<CampaignMapSettings | null>(null);
  const [error, setError] = useState<MapFailure | null>(null);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [lastSection, setLastSection] = useState<string | undefined>(undefined);
  const [retry, setRetry] = useState(0);
  const [query, setQuery] = useState('');
  const [reverting, setReverting] = useState(false);
  const revertTrigger = useRef<HTMLButtonElement>(null);
  const [clearing, setClearing] = useState(false);
  const clearTrigger = useRef<HTMLButtonElement>(null);
  const modTools = useRef<HTMLDivElement>(null);
  const writes = useRef({ next: null as CampaignMapSettings | null, running: false, live: true, slot: writeSlot() });
  useEffect(() => {
    const controller = new AbortController();
    setError(null); setReply(null); setDraft(null);
    if (!available) return;
    if (!client.mapSettings) { setError({ text: 'Map settings are unavailable in this version.' }); return; }
    void client.mapSettings(source?.id ?? '', source?.revision ?? '', null, controller.signal, target).then(value => {
      if (!controller.signal.aborted) { setReply(value); setDraft(value.settings); }
    }).catch(cause => { if (!controller.signal.aborted) setError({ text: 'Couldn’t read map settings.', detail: errorMessage(cause), retry: () => setRetry(value => value + 1) }); });
    return () => controller.abort();
  }, [client, source?.id, source?.revision, global, viewId, retry]);
  useEffect(() => { const queue = writes.current; queue.live = true; return () => { queue.live = false; if (!queue.running) queue.slot.discard(); }; }, []);
  const mapDirty = !!reply && !!draft && JSON.stringify(reply.settings) !== JSON.stringify(draft);
  const graphicsCustom = !!draft && !!reply && overridden(draft);
  const videoCustom = global && video.canReset && !video.atGame;
  const inherited = reply?.inheritedSettings;
  const enabled = !!draft && !busy;
  const unsaved = error?.write && reply && draft ? unsavedSettings(reply.settings, draft) : [];
  const unsavedPlaces = unsaved.flatMap(key => PLACES.get(key) ?? []);
  const unsavedSections = [...new Set(unsavedPlaces.map(place => place.section))];
  const failureSection = !error?.write ? error?.section : unsavedSections.length === 1 ? unsavedSections[0]
    : lastSection && unsavedSections.includes(lastSection) ? lastSection : undefined;
  const unsavedLabels = unsavedPlaces.map(place => sentence(place.label));
  const failureText = error?.write && unsavedLabels.length > 0 && unsavedLabels.length <= 3
    ? `Couldn’t save your ${unsavedLabels.length === 1 ? 'change' : 'changes'} to ${named(unsavedLabels)}.` : error?.text;
  const unsavedMark = (setting: string) => unsaved.includes(setting) && <span className="map-option-unsaved"><Icon name="alert" />Not saved</span>;
  const failureLine = error && <FailureLine className="map-menu-error" message={failureText ?? error.text} detail={error.detail} onRetry={error.retry} retryDisabled={busy || saving} />;
  const failure = !failureSection && failureLine;
  const failureIn = (...sections: string[]) => failureSection && sections.includes(failureSection) ? global ? <div className="settings-group-body map-section-failure">{failureLine}</div> : failureLine : null;
  const write = async () => {
    const queue = writes.current;
    if (queue.running || !client.mapSettings) return;
    queue.running = true;
    queue.slot.enter();
    setSaving(true);
    let failed: CampaignMapSettings | null = null;
    while (queue.next) {
      const settings = queue.next; queue.next = null;
      try {
        const value = await client.mapSettings(source?.id ?? '', source?.revision ?? '', settings, new AbortController().signal, target);
        failed = null;
        if (queue.live) { setReply(value); setError(null); if (!queue.next) setDraft(value.settings); }
      } catch (cause) {
        failed = settings;
        if (queue.live) setError({ text: 'Couldn’t save that change.', detail: errorMessage(cause), write: true, retry: () => { queue.next ??= settings; void write(); } });
      }
    }
    queue.running = false;
    const again = failed;
    if (!again) queue.slot.saved(); else if (queue.live) queue.slot.failed(() => { queue.next ??= again; void write(); }); else queue.slot.discard();
    if (queue.live) setSaving(false);
  };
  const change = (next: CampaignMapSettings) => {
    const sections = new Set(unsavedSettings(draft ?? next, next).map(key => PLACES.get(key)?.section));
    setLastSection(sections.size === 1 ? [...sections][0] : undefined);
    setDraft(next);
    writes.current.slot.enter(); writes.current.next = next; void write();
  };
  const setGraphics = (key: keyof MapGraphics, value: unknown) => { if (draft) change({ ...draft, graphics: { ...draft.graphics, [key]: value } }); };
  const setAudio = (key: keyof MapAudio, value: number | null) => { if (draft) change({ ...draft, audio: { ...draft.audio, [key]: value } }); };
  const pending = !available ? 'No campaign' : error ? 'Unavailable' : 'Loading…';
  const unknown = (label: string) => <span className="map-option-unknown" role="img" aria-label={`${label}: ${draft ? 'not known yet' : pending}`}>—</span>;
  const reset = (label: string, onClick: () => void) => {
    const text = `Reset ${sentence(label)} to ${parentLabel}`;
    return <button type="button" className="icon-button small map-option-inherit" disabled={!enabled} data-tooltip-side="left" aria-label={text} data-tooltip={text} onClick={onClick}><Icon name="refresh" /></button>;
  };
  const switchRow = (key: Switch, label: string, part = false) => {
    const own = draft?.graphics[key];
    const value = isSet(own) ? own as boolean : inherited?.graphics[key] as boolean | null | undefined;
    return <label className={`map-option-row${part ? ' map-option-part' : ''}`} key={key} htmlFor={`${rowId}-${key}`} data-overridden={isSet(own) || undefined} {...applied(`graphics.${key}`, value)}>
      <span>{label}{unsavedMark(`graphics.${key}`)}</span>
      <span className="map-option-control">
        {isSet(own) && reset(label, () => setGraphics(key, null))}
        {isSet(value) ? <input id={`${rowId}-${key}`} type="checkbox" role="switch" aria-label={label} disabled={!enabled} checked={value!} aria-checked={value!}
          onChange={event => setGraphics(key, event.currentTarget.checked)} /> : unknown(label)}
      </span>
    </label>;
  };
  const selectRow = (key: 'samples' | 'textureQuality', label: string, choices: { value: string; label: string; disabled?: boolean }[]) => {
    const own = draft?.graphics[key];
    const parent = choices.find(choice => choice.value === String(inherited?.graphics[key]))?.label;
    return <div className="map-option-row" key={key} data-overridden={isSet(own) || undefined} {...applied(`graphics.${key}`, isSet(own) ? own : inherited?.graphics[key])}><span>{label}{unsavedMark(`graphics.${key}`)}</span>
      <span className="map-option-control">
        <Select label={label} disabled={!enabled} value={isSet(own) ? String(own) : ''} onChange={value => setGraphics(key, value === '' ? null : Number(value))}
          options={[{ value: '', label: !draft ? pending : parent ? `Default (${parent})` : 'Default' }, ...choices]} />
      </span></div>;
  };
  const supported = reply?.sampleCounts ?? null;
  const samples = [...new Set([...(reply ? supported ?? MAP_SAMPLE_COUNTS : []), ...(isSet(draft?.graphics.samples) ? [draft!.graphics.samples!] : [])])].sort((a, b) => a - b)
    .map(count => {
      const unavailable = !!supported && !supported.includes(count);
      return { value: String(count), label: `${count === 1 ? 'Off' : `${count}× MSAA`}${unavailable ? ' (unavailable)' : ''}`, disabled: unavailable };
    });
  const filter = draft?.counters ?? null;
  const shownFilter = filter ?? inherited?.counters ?? { kind: 'all' as const };
  const chosen = shownFilter.kind === 'countries' ? shownFilter.tags : [];
  const campaignKey = source?.metadata.campaignKey ?? null;
  const counterScope = viewId ? `widget:${viewId}` : 'campaign';
  const [counterFailure, setCounterFailure] = useState<{ text: string; retry?: () => void } | null>(null);
  const remember = (tags: string[]) => campaignKey ? saveCampaignSettings(setCounterCountries(currentCampaignSettings(), campaignKey, counterScope, tags)) : null;
  const setCounters = (value: string) => {
    if (!draft) return;
    setCounterFailure(null);
    if (filter?.kind === 'countries' && value !== 'countries' && remember(filter.tags)) {
      setCounterFailure({ text: `Couldn’t keep your ${filter.tags.length === 1 ? 'chosen country' : `${filter.tags.length} chosen countries`} on this device, so they’re still shown.`, retry: () => setCounters(value) });
      return;
    }
    const tags = filter?.kind === 'countries' ? filter.tags : (campaignKey ? counterCountries(currentCampaignSettings(), campaignKey, counterScope) : undefined) ?? chosen;
    change({ ...draft, counters: value === '' ? null : value === 'countries' ? { kind: 'countries', tags } : { kind: value as 'all' | 'players' } });
  };
  const selected = draft?.visualMods ?? reply?.inheritedSettings.visualMods ?? [];
  const savedMods = reply?.settings.visualMods ?? reply?.inheritedSettings.visualMods ?? [];
  const mods = [...(reply?.mods ?? []), ...selected.filter(choice => !reply?.mods.some(mod => mod.id === choice.id)).map(choice => ({ ...choice, problem: 'No longer installed' }))]
    .filter(mod => mod.name.toLowerCase().includes(query.trim().toLowerCase()));
  const modCount = selected.length ? `${selected.length} enabled` : 'None enabled';
  const recorded = new Set(reply?.recordedMods.map(mod => mod.recordedId));
  const refreshMods = async () => {
    if (!client.mapSettings) return;
    setBusy(true); setError(null);
    try {
      const value = await client.mapSettings(source?.id ?? '', source?.revision ?? '', null, new AbortController().signal, target); setReply(value); if (!mapDirty) setDraft(value.settings);
      if (desktopAvailable) await mapAction({ kind: 'refreshVisualMods' });
    }
    catch (cause) { setError({ text: 'Couldn’t refresh installed mods.', detail: errorMessage(cause), section: 'mods', retry: () => void refreshMods() }); }
    finally { setBusy(false); }
  };

  const display = [selectRow('samples', ANTI_ALIASING, samples),
    selectRow('textureQuality', TEXTURE_QUALITY, [{ value: '0', label: 'High' }, { value: '1', label: 'Medium' }, { value: '2', label: 'Low' }])];
  const counterRow = <div className="map-option-row" key="counters" data-overridden={filter !== null || undefined} {...applied('counters', inherited || filter ? shownFilter : null)}><span>{COUNTER_FILTER}{unsavedMark('counters')}</span>
    <span className="map-option-control">
      <Select label={COUNTER_FILTER} disabled={!enabled} value={filter?.kind ?? ''}
        onChange={setCounters}
        options={[{ value: '', label: !draft ? pending : inherited ? `Default (${filterLabel(inherited.counters)})` : 'Default' }, { value: 'all', label: 'All countries' }, { value: 'players', label: 'Players only' },
          ...(source ? [{ value: 'countries', label: 'Chosen countries' }] : [])]} />
    </span>
  </div>;
  const counterNote = counterFailure && <FailureLine className="map-menu-error" message={counterFailure.text} onRetry={counterFailure.retry} retryDisabled={!enabled} />;
  const countries = source && shownFilter.kind === 'countries' && <div className="map-counter-countries">
    <CountryPicker source={source} client={client} countries={source.metadata.countries} playerCountryId={source.metadata.playerCountryId ?? null} selectedTag={null} field chosen={chosen}
      disabled={!enabled} addLabel={`${chosen.length} ${chosen.length === 1 ? 'country' : 'countries'}: add or remove`} onChange={tag => {
        if (!tag || !draft || !enabled) return;
        const tags = chosen.includes(tag) ? chosen.filter(item => item !== tag) : [...chosen, tag];
        const keep = () => setCounterFailure(remember(tags) ? { text: 'Couldn’t save your chosen countries on this device. They apply until the app closes.', retry: keep } : null);
        keep();
        change({ ...draft, counters: { kind: 'countries', tags } });
      }} />
  </div>;
  const parts = [...GROUPS.flatMap(group => group.items).filter(item => item.parts).map(item => ({ title: item.label, rows: item.parts!.map(([key, label]) => switchRow(key, label, true)) })),
    { title: 'Aircraft missions', rows: AIR_MISSIONS.map(([key, label]) => switchRow(key, label, true)) }];
  const volumes = AUDIO.map(([key, label]) => {
    const own = draft?.audio[key];
    return <div className="map-option-row" key={key} data-overridden={isSet(own) || undefined} {...applied(`audio.${key}`, isSet(own) ? own : inherited?.audio[key])}><span>{label}{unsavedMark(`audio.${key}`)}</span>
      <span className="map-option-control">
        {isSet(own) && reset(label, () => setAudio(key, null))}
        <Volume label={label} value={isSet(own) ? own : inherited?.audio[key]} disabled={!enabled} onCommit={value => setAudio(key, value)} />
      </span>
    </div>;
  });
  const cancelClear = () => { setClearing(false); requestAnimationFrame(() => clearTrigger.current?.focus()); };
  const clearMods = () => {
    setClearing(false);
    if (draft) change({ ...draft, visualMods: null });
    requestAnimationFrame(() => modTools.current?.querySelector<HTMLElement>('input, button')?.focus());
  };
  const modsBody = <>
    <p className="map-menu-note">Use installed mods for artwork, fonts and interface text on {global ? 'every campaign’s maps and widgets' : viewId ? 'this map only' : 'this recording’s maps and widgets'}. The recording supplies the map geometry and campaign data.{' '}
      <span className="folder-info" tabIndex={0} role="img" aria-label="About visual mods" data-tooltip={MODS_HELP}><Icon name="info" /></span></p>
    {reply && reply.recordedMods.length > 0 && <p className="map-menu-note">Recorded with {reply.recordedMods.map(mod => mod.name).join(', ')}.</p>}
    {!reply && <p className="map-menu-note" role="status">{!available ? 'Load a campaign to see installed visual mods.' : error ? 'Couldn’t read map settings.' : 'Reading installed mods…'}</p>}
    {reply && <div ref={modTools} className="map-mod-tools">{reply.mods.length > 0 && <label className="search-field"><Icon name="search" /><input className="map-mod-search" type="search" aria-label="Search installed visual mods" placeholder="Search installed mods" value={query} onChange={event => setQuery(event.currentTarget.value)} /></label>}
      <button type="button" className="button small" disabled={busy} onClick={() => void refreshMods()}><Icon name="refresh" />Refresh</button></div>}
    <div className="map-mod-list" data-overridden={draft?.visualMods != null || undefined} {...applied('visualMods', selected.map(choice => choice.id).sort())}>{mods.map(mod => <label key={mod.id} className="map-mod-row">
      <span>{mod.name}{'recordedId' in mod && recorded.has(mod.recordedId) && <small>In the recording</small>}{mod.problem && <small>{mod.problem}</small>}
        {unsaved.includes('visualMods') && savedMods.some(choice => choice.id === mod.id) !== selected.some(choice => choice.id === mod.id)
          && <small className="map-option-unsaved"><Icon name="alert" />Not saved</small>}</span>
      <input type="checkbox" role="switch" checked={selected.some(choice => choice.id === mod.id)} disabled={busy || !draft} onChange={event => {
        if (draft) change({ ...draft, visualMods: event.currentTarget.checked ? [...selected, { id: mod.id, name: mod.name }] : selected.filter(choice => choice.id !== mod.id) });
      }} />
    </label>)}</div>
    {reply && mods.length === 0 && <p className="map-menu-note">{query ? 'No matching installed mods.' : 'No installed mods found.'}</p>}
    {failureSection === 'mods' && failureLine}
    {reply && !mapDirty && reply.appliedMods.length > 1 && <div className="map-mod-order"><small>Shown in the game’s load order; later mods cover earlier ones.</small>{reply.appliedMods.map(mod => <div key={mod.id}><span>{mod.name}</span></div>)}</div>}
    {reply && !mapDirty && reply.appliedNotes.map(note => <p key={note} className="map-menu-note">{note}</p>)}
    {draft?.visualMods !== null && draft && (clearing && selected.length > 0 ? global
      ? <ConfirmRow title="Show no visual mods?" action="Show none" onCancel={cancelClear} onConfirm={clearMods}
        detail={`${named(selected.map(mod => mod.name))} ${selected.length === 1 ? 'stops' : 'stop'} showing over every campaign’s maps and widgets. Mods chosen for a campaign or map widget stay.`} />
      : <div className="graphics-reset-confirm map-mods-confirm" onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancelClear(); } }}>
        <ConfirmStep confirmation={{ title: `Use ${parentLabel} for mods?`, action: 'Reset mods',
          detail: `${named(selected.map(mod => mod.name))}, chosen here, ${selected.length === 1 ? 'is' : 'are'} cleared, and ${viewId ? 'this map' : 'this recording’s maps and widgets'} show${viewId ? 's' : ''} the mods from ${parentLabel}.` }}
          onCancel={cancelClear} onConfirm={clearMods} /></div>
      : <button ref={clearTrigger} type="button" className="map-menu-action map-mods-inherit" disabled={busy}
        onClick={() => selected.length > 0 ? setClearing(true) : change({ ...draft, visualMods: null })}>{global ? 'Show no visual mods' : `Use ${parentLabel} for mods`}</button>)}
  </>;

  const changedGroups = [
    ...(graphicsCustom && Object.values(draft!.graphics).some(isSet) ? ['map graphics'] : []),
    ...(graphicsCustom && Object.values(draft!.audio).some(isSet) ? ['sound volumes'] : []),
    ...(graphicsCustom && draft!.counters !== null ? ['the counter filter'] : []),
    ...(videoCustom ? ['display settings'] : []),
  ];
  const groupList = changedGroups.length > 1 ? `${changedGroups.slice(0, -1).join(', ')} and ${changedGroups.at(-1)}` : changedGroups[0] ?? 'settings';
  const changedText = groupList[0].toUpperCase() + groupList.slice(1);
  const cancelRevert = () => { setReverting(false); requestAnimationFrame(() => revertTrigger.current?.focus()); };

  if (global) {
    const custom = graphicsCustom || videoCustom;
    const list = (rows: ReactNode, single = false) => <div className={`map-option-list${single ? ' is-single' : ''}`}>{rows}</div>;
    return <div className="map-settings">
      <section className="settings-block" id="graphics-defaults" aria-label="Game settings">
        <div className="settings-group">
          {reverting && custom ? <ConfirmRow title="Revert to your game settings?" action="Revert" onCancel={cancelRevert}
            detail={`${changedText} you changed here follow your game settings again. Campaign and map widget overrides and visual mods stay.`}
            onConfirm={() => { setReverting(false); if (graphicsCustom) change(inheritingAll(draft!)); if (videoCustom) video.reset(); }} />
            : <div className="settings-row"><span className="settings-row-text"><strong>Game settings</strong>
            <small>{custom ? 'Some settings here differ from your game settings.' : 'Every setting follows your game settings until you change it here.'} Campaigns and map widgets can override each one.</small></span>
            {custom ? <button ref={revertTrigger} type="button" className="button small" disabled={busy} onClick={() => setReverting(true)}>
              <Icon name="refresh" />Revert to game settings</button>
              : draft && <span className="settings-value">Following the game</span>}
          </div>}
          {(available && !reply && !error) || reply?.gameSettingsError || reply?.saveWarning || video.warning || video.gameError || failure ? <div className="settings-group-body map-settings-notes">
            {available && !reply && !error && <p className="settings-help" role="status">Reading map settings…</p>}
            {reply?.gameSettingsError && <div className="notice warning" role="status" data-detail={reply.gameSettingsError}><Icon name="alert" /><span>Couldn’t read your game’s graphics settings.</span></div>}
            {video.warning && <div className="notice warning" role="status" data-detail={video.warning}><Icon name="alert" /><span>Saved, but Windows didn’t confirm the display change reached the disk.</span></div>}
            {reply?.saveWarning && <div className="notice warning" role="status" data-detail={reply.saveWarning}><Icon name="alert" /><span>Saved, but Windows didn’t confirm the change reached the disk.</span></div>}
            {video.gameError && <div className="notice warning" role="status" data-detail={video.gameError}><Icon name="alert" /><span>Couldn’t read your game’s display settings.</span></div>}
            {failure}
          </div> : null}
        </div>
      </section>
      <SettingsBlock id="graphics-display" title="Display">{list(<>{display}<VideoControls video={video} busy={busy} /></>)}
        {video.error && <div className="settings-group-body map-section-failure"><FailureLine className="map-menu-error" message={video.writeFailed ? 'Couldn’t save that display change.' : 'Couldn’t read display settings.'}
          detail={video.error} onRetry={video.retry} retryDisabled={video.pending} /></div>}
        {failureIn('display')}</SettingsBlock>
      <SettingsBlock id="graphics-quality" title="Quality">{list(QUALITY.map(([key, label]) => switchRow(key, label)))}{failureIn('quality')}</SettingsBlock>
      {GROUPS.map(group => <SettingsBlock key={group.title} id={`graphics-${group.title.toLowerCase().replaceAll(' ', '-')}`} title={group.title}>
        {list(<>{group.items.map(item => switchRow(item.key, item.label))}{group.title === 'Units' && counterRow}</>)}
        {failureIn(group.title, ...(group.title === 'Units' ? ['counters'] : []))}
      </SettingsBlock>)}
      <SettingsBlock id="graphics-advanced" title="Advanced">
        <details className="settings-details">
          <summary><span className="settings-row-text"><strong>Parts of each layer</strong><small>Each part shows only while its main switch is on.</small></span>
            <Icon name="chevron-down" className="settings-details-chevron" /></summary>
          {parts.map(group => <div className="map-advanced-group" key={group.title}><span className="group-label">{group.title}</span>{list(group.rows)}</div>)}
        </details>
        {failureIn('advanced')}
      </SettingsBlock>
      <SettingsBlock id="graphics-sound" title="Sound" description={SOUND_NOTE}>{list(volumes, true)}{failureIn('sound')}</SettingsBlock>
      <SettingsBlock id="graphics-visual-mods" title="Visual mods" actions={reply && <span className="settings-value">{modCount}</span>}>
        <div className="settings-group-body map-visual-mods-body">{modsBody}</div>
      </SettingsBlock>
    </div>;
  }

  const group = (title: string, rows: ReactNode) => <div className="map-graphics-group" key={title}>
    <span className="group-label">{title}</span><div className="map-graphics-grid">{rows}</div></div>;
  return <div className="map-menu-section map-appearance" data-saving={mapDirty || undefined}>
    <div className="map-appearance-heading"><strong>{scopeLabel}</strong><span className="folder-info" tabIndex={0} role="img" aria-label="About graphics settings" data-tooltip={viewId ? 'Changes apply only to this widget for this campaign. Settings you leave follow the campaign.' : 'Changes apply to this campaign and widgets that follow it. Settings you leave follow the global defaults.'}><Icon name="info" /></span></div>
    {!available && <p className="map-menu-note" role="status">Load a campaign to configure its map graphics and visual mods.</p>}
    {draft && <small className="pill graphics-inheritance">{graphicsCustom ? 'Some settings changed here' : `Using ${parentLabel}`}</small>}
    {reply?.saveWarning && <div className="notice warning" role="status" data-detail={reply.saveWarning}><Icon name="alert" /><span>Saved, but Windows didn’t confirm the change reached the disk.</span></div>}
    <div className="map-graphics-options">
      <div className="map-graphics-fields">{display}</div>
      {failureIn('display')}
      <div className="map-graphics-grid">{QUALITY.map(([key, label]) => switchRow(key, label))}</div>
      {failureIn('quality')}
      {GROUPS.map(item => <Fragment key={item.title}>{group(item.title, item.items.map(entry => switchRow(entry.key, entry.label)))}{failureIn(item.title)}</Fragment>)}
      {counterRow}
      {counterNote}
      {failureIn('counters')}
      {countries}
      <details className="map-visual-mods map-advanced"><summary>Advanced</summary>
        <p className="map-menu-note">Each part shows only while its main switch is on.</p>
        {parts.map(item => group(item.title, item.rows))}
        {failureIn('advanced')}
      </details>
      <div className="map-graphics-group">
        <span className="group-label map-sound-label">Sound <span className="folder-info" tabIndex={0} role="img" aria-label="About map sound" data-tooltip={SOUND_NOTE}><Icon name="info" /></span></span>
        {volumes}
        {failureIn('sound')}
      </div>
    </div>
    {graphicsCustom && (reverting ? <div className="graphics-reset-confirm" onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancelRevert(); } }}>
      <ConfirmStep confirmation={{ title: `Use ${parentLabel} again?`, action: 'Reset',
        detail: `${changedText} you changed here follow ${parentLabel} again. Visual mods stay.` }}
        onCancel={cancelRevert} onConfirm={() => { setReverting(false); change(inheritingAll(draft!)); }} /></div>
      : <button ref={revertTrigger} type="button" className="map-menu-action graphics-reset" disabled={busy} onClick={() => setReverting(true)}>{`Use ${parentLabel}`}</button>)}
    <details className="map-visual-mods"><summary>Visual mods{reply && <span>{modCount}</span>}</summary>{modsBody}</details>
    {available && !reply && !error && <p className="map-menu-note" role="status">Reading map settings…</p>}
    {failure}
  </div>;
}
