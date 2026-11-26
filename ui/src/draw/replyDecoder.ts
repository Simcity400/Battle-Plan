import { Decoder } from '@msgpack/msgpack';
import { utf8Decode } from '@msgpack/msgpack/dist.esm/utils/utf8.mjs';

interface CachedString {
  hash: number;
  bytes: Uint8Array;
  text: string;
  users: number;
  generation: number;
  next?: CachedString;
}

const MAX_FIXSTR = 0x1f;

class ShortStrings {
  private readonly strings = new Map<number, CachedString>();
  private readonly requests = new Map<number, CachedString[]>();
  private current: CachedString[] = [];
  private generation = 0;

  canBeCached(length: number) { return length <= MAX_FIXSTR; }

  decode(input: Uint8Array, offset: number, length: number): string {
    let hash = length;
    for (let i = 0; i < length; i++) hash = Math.imul(hash ^ input[offset + i]!, 0x01000193);
    const first = this.strings.get(hash);
    for (let entry = first; entry; entry = entry.next) {
      if (entry.bytes.length !== length) continue;
      let i = 0;
      while (i < length && input[offset + i] === entry.bytes[i]) i++;
      if (i === length) {
        if (entry.generation !== this.generation) {
          entry.generation = this.generation;
          entry.users++;
          this.current.push(entry);
        }
        return entry.text;
      }
    }
    const text = utf8Decode(input, offset, length);
    const end = offset + length;
    if ((length >= 1 && input[end - 1]! >= 0xc0) || (length >= 2 && input[end - 2]! >= 0xe0) ||
        (length >= 3 && input[end - 3]! >= 0xf0)) return text;
    const entry = { hash, text, bytes: Uint8Array.prototype.slice.call(input, offset, end), next: first, users: 1, generation: this.generation };
    this.strings.set(hash, entry);
    this.current.push(entry);
    return text;
  }

  finish(request?: number) {
    const previous = request === undefined ? this.current : this.requests.get(request);
    if (request !== undefined) {
      if (this.current.length) this.requests.set(request, this.current);
      else this.requests.delete(request);
    }
    if (previous) this.release(previous);
    this.current = previous ?? [];
    this.generation++;
  }

  forget(request: number) {
    const entries = this.requests.get(request);
    if (!entries) return;
    this.requests.delete(request);
    this.release(entries);
  }

  private release(entries: CachedString[]) {
    for (const entry of entries) {
      if (--entry.users) continue;
      const head = this.strings.get(entry.hash)!;
      if (head !== entry) {
        let before = head;
        while (before.next !== entry) before = before.next!;
        before.next = entry.next;
      } else if (entry.next) this.strings.set(entry.hash, entry.next);
      else this.strings.delete(entry.hash);
    }
    entries.length = 0;
  }
}

export class ReplyDecoder {
  private readonly strings = new ShortStrings();
  private readonly decoder = new Decoder({ keyDecoder: this.strings });

  constructor() {
    Object.defineProperty(this.decoder, 'stateIsMapKey', { value: () => true });
  }

  readonly decode = (input: Uint8Array, request = 0): unknown => {
    let value: unknown;
    try {
      value = this.decoder.decode(input);
    } catch (error) {
      this.strings.finish();
      throw error;
    }
    this.strings.finish(request);
    return value;
  };

  forget(request: number) { this.strings.forget(request); }
}
