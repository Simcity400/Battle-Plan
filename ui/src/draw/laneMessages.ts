import type { Channel } from '../dashboard/follow/facts';
import type { FrameRequest } from '../dashboard/follow/page';
import type { PayloadsMessage } from '../dashboard/follow/payloads';
import type { Binding, Presented, ScrubSample, Supply } from './lane';
import type { StripFrame } from '../dashboard/strip/scrub.ts';
import type { StripReceipt, StripUpdate } from '../dashboard/strip/store.ts';

export type ProjectionKey = readonly [family: string, kind: string, view: string];

export interface SharedSurfaceSlot {
  readonly widget: number;
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
  readonly visible: boolean;
}

export interface LaneSlotsPayload {
  readonly width: number;
  readonly height: number;
  readonly dpr: number;
  readonly scrollTop: number;
  readonly scrollLeft: number;
  readonly surfaceColor: string;
  readonly clipPathFallback: boolean;
  readonly slots: readonly SharedSurfaceSlot[];
}

export type ToLane =
  | { type: 'init'; lane: number; lanes: number; port: MessagePort; rate: number }
  | { type: 'options'; rate: number }
  | { type: 'prepare'; ask: number; families: readonly string[]; projections: readonly ProjectionKey[] }
  | { type: 'follow'; sub: number; channel: Channel; product?: string; route?: { id: number; seek: number; offset: number } }
  | { type: 'route'; sub: number; route: number; native: number; seek: number; offset: number }
  | { type: 'unfollow'; sub: number }
  | { type: 'mount'; widget: number; subs: number[]; view: string; model?: unknown }
  | { type: 'unmount'; widget: number }
  | { type: 'strip'; widget: number; build: number; receipt: number; update: StripUpdate }
  | { type: 'scrub'; page: number; frame: StripFrame | null }
  | { type: 'bind'; widget: number; binding: Binding; model?: unknown; retry?: boolean; remounted?: boolean }
  | ({ type: 'supply'; widget: number } & Supply)
  | PayloadsMessage
  | { type: 'exact'; sub: number; hour: number; data: unknown; refused: string | null }
  | { type: 'stream'; route: number; seek: number; target: number }
  | { type: 'fail'; route: number; seek: number; command: number; reason: string }
  | { type: 'pointer'; widget: number; surface: number; point: [number, number] | null; installed?: number }
  | { type: 'canvas'; widget: number; canvas: OffscreenCanvas; opaque: boolean }
  | { type: 'lane-surface'; canvas: OffscreenCanvas; opaque: boolean }
  | ({ type: 'lane-slots' } & LaneSlotsPayload)
  | FrameRequest;

export type FromLane =
  | ({ type: 'presentation' } & Presented<ImageBitmap>)
  | { type: 'failure'; lane: number; reason: string }
  | { type: 'prepared'; lane: number; ask: number }
  | { type: 'route-bound'; sub: number; route: number; seek: number }
  | { type: 'strip'; receipt: number; held: StripReceipt } | { type: 'strip'; receipt: number; error: string }
  | { type: 'scrubbed'; frame: StripFrame; samples: readonly ScrubSample[] }
  | { type: 'sub-failed'; lane: number; sub: number; reason: string }
  | { type: 'sub-waiting'; lane: number; sub: number; waiting: boolean }
  | { type: 'widget-failed'; lane: number; widget: number; reason: string; stamp?: number }
  | { type: 'hit'; widget: number; key: string | null; installedId?: number; owner?: string }
  | { type: 'served'; seek: number; target: number; exact: boolean; changed: number; ms: number; cancelled?: boolean; costs?: [widget: number, cost: number][] };

export type FromPull = { kind: 'follow'; bytes: Uint8Array; resets: number } | { kind: 'panels'; bytes: Uint8Array } | { kind: 'failure'; reason: string };

export type ToPull = { release: number };
