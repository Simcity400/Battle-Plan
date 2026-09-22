import type { SourceReply } from '../generated/dashboard';
import type { DashboardClient, DashboardSource, MapSettingsReply, ProductReply, ProductRequest, ReplayLibrary } from './client';

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
  available: true, nativeMaps: false,
  contentFolders: signal => request({ type: 'contentFolders' }, signal),
  chooseContentFolder: (kind, initial) => request({ type: 'chooseFolder', kind, initial }),
  setContentFolders: (game, user) => request({ type: 'setContentFolders', game, user }),
  chooseRecording,
  replayLibrary: signal => request<ReplayLibrary>({ type: 'library' }, signal),
  setReplayFolder: folder => request<ReplayLibrary>({ type: 'setReplayFolder', folder }),
  chooseReplayFolder: (initial = '') => request({ type: 'chooseFolder', kind: 'replay', initial }),

  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const reply = await request<MapSettingsReply>({ type: 'mapSettings', sourceId, revision, settings, target }, signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    return reply;
  },

  async openSource(path, signal): Promise<DashboardSource> {
    const source = await request<SourceReply>({ type: 'open', path }, signal);
    return { id: source.id, path, revision: source.revision, metadata: { ...source.metadata, revision: source.revision } };
  },

  product<T>(query: ProductRequest, signal: AbortSignal) {
    return request<ProductReply<T>>({ type: 'product', request: query }, signal);
  },

  async releaseSource(sourceId) { await request({ type: 'release', sourceId }); },
  async setMapViews() {},
  async onMapStates() { return () => {}; },
};
