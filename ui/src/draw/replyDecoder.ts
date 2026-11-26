import { Decoder } from '@msgpack/msgpack';
import { utf8Decode } from '@msgpack/msgpack/dist.esm/utils/utf8.mjs';

interface WireString {
  text: string;
  bytes?: Uint8Array;
}

interface CachedString extends WireString {
  hash: number;
  users: number;
  generation: number;
  repeated: boolean;
  next?: CachedString;
}

const MAX_FIXSTR = 0x1f;

function matches(input: Uint8Array, offset: number, length: number, text: string, bytes?: Uint8Array): boolean {
  if (bytes) {
    if (length !== bytes.length) return false;
    for (let i = 0; i < length; i++) if (input[offset + i] !== bytes[i]) return false;
  } else {
    if (length !== text.length) return false;
    for (let i = 0; i < length; i++) if (input[offset + i] !== text.charCodeAt(i)) return false;
  }
  return true;
}

class ShortStrings {
  private readonly strings = new Map<number, CachedString>();
  private readonly firsts = new Map<number, string | WireString>();
  private readonly requests = new Map<number, CachedString[]>();
  private current: CachedString[] = [];
  private generation = 0;

  canBeCached(length: number) { return length <= MAX_FIXSTR; }

  decode(input: Uint8Array, offset: number, length: number): string {
    let hash = length;
    for (let i = 0; i < length; i++) hash = Math.imul(hash ^ input[offset + i]!, 0x01000193);
    const first = this.strings.get(hash);
    for (let entry = first; entry; entry = entry.next) {
      if (!matches(input, offset, length, entry.text, entry.bytes)) continue;
      if (entry.generation !== this.generation) {
        entry.generation = this.generation;
        entry.repeated = false;
      } else if (!entry.repeated) {
        entry.repeated = true;
        entry.users++;
        this.current.push(entry);
      }
      return entry.text;
    }
    const end = offset + length;
    if ((length >= 1 && input[end - 1]! >= 0xc0) || (length >= 2 && input[end - 2]! >= 0xe0) ||
        (length >= 3 && input[end - 3]! >= 0xf0)) return utf8Decode(input, offset, length);
    let slot = hash;
    for (let seen = this.firsts.get(slot); seen !== undefined; seen = this.firsts.get(++slot)) {
      const text = typeof seen === 'string' ? seen : seen.text;
      const bytes = typeof seen === 'string' ? undefined : seen.bytes;
      if (!matches(input, offset, length, text, bytes)) continue;
      const entry = { hash, text, bytes: bytes && Uint8Array.prototype.slice.call(bytes), next: first, users: 1, generation: this.generation, repeated: true };
      this.strings.set(hash, entry);
      this.current.push(entry);
      return text;
    }
    const text = utf8Decode(input, offset, length);
    this.firsts.set(slot, matches(input, offset, length, text) ? text : { text, bytes: input.subarray(offset, end) });
    return text;
  }

  finish(request?: number) {
    this.firsts.clear();
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
