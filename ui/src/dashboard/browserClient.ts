import { queryFailure, RefusedError } from './queryError.ts';
import type { SourceReply } from '../generated/dashboard';
import type { DashboardClient, DashboardSource, MapSettingsReply, ProductReply, ProductRequest, ReplayLibrary } from './client';
import { replyLengths } from './products.ts';
import { parseReplyBytes } from './replyParse.ts';
import { contentRevision } from './contentRevision.ts';
import { forgetReplySizes } from './replySizes.ts';
import { INTERACTIVE_PRODUCTS, REPLAY_PRODUCTS } from '../generated/dashboard.ts';
import { BrowserRequestQueue } from './browserRequestQueue.ts';
import type { BrowserRequestLane } from './browserRequestQueue';
import { newId } from '../shell/ids.ts';
import { busy } from '../shell/busy.ts';
import { OpenedSources } from './openedSources.ts';

const requests = new BrowserRequestQueue();
const opened = new OpenedSources();
const page = newId();
let owning = false;
function ownRecordings(): void {
  if (owning || typeof window === 'undefined') return;
  owning = true;
  addEventListener('pagehide', event => {
    if (!event.persisted) navigator.sendBeacon('/api/recordings', new Blob([JSON.stringify({ type: 'pageGone', page })], { type: 'application/json' }));
  });
}
const interactiveProducts: ReadonlySet<string> = new Set(INTERACTIVE_PRODUCTS);
const replayProducts: ReadonlySet<string> = new Set(REPLAY_PRODUCTS);

function chooseRecording(): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const input = document.createElement('input');
    input.type = 'file'; input.accept = '.battleplan'; input.hidden = true;
    document.body.append(input);
    input.addEventListener('cancel', () => { input.remove(); resolve(null); }, { once: true });
    input.addEventListener('change', () => {
      const file = input.files?.[0]; input.remove();
      if (!file) { resolve(null); return; }
      if (!/\.battleplan$/i.test(file.name)) { reject(new Error('Choose a BattlePlan recording.')); return; }
      if (file.size > 2 * 1024 * 1024 * 1024) { reject(new Error('Open recordings larger than 2 GB in the desktop app.')); return; }
      ownRecordings();
      void requests.run('history', async signal => {
        const response = await fetch('/api/recordings/import', { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-Recording-Name': encodeURIComponent(file.name), 'X-Recording-Page': page }, body: file, signal });
        const data = await response.json();
        if (!response.ok) throw queryFailure(data.error);
        return data.path as string;
      }).then(resolve, reject);
    }, { once: true });
    input.click();
  });
}

async function request<T>(body: { type: string; request?: ProductRequest; sourceId?: string; [key: string]: unknown }, signal?: AbortSignal,
  releaseLate?: (value: T) => Promise<void>): Promise<T> {
  const lane: BrowserRequestLane = body.type === 'release' ? 'control'
    : body.type === 'product' && replayProducts.has(body.request!.product) ? 'replays'
    : body.type === 'product' && interactiveProducts.has(body.request!.product) ? 'interactive' : 'history';
  ownRecordings();
  const done = busy('requests');
  return requests.run(lane, async active => {
    const response = await fetch('/api/recordings', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: active });
    const bytes = await response.arrayBuffer();
    const length = bytes.byteLength;
    const data = await parseReplyBytes(bytes) as { error?: unknown } | null;
    if (!response.ok) throw queryFailure(data?.error as Parameters<typeof queryFailure>[0]);
    if (data && typeof data === 'object') replyLengths.set(data, length);
    return data as T;
  }, { signal, sourceId: body.type === 'release' ? undefined : body.request?.sourceId ?? body.sourceId, refresh: body.refresh === true, onCancelledValue: releaseLate }).finally(done);
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

  chooseHistory: async (path, segmentId, choice, signal) => { await request<unknown>({ type: 'historyChoice', id: crypto.randomUUID(), path, segmentId, choice }, signal); },
  async mapSettings(sourceId, revision, settings, signal, target = { scope: 'campaign' }) {
    const reply = await request<MapSettingsReply>({ type: 'mapSettings', sourceId, revision, settings, flip: null, target }, signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    contentRevision(browserDashboardClient).observePresentation(reply.presentationGeneration);
    return reply;
  },
  async flipMapSetting(sourceId, revision, setting, signal, target) {
    const reply = await request<MapSettingsReply>({ type: 'mapSettings', sourceId, revision, settings: null, flip: setting, target }, signal);
    if (reply.sourceId !== sourceId || reply.revision !== revision) throw new Error('Map settings do not match this campaign');
    contentRevision(browserDashboardClient).observePresentation(reply.presentationGeneration);
    return reply;
  },


  async openSource(path, signal): Promise<DashboardSource> {
    const releaseLate = async (late: SourceReply) => { await request({ type: 'release', sourceId: late.id }); };
    const source = await request<SourceReply>({ type: 'open', path, page }, signal, releaseLate);
    if (signal.aborted) {
      await releaseLate(source).catch(() => {});
      throw new DOMException('Request cancelled', 'AbortError');
    }
    contentRevision(browserDashboardClient).observePresentation(source.presentationGeneration);
    return opened.add({ id: source.id, path, revision: source.revision, metadata: { ...source.metadata, revision: source.revision } });
  },

  product<T>(query: ProductRequest, signal: AbortSignal) {
    const refused = opened.refusal(query);
    if (refused) return Promise.reject(new RefusedError(refused));
    return request<ProductReply<T>>({ type: 'product', request: query }, signal);
  },

  async releaseSource(sourceId) {
    opened.delete(sourceId);
    forgetReplySizes(browserDashboardClient, sourceId);
    requests.cancelSource(sourceId);
    await request({ type: 'release', sourceId });
  },
  async setMapViews() {},
  async onMapStates() { return () => {}; },
};
