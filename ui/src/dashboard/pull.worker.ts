import { createPullDispatcher, PUSH, readStream, type PullDispatcher } from './pull.ts';
import { credits } from './credits.ts';

const CREDIT_EVERY_MS = 16;

let dispatcher: PullDispatcher | null = null;

self.onmessage = ({ data }: MessageEvent<{ session: string; ports: MessagePort[] } | { type: 'retire-lane'; lane: number }>) => {
  if ('type' in data) {
    if (data.type === 'retire-lane') dispatcher?.retireLane(data.lane);
    return;
  }
  const { ports, session } = data;
  const owed = credits(released => fetch(`${PUSH}credit?session=${encodeURIComponent(session)}&released=${released}`, { cache: 'no-store' }).then(reply => reply.ok),
    run => setTimeout(run, CREDIT_EVERY_MS));
  dispatcher = createPullDispatcher(ports, bytes => owed.owe(bytes));
  void readStream({ session,
    ready: () => self.postMessage('ready'),
    fail: reason => { dispatcher!.failAll(reason); self.postMessage({ reason }); },
    answer: async buffers => {
      for (const buffer of buffers) dispatcher!.dispatch(buffer);
    },
  });
};
