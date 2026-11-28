import { DATED_PANEL, DATED_PANEL_VERSION } from './message.ts';

const PATCHED = 0x31444e50;
const BATCH = 0x31425750;
const DELTA = 8;
const MAX_BASE_BYTES = 64 << 20;
const aligned = (at: number) => Math.ceil(at / 8) * 8;

const fingerprint = (bytes: Uint8Array) => {
  let hash = 0x811c9dc5;
  for (const byte of bytes) hash = Math.imul(hash ^ byte, 0x01000193);
  return hash >>> 0;
};

function apply(base: Uint8Array | undefined, bytes: Uint8Array): Uint8Array {
  const patch = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (!base || bytes.length < 12 || fingerprint(base) !== patch.getUint32(0, true)) {
    throw new Error('The panel delta has no matching base.');
  }
  const length = patch.getUint32(4, true);
  if (length > MAX_BASE_BYTES) throw new Error('A restored panel exceeds its byte allowance.');
  const out = new Uint8Array(length);
  let wrote = 0;
  for (let at = 12; at < bytes.length;) {
    const op = patch.getUint32(at, true), count = op & 0x7fff_ffff;
    at += 4;
    if (!count || count > length - wrote) throw new Error('A panel delta has an invalid span.');
    if (op >>> 31) {
      if (count > bytes.length - at) throw new Error('A panel delta ends inside a literal.');
      out.set(bytes.subarray(at, at + count), wrote);
      at += count;
    } else {
      const offset = patch.getUint32(at, true);
      at += 4;
      if (count > base.length - offset) throw new Error('A panel delta reads past its base.');
      out.set(base.subarray(offset, offset + count), wrote);
    }
    wrote += count;
  }
  if (wrote !== length || fingerprint(out) !== patch.getUint32(8, true)) throw new Error('A panel delta did not restore the expected value.');
  return out;
}

type Base = { route: number; seek: number; body: Uint8Array };

class PanelBodies {
  private readonly bases = new Map<number, Base>();
  private bytes = 0;

  private forget(id: number) {
    const base = this.bases.get(id);
    if (base) { this.bytes -= base.body.length; this.bases.delete(id); }
  }

  private remember(id: number, route: number, seek: number, body: Uint8Array) {
    this.forget(id);
    if (body.length > MAX_BASE_BYTES) return;
    if (this.bytes + body.length > MAX_BASE_BYTES) {
      this.bases.clear();
      this.bytes = 0;
    }
    this.bases.set(id, { route, seek, body });
    this.bytes += body.length;
  }

  decode(buffer: ArrayBuffer): ArrayBuffer {
    const data = new DataView(buffer);
    if (data.byteLength < 4) return buffer;
    const magic = data.getUint32(0, true);
    if (magic !== PATCHED && magic !== DATED_PANEL) return buffer;
    if (data.byteLength < 24 || data.getUint16(4, true) !== DATED_PANEL_VERSION) throw new Error('The panel body has an unsupported header.');
    const patched = magic === PATCHED;
    const count = data.getUint32(8, true), route = data.getUint32(16, true), seek = data.getUint32(20, true);
    const bytes = new Uint8Array(buffer);
    const pieces: Uint8Array[] = [];
    let at = 24;
    for (let i = 0; i < count; i++) {
      const id = data.getUint32(at, true), kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true);
      const end = aligned(at + 16 + length);
      if (end > bytes.length) throw new Error('A panel ends inside its record.');
      const body = bytes.subarray(at + 16, at + 16 + length);
      if (kind === DELTA) {
        if (!patched) throw new Error('A canonical panel contains a delta.');
        const base = this.bases.get(id);
        const next = apply(base?.route === route && base.seek === seek ? base.body : undefined, body);
        this.remember(id, route, seek, next);
        const record = new Uint8Array(aligned(16 + next.length));
        record.set(bytes.subarray(at, at + 16));
        record[8] = 1;
        new DataView(record.buffer).setUint32(12, next.length, true);
        record.set(next, 16);
        pieces.push(record);
      } else {
        if (kind === 1) this.remember(id, route, seek, body.slice());
        else if (kind === 0 || kind === 7) this.forget(id);
        if (patched) pieces.push(bytes.subarray(at, end));
      }
      at = end;
    }
    if (at !== bytes.length) throw new Error('A panel holds bytes past its records.');
    if (!patched) return buffer;
    const output = new Uint8Array(24 + pieces.reduce((total, piece) => total + piece.length, 0));
    output.set(bytes.subarray(0, 24));
    new DataView(output.buffer).setUint32(0, DATED_PANEL, true);
    let offset = 24;
    for (const piece of pieces) { output.set(piece, offset); offset += piece.length; }
    return output.buffer;
  }
}

export function restorePanelBatches(
  deliver: (payload: ArrayBuffer) => void,
  repair: (serial: number) => Promise<ArrayBuffer[]>,
) {
  const bodies = new PanelBodies();
  return async (payloads: ArrayBuffer[]) => {
    const first = payloads[0];
    if (!first) return;
    const marker = new DataView(first);
    if (first.byteLength !== 8 || marker.getUint32(0, true) !== BATCH) {
      for (const payload of payloads) deliver(bodies.decode(payload));
      return;
    }
    let restored: ArrayBuffer[];
    try {
      restored = payloads.slice(1).map(payload => bodies.decode(payload));
    } catch {
      const whole = await repair(marker.getUint32(4, true));
      if (whole.length !== payloads.length - 1) throw new Error('The full panel batch did not contain all of its packets.');
      restored = whole.map(payload => bodies.decode(payload));
    }
    for (const payload of restored) deliver(payload);
  };
}
