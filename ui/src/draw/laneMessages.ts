import type { Channel } from '../dashboard/follow/facts';
import type { FrameRequest } from '../dashboard/follow/page';
import type { PayloadsMessage } from '../dashboard/follow/payloads';
import type { Binding, Presented, Supply } from './lane';

export type ToLane =
  | { type: 'init'; lane: number; lanes: number; port: MessagePort; rate: number }
  | { type: 'options'; rate: number }
  | { type: 'follow'; sub: number; channel: Channel; product?: string }
  | { type: 'route'; sub: number; route: number; native: number; seek: number; offset: number }
  | { type: 'unfollow'; sub: number }
  | { type: 'mount'; widget: number; subs: number[]; view: string; model?: unknown }
  | { type: 'unmount'; widget: number }
  | { type: 'bind'; widget: number; binding: Binding; model?: unknown; retry?: boolean; remounted?: boolean }
  | ({ type: 'supply'; widget: number } & Supply)
  | PayloadsMessage
  | { type: 'exact'; sub: number; hour: number; data: unknown; refused: string | null }
  | { type: 'stream'; route: number; seek: number; target: number }
  | { type: 'fail'; route: number; seek: number; command: number; reason: string }
  | { type: 'pointer'; widget: number; surface: number; point: [number, number] | null; installed?: number }
  | FrameRequest;

export type FromLane =
  | ({ type: 'presentation' } & Presented<ImageBitmap>)
  | { type: 'failure'; lane: number; reason: string }
  | { type: 'sub-failed'; lane: number; sub: number; reason: string }
  | { type: 'widget-failed'; lane: number; widget: number; reason: string; stamp?: number }
  | { type: 'hit'; widget: number; key: string | null; installedId?: number; owner?: string }
  | { type: 'served'; seek: number; target: number; exact: boolean; changed: number; ms: number; cancelled?: boolean; costs?: [widget: number, cost: number][] };

export type FromPull = { kind: 'follow'; bytes: Uint8Array; resets: number } | { kind: 'panels'; bytes: Uint8Array } | { kind: 'failure'; reason: string };

export type ToPull = { release: number };
