import { Decoder } from '@msgpack/msgpack';
import { utf8Decode } from '@msgpack/msgpack/dist.esm/utils/utf8.mjs';

interface CachedString {
  bytes: Uint8Array;
  text: string;
  next?: CachedString;
}

const MAX_FIXSTR = 0x1f;

class ShortStrings {
  private readonly strings = new Map<number, CachedString>();

  canBeCached(length: number) { return length <= MAX_FIXSTR; }

  decode(input: Uint8Array, offset: number, length: number): string {
    let hash = length;
    for (let i = 0; i < length; i++) hash = Math.imul(hash ^ input[offset + i]!, 0x01000193);
    const first = this.strings.get(hash);
    for (let entry = first; entry; entry = entry.next) {
      if (entry.bytes.length !== length) continue;
      let i = 0;
      while (i < length && input[offset + i] === entry.bytes[i]) i++;
      if (i === length) return entry.text;
    }
    const text = utf8Decode(input, offset, length);
    const end = offset + length;
    if ((length >= 1 && input[end - 1]! >= 0xc0) || (length >= 2 && input[end - 2]! >= 0xe0) ||
        (length >= 3 && input[end - 3]! >= 0xf0)) return text;
    this.strings.set(hash, { text, bytes: Uint8Array.prototype.slice.call(input, offset, offset + length), next: first });
    return text;
  }
}

export class ReplyDecoder {
  private readonly decoder = new Decoder({ keyDecoder: new ShortStrings() });

  constructor() {
    Object.defineProperty(this.decoder, 'stateIsMapKey', { value: () => true });
  }

  readonly decode = (input: Uint8Array): unknown => this.decoder.decode(input);
}
