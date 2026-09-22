import { validateResearchReply } from './researchData';
import { validateFocusReply } from './focusData';
import type { FocusReply, ResearchReply } from './client';
import type { MapSettingsReply, DashboardClient, DashboardSource, CountryFlagReply, LandOwnersReply, ReplayLibrary } from './client';
import type { QueryResult } from '../generated/analytics';
import { newId } from '../ids';

function chooseRecording(): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const input = document.createElement('input');
    input.type = 'file'; input.accept = '.battleplan,.hoi4replay'; input.hidden = true;
    document.body.append(input);
    input.addEventListener('cancel', () => { input.remove(); resolve(null); }, { once: true });
    input.addEventListener('change', () => {
      const file = input.files?.[0]; input.remove();
      if (!file) { resolve(null); return; }
      if (!(/\.(?:battleplan|hoi4replay)$/i.test(file.name))) { reject(new Error('Choose a .battleplan recording')); return; }
      if (file.size > 2 * 1024 * 1024 * 1024) { reject(new Error('Open recordings larger than 2 GB in the desktop app.')); return; }
      void fetch('/api/recordings/import', { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-Recording-Name': encodeURIComponent(file.name) }, body: file })
        .then(async response => { const data = await response.json(); if (!response.ok) throw new Error(data.error ?? 'Cannot open recording'); resolve(data.path); })
        .catch(reject);
    }, { once: true });
    input.click();
  });
}

async function request<T>(body: unknown, signal?: AbortSignal): Promise<T> {
  const response = await fetch('/api/recordings', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error ?? 'Cannot read recording');
  return data;
}

export const browserDashboardClient: DashboardClient = {
  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const reply = await request<MapSettingsReply>({ type: 'mapSettings', sourceId, revision, settings, target }, signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    if (!reply.inheritedSettings) throw new Error('The app or local bridge needs updating to support global and widget settings.');
    return reply;
  },
  contentFolders: signal => request({ type: 'contentFolders' }, signal),
  chooseContentFolder: (kind, initial) => request({ type: 'chooseFolder', kind, initial }),
  setContentFolders: (game, user) => request({ type: 'setContentFolders', game, user }),
  available: true, nativeMaps: false,
  chooseRecording,
  replayLibrary: signal => request<ReplayLibrary>({ type: 'library' }, signal),
  setReplayFolder: folder => request<ReplayLibrary>({ type: 'setReplayFolder', folder }),
  chooseReplayFolder: (initial = '') => request({ type: 'chooseFolder', kind: 'replay', initial }),
  async landOwners(sourceId, revision, signal) {
    const reply = await request<LandOwnersReply>({ type: 'landOwners', sourceId, revision }, signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Countries do not match this recording');
    return reply;
  },
  async openSource(path, signal) {
    const source = await request<DashboardSource>({ type: 'open', path }, signal);
    return { ...source, path, metadata: { ...source.metadata, revision: source.revision } };
  },
  async focusHistory(query, signal) {
    const reply = await request<FocusReply>({ type: 'focusHistory', request: query }, signal);
    return validateFocusReply(reply, query);
  },

  async researchHistory(query, signal) {
    const reply = await request<ResearchReply>({ type: 'researchHistory', request: query }, signal);
    return validateResearchReply(reply, query);
  },
  async countryFlags(query, signal) {
    const reply = await request<CountryFlagReply>({ type: 'countryFlags', request: query }, signal);
    if (reply.size !== query.size || reply.sourceId !== query.sourceId || reply.revision !== query.revision || reply.branchId !== query.branchId || reply.hour !== query.hour
      || reply.flags.some(flag => !query.tags.includes(flag.tag) || flag.width < 1 || flag.width > 128 || flag.height < 1 || flag.height > 128 || flag.pixels.length !== flag.width * flag.height * 4)) throw new Error('Flag response does not match this recording');
    return { ...reply, flags: reply.flags.map(flag => ({ ...flag, pixels: new Uint8ClampedArray(flag.pixels) })) };
  },
  async query(query, signal, priority) {
    const wire = { scopeId: newId(), generation: '1', ...query, kind: query.kind === 'exact' ? 'exactWindow' : 'history', priority: priority ?? (query.kind === 'exact' ? 'current' : 'history') };
    const result = await request<QueryResult>({ type: 'query', request: wire }, signal);
    if (result.revision !== query.revision || result.branchId !== query.branchId || result.firstHour !== query.firstHour || result.lastHour !== query.lastHour || result.kind !== wire.kind
      || result.maxPoints !== query.maxPoints || JSON.stringify([...result.countryIds].sort()) !== JSON.stringify([...new Set(query.countryIds)].sort())) throw new Error('Query response does not match this recording');
    return { sourceId: query.sourceId, revision: result.revision, series: result.series.map(series => ({ ...series, revision: result.revision })) };
  },
  async releaseSource(sourceId) { await request({ type: 'release', sourceId }); },
  async setMapViews() {},
  async onMapStates() { return () => {}; },
};
