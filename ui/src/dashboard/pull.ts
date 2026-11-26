
import { charge, packet, resetStamps, splitForLanes } from './follow/split.ts';
import type { FromPull, ToPull } from '../draw/laneMessages';

export const PUSH = 'http://push.localhost/';

export interface PullDispatcher {
  dispatch(buffer: ArrayBuffer | Uint8Array): void;
  retireLane(lane: number): void;
  failAll(reason: string): void;
  outstanding(lane?: number): number;
}

export function createPullDispatcher(
  ports: readonly MessagePort[],
  credit: (bytes: number) => void,
  onFailure?: (reason: string) => void,
): PullDispatcher {
  const held = Array.from({ length: ports.length }, () => 0);
  const dead = Array.from({ length: ports.length }, () => false);
  const stamp = resetStamps();
  ports.forEach((port, lane) => {
    port.onmessage = ({ data }: MessageEvent<ToPull>) => {
      if (dead[lane] || !data || data.release <= 0) return;
      held[lane] = Math.max(0, held[lane]! - data.release);
      credit(data.release);
    };
  });
  const failAll = (reason: string) => {
    for (let lane = 0; lane < ports.length; lane++) {
      if (!dead[lane]) ports[lane]!.postMessage({ kind: 'failure', reason } satisfies FromPull);
    }
    onFailure?.(reason);
  };
  return {
    dispatch(buffer) {
      const raw = buffer instanceof Uint8Array ? buffer : new Uint8Array(buffer);
      let split, resets: number | undefined;
      try { resets = stamp(raw); split = splitForLanes(buffer, ports.length); }
      catch (error) {
        failAll(`A pushed message could not be read: ${error instanceof Error ? error.message : String(error)}`);
        credit(charge(raw));
        return;
      }
      if (split.kind === 'failure') { failAll(split.reason); return; }
      let immediate = split.header;
      split.lanes.forEach((bytes, lane) => {
        if (!bytes) return;
        const owned = split.owned[lane]!;
        if (dead[lane]) {
          immediate += owned;
          return;
        }
        held[lane]! += owned;
        const msg = packet(split.kind, bytes, resets);
        if (bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength) {
          ports[lane]!.postMessage(msg, [bytes.buffer]);
        } else {
          ports[lane]!.postMessage(msg);
        }
      });
      if (immediate > 0) credit(immediate);
    },
    retireLane(lane) {
      if (lane < 0 || lane >= ports.length || dead[lane]) return;
      dead[lane] = true;
      ports[lane]!.onmessage = null;
      ports[lane]!.close();
      const owed = held[lane]!;
      held[lane] = 0;
      if (owed > 0) credit(owed);
    },
    failAll,
    outstanding(lane) {
      if (lane !== undefined) return held[lane] ?? 0;
      let total = 0;
      for (const bytes of held) total += bytes;
      return total;
    },
  };
}

export function payloads(body: ArrayBuffer): ArrayBuffer[] {
  const view = new DataView(body), out: ArrayBuffer[] = [];
  for (let at = 0; at < body.byteLength;) {
    if (at + 4 > body.byteLength) throw new Error('A pushed answer ends inside a payload\'s length');
    const length = view.getUint32(at, true);
    at += 4;
    if (at + length > body.byteLength) throw new Error('A pushed answer ends inside a payload');
    out.push(body.slice(at, at + length));
    at += length;
  }
  return out;
}

export interface StreamReader {
  session: string;
  ready(): void;
  answer(payloads: ArrayBuffer[]): Promise<void>;
  fail(message: string): void;
  ask?: typeof fetch;
  pause?: () => Promise<void>;
  stopped?: () => boolean;
  asking?: () => void;
}

export async function readStream(reader: StreamReader): Promise<void> {
  const ask = reader.ask ?? fetch, pause = reader.pause ?? (() => new Promise<void>(resolve => setTimeout(resolve, 1000)));
  const url = `${PUSH}follow?session=${encodeURIComponent(reader.session)}`;
  let failing = false, registered = false;
  const read = async () => {
    try {
      reader.asking?.();
      const response = await ask(url, { cache: 'no-store' });
      if (!response.ok) throw new Error(`the host answered ${response.status}: ${await response.text()}`);
      return { ok: true as const, items: payloads(await response.arrayBuffer()) };
    } catch (error) {
      return { ok: false as const, error };
    }
  };
  let next: ReturnType<typeof read> | null = null;
  while (next || !reader.stopped?.()) {
    const answer = await (next ?? read());
    next = null;
    if (!answer.ok) {
      const { error } = answer;
      if (!failing) reader.fail(`The follow stream could not be read: ${error instanceof Error ? error.message : String(error)}`);
      failing = true;
      await pause();
      continue;
    }
    failing = false;
    if (!registered) { registered = true; reader.ready(); }
    if (answer.items.length) {
      if (!reader.stopped?.()) next = read();
      await reader.answer(answer.items);
    }
  }
}
