import { PUSH, readStream } from './pull.ts';
import { splitForLanes } from './follow/split.ts';
import type { FromPull, ToPull } from '../draw/laneMessages';

const CREDIT_EVERY_MS = 16;

self.onmessage = ({ data }: MessageEvent<{ session: string; ports: MessagePort[] }>) => {
  const { ports, session } = data;
  let owed = 0, timer: ReturnType<typeof setTimeout> | undefined;
  const credit = (bytes: number) => {
    owed += bytes;
    timer ??= setTimeout(() => {
      timer = undefined;
      const bytes = owed;
      owed = 0;
      if (bytes !== 0) void fetch(`${PUSH}credit?session=${encodeURIComponent(session)}&bytes=${bytes}`, { cache: 'no-store' }).catch(() => {});
    }, CREDIT_EVERY_MS);
  };
  for (const port of ports) port.onmessage = ({ data }: MessageEvent<ToPull>) => credit('release' in data ? data.release : -data.charge);
  const all = (message: FromPull) => { for (const port of ports) port.postMessage(message); };
  void readStream({ session,
    ready: () => self.postMessage('ready'),
    fail: reason => { all({ kind: 'failure', reason }); self.postMessage({ reason }); },
    answer: async buffers => {
      for (const buffer of buffers) {
        let split;
        try { split = splitForLanes(new Uint8Array(buffer), ports.length); }
        catch (error) { all({ kind: 'failure', reason: `A pushed message could not be read: ${error instanceof Error ? error.message : String(error)}` }); credit(buffer.byteLength); continue; }
        if (split.kind === 'failure') { all({ kind: 'failure', reason: split.reason }); continue; }
        credit(split.header);
        split.lanes.forEach((bytes, lane) => { if (bytes) ports[lane]!.postMessage({ kind: split.kind, bytes } satisfies FromPull, [bytes.buffer]); });
      }
    },
  });
};
