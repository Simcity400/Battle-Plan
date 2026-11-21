
export const PUSH = 'http://push.localhost/';

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
