import {
  DATED_FOLLOW,
  DATED_PANEL,
  DATED_PANEL_VERSION,
  parseAnswer,
  parseDatedPanelRecord,
  wireKind,
  type DatedPanel,
  type FollowedAnswer,
} from '../dashboard/follow/message.ts';

const RESET = 1;
const aligned = (at: number) => (at + 7) & ~7;

export interface FollowedMessage {
  limited?: Map<number, number>;
  route: number;
  hour: number;
  reset: boolean;
  seek: number;
  clock: number;
  answers: { id: number; hour: number; answer: FollowedAnswer; span?: [number, number] }[];
}

export function parseFollowed(bytes: ArrayBuffer | Uint8Array): FollowedMessage {
  const view = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  const data = new DataView(view.buffer, view.byteOffset, view.byteLength);
  const byteLength = view.byteLength;
  if (byteLength < 24 || data.getUint32(0, true) !== DATED_FOLLOW) throw new Error('Not a followed message.');
  const flags = data.getUint32(4, true);
  const message: FollowedMessage = {
    route: flags >>> 1,
    hour: data.getInt32(8, true),
    reset: (flags & RESET) !== 0,
    seek: data.getUint32(16, true),
    clock: data.getInt32(20, true),
    answers: [],
  };
  const count = data.getUint32(12, true);
  let at = 24;
  for (let i = 0; i < count; i++) {
    if (at + 16 > byteLength) throw new Error('A followed message ends early.');
    const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), length = data.getUint32(at + 8, true);
    const start = at + 16, end = start + length;
    if (end > byteLength) throw new Error('A followed message ends early.');
    if (!length) (message.limited ??= new Map()).set(id, hour);
    else message.answers.push({ id, hour, answer: parseAnswer(view.subarray(start, end)), span: [start, end] });
    at = end;
  }
  if (at !== byteLength) throw new Error('A followed message holds bytes past its answers.');
  return message;
}

export function parseDatedPanels(bytes: Uint8Array) {
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (data.byteLength < 24 || wireKind(bytes) !== DATED_PANEL || data.getUint16(4, true) !== DATED_PANEL_VERSION) {
    throw new Error('Invalid dated panel header.');
  }
  const count = data.getUint32(8, true), clock = data.getInt32(12, true), route = data.getUint32(16, true), seek = data.getUint32(20, true);
  const panels: DatedPanel[] = [];
  let completed: number | undefined;
  let at = 24;
  for (let i = 0; i < count; i++) {
    const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), kind = data.getUint8(at + 8);
    const length = data.getUint32(at + 12, true), end = at + 16 + length;
    if (end > bytes.length || ![0, 1, 3, 4, 5, 6].includes(kind) || (kind >= 3 && length)) throw new Error('Invalid dated panel record.');
    if (kind === 4) {
      if (id !== 0 || i !== count - 1) throw new Error('Invalid completed product step.');
      completed = hour;
      at = aligned(end);
      continue;
    }
    panels.push(parseDatedPanelRecord(bytes.subarray(at, end)));
    at = aligned(end);
  }
  if (at !== bytes.length) throw new Error('Unexpected bytes after dated panels.');
  return { route, clock, seek, panels, completed };
}
