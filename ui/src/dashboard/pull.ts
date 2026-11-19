/** A stream the host pushes to the page, read off its main thread by a worker (`crates/desktop/src/pull.rs`): each ask
 * is answered with everything waiting, or held until something comes, and says the answer before it was handled. A
 * page's first ask is answered empty at once: from then on the stream is the page's, and nothing it asks for is lost. */

export const PUSH = 'http://push.localhost/';

/** An answer's payloads, each in a buffer of its own: a u32 length and its bytes, little-endian. */
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
  /** The page's session: its first ask makes the stream the page's. */
  session: string;
  /** The stream is the page's: its first ask was answered. */
  ready(): void;
  /** An answer's payloads; the next is asked once it resolves. */
  answer(payloads: ArrayBuffer[]): Promise<void>;
  /** Said once when asks start failing. */
  fail(message: string): void;
  ask?: typeof fetch;
  /** Waits before asking again after a failure. */
  pause?: () => Promise<void>;
  /** Whether to stop: asked before each ask. */
  stopped?: () => boolean;
  /** Told as each ask goes. */
  asking?: () => void;
}

/** Delivers in order while the next follow response is in flight. */
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
    // Empty: the page's first answer, or an ask of this page replaced: asked again.
    if (answer.items.length) {
      if (!reader.stopped?.()) next = read();
      await reader.answer(answer.items);
    }
  }
}
