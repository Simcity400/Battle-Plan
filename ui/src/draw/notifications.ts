export type AuditRow = { row: string; key: string; value?: string; overflow?: number };
export interface PictureSample {
  t: 'picture'; at: number; shownAt: number; seek: number; widget: string; surface: string; placement: number; owner: number; generation?: number;
  renderer: 'page'; head: number; hour: number | null; offset: number; fixedHour?: number; rows?: readonly AuditRow[]; auditId?: number; auditMissing?: boolean; detached?: boolean; face?: string;
  since?: number; through?: number; eligibleHour: number | null; inputThrough?: number; receivedThrough?: number; entry?: number; parts?: readonly string[];
  drawMs: number;
  mode: string; reason?: string; dim: boolean; reset: number; binding: number; inspect: string | null; tooltip: boolean;
}
export interface FrameSample {
  t: 'workerFrame'; at: number; lane: number; lanes: number; seek: number; head: number; playing: boolean; changed: number; workMs: number; cancelled?: boolean;
}
export type DrawSample = PictureSample | FrameSample;
