import { createPullDispatcher, payloads, PUSH, readStream, type PullDispatcher } from './pull.ts';
import { credits } from './credits.ts';
import { restorePanelBatches } from './follow/panelWire.ts';

const CREDIT_EVERY_MS = 16;

let dispatcher: PullDispatcher | null = null;

self.onmessage = ({ data }: MessageEvent<{ session: string; ports: MessagePort[]; panelDeltas: boolean } | { type: 'retire-lane'; lane: number }>) => {
  if ('type' in data) {
    if (data.type === 'retire-lane') dispatcher?.retireLane(data.lane);
    return;
  }
  const { ports, session, panelDeltas } = data;
  const owed = credits(released => fetch(`${PUSH}credit?session=${encodeURIComponent(session)}&released=${released}`, { cache: 'no-store' }).then(reply => reply.ok),
    run => setTimeout(run, CREDIT_EVERY_MS));
  dispatcher = createPullDispatcher(ports, bytes => owed.owe(bytes));
  const fail = (reason: string) => { dispatcher!.failAll(reason); self.postMessage({ reason }); };
  const answer = panelDeltas ? restorePanelBatches(payload => dispatcher!.dispatch(payload), async serial => {
    const response = await fetch(`${PUSH}follow?session=${encodeURIComponent(session)}&whole=${serial}`, { cache: 'no-store' });
    if (!response.ok) throw new Error(`The full panel batch could not be read: ${response.status}.`);
    return payloads(await response.arrayBuffer());
  }) : async (buffers: ArrayBuffer[]) => {
    for (const buffer of buffers) dispatcher!.dispatch(buffer);
  };
  void readStream({ session,
    ready: () => self.postMessage('ready'),
    fail,
    answer,
  }).catch(error => fail(error instanceof Error ? error.message : String(error)));
};
