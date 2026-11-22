import type { PictureInterval } from './pictureDate';

export type AuditRow = { row: string; key: string; value?: string; overflow?: number };
export interface PictureSample {
  t: 'picture'; at: number; shownAt: number; seek: number; widget: string; surface: string; placement: number; owner: number;
  renderer: 'worker'; head: number; hour: number | null; offset: number; fixedHour?: number; rows?: readonly AuditRow[]; detached?: boolean;
  since?: number; through?: number; eligibleHour: number | null; inputThrough?: number; receivedThrough?: number;
  drawMs: number;
}
export interface FrameSample {
  t: 'workerFrame'; at: number; frameAt: number; shownAt: number; head: number; seek: number; playing: boolean; workMs: number; changed: number; rate: number; lane: number;
  applyMs: number; paintMs: number;
}
export type DrawSample = PictureSample | FrameSample;

export interface Present {
  seek: number; generation: number;
  rows?: readonly { row: string; key: string; value?: string }[]; id: number; owner: number; date?: PictureInterval; follow?: number; readHour?: number; holdsThrough?: number; head?: number; cellHour?: number; at?: number; shown?: number;
}
export type Notice = { type: 'present'; surfaces: Present[] } | { type: 'hit'; id: number; owner: number; key: string | null };
