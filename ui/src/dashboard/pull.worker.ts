import { PUSH, readStream } from './pull.ts';
import { charge, packet, resetStamps, splitForLanes } from './follow/split.ts';
import { credits } from './credits.ts';
import type { FromPull, ToPull } from '../draw/laneMessages';

const CREDIT_EVERY_MS = 16;

self.onmessage = ({ data }: MessageEvent<{ session: string; ports: MessagePort[] }>) => {
  const { ports, session } = data;
  const owed = credits(released => fetch(`${PUSH}credit?session=${encodeURIComponent(session)}&released=${released}`, { cache: 'no-store' }).then(reply => reply.ok),
    run => setTimeout(run, CREDIT_EVERY_MS));
  const credit = (bytes: number) => owed.owe(bytes);
  for (const port of ports) port.onmessage = ({ data }: MessageEvent<ToPull>) => credit(data.release);
  const all = (message: FromPull) => { for (const port of ports) port.postMessage(message); };
  const stamp = resetStamps();
  void readStream({ session,
    ready: () => self.postMessage('ready'),
    fail: reason => { all({ kind: 'failure', reason }); self.postMessage({ reason }); },
    answer: async buffers => {
      for (const buffer of buffers) {
        let split, resets: number | undefined;
        try { resets = stamp(new Uint8Array(buffer)); split = splitForLanes(new Uint8Array(buffer), ports.length); }
        catch (error) { all({ kind: 'failure', reason: `A pushed message could not be read: ${error instanceof Error ? error.message : String(error)}` }); credit(charge(new Uint8Array(buffer))); continue; }
        if (split.kind === 'failure') { all({ kind: 'failure', reason: split.reason }); continue; }
        credit(split.header);
        split.lanes.forEach((bytes, lane) => { if (bytes) ports[lane]!.postMessage(packet(split.kind, bytes, resets), [bytes.buffer]); });
      }
    },
  });
};
