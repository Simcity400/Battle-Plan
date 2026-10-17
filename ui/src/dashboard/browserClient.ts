import { validatePreparationReply, validatePreparationPreferences } from '../shell/preparation/contract.ts';
import { preparedReplyData, validateProductReply } from './client.ts';
import type { PreparationAction, PreparationRequest, PreparationReply, PreparationPreferences } from '../shell/preparation/contract';
import type { SourceReply } from '../generated/dashboard';
import type { DashboardClient, DashboardSource, HistoryCacheReply, MapSettingsReply, PreparedQueryOptions, ProductReply, ProductRequest, ReplayLibrary } from './client';
import { replyLengths } from './products.ts';
import { INTERACTIVE_PRODUCTS } from '../generated/dashboard.ts';
import { BrowserRequestQueue } from './browserRequestQueue.ts';
import type { BrowserRequestLane } from './browserRequestQueue';

const requests = new BrowserRequestQueue();
const interactiveProducts: ReadonlySet<string> = new Set(INTERACTIVE_PRODUCTS);

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
      void requests.run('history', async signal => {
        const response = await fetch('/api/recordings/import', { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-Recording-Name': encodeURIComponent(file.name) }, body: file, signal });
        const data = await response.json();
        if (!response.ok) throw new Error(data.error ?? 'Cannot open recording');
        return data.path as string;
      }).then(resolve)
        .catch(reject);
    }, { once: true });
    input.click();
  });
}

async function request<T>(body: { type: string; request?: ProductRequest; sourceId?: string; [key: string]: unknown }, signal?: AbortSignal,
  releaseLate?: (value: T) => Promise<void>): Promise<T> {
  const lane: BrowserRequestLane = body.type === 'release' ? 'control'
    : body.type === 'preparedQuery' || body.type === 'preparation' || body.type === 'product' && interactiveProducts.has(body.request!.product) ? 'interactive' : 'history';
  return requests.run(lane, async active => {
    const response = await fetch('/api/recordings', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: active });
    const text = await response.text();
    const data = JSON.parse(text);
    if (!response.ok) throw new Error(data.error ?? 'Cannot read recording');
    if (data && typeof data === 'object') replyLengths.set(data, text.length);
    return data;
  }, { signal, sourceId: body.type === 'release' ? undefined : body.request?.sourceId ?? body.sourceId, refresh: body.refresh === true, onCancelledValue: releaseLate });
}

/** Recordings this page holds open. The bridge outlives the page, so a page that goes
 * away (closed or reloaded) without unloading them releases them as it leaves.
 * A back/forward-cache visit retains the page and its source handles and leases. */
const leases = new Set<string>();
if (typeof window !== 'undefined') addEventListener('pagehide', event => {
  if (event.persisted) return;
  for (const sourceId of leases) navigator.sendBeacon('/api/recordings', new Blob([JSON.stringify({ type: 'release', sourceId })], { type: 'application/json' }));
});

async function preparation(action: PreparationAction, query: PreparationRequest, signal: AbortSignal) {
  return validatePreparationReply(await request<PreparationReply>({ type: 'preparation', id: query.requestId, sourceId: query.sourceId, revision: query.revision, action }, signal), query);
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

  preparationPreferences: async (enabled, signal) => validatePreparationPreferences(await request<PreparationPreferences>({ type: 'preparationPreferences', id: crypto.randomUUID(), enabled }, signal)),
  preparationStatus: (query, signal) => preparation('status', query, signal),
  async preparedQuery<T>(source: DashboardSource, product: string, params: unknown, options: PreparedQueryOptions): Promise<T> {
    const query = { sourceId: source.id, revision: source.revision, product, params };
    const reply = await request<ProductReply<T>>({ type: 'preparedQuery', request: query, refresh: options.refresh ?? false }, options.signal);
    return preparedReplyData(validateProductReply(reply, query), options);
  },
  prepareSource: (query, signal) => preparation('start', query, signal),
  cancelPreparation: (query, signal) => preparation('cancel', query, signal),
  clearPreparation: (query, signal) => preparation('clear', query, signal),
  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const reply = await request<MapSettingsReply>({ type: 'mapSettings', sourceId, revision, settings, flip: null, target }, signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    return reply;
  },
  async flipMapSetting(sourceId, revision, setting, signal, target) {
    const reply = await request<MapSettingsReply>({ type: 'mapSettings', sourceId, revision, settings: null, flip: setting, target }, signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    return reply;
  },

  historyCache: (clear, signal) => request<HistoryCacheReply>({ type: 'historyCache', clear }, signal),

  async openSource(path, signal): Promise<DashboardSource> {
    const releaseLate = async (late: SourceReply) => {
      leases.add(late.id);
      await request({ type: 'release', sourceId: late.id });
      leases.delete(late.id);
    };
    const source = await request<SourceReply>({ type: 'open', path }, signal, releaseLate);
    if (signal.aborted) {
      await releaseLate(source).catch(() => {});
      throw new DOMException('Request cancelled', 'AbortError');
    }
    leases.add(source.id);
    return { id: source.id, path, revision: source.revision, metadata: { ...source.metadata, revision: source.revision } };
  },

  product<T>(query: ProductRequest, signal: AbortSignal) {
    return request<ProductReply<T>>({ type: 'product', request: query }, signal);
  },

  async releaseSource(sourceId) {
    requests.cancelSource(sourceId);
    await request({ type: 'release', sourceId });
    leases.delete(sourceId);
  },
  async setMapViews() {},
  async onMapStates() { return () => {}; },
};
