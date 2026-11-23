import type { LiveValues } from '../../charts/live';

export interface RowCell {
  key: string; row: string; text: string; x: number; y: number; width: number; height: number;
  meter?: 'horizontal' | 'vertical'; knob?: boolean; radius?: number;
  baseline: number; font: string; color: string; align: 'left' | 'right' | 'center';
}
export interface PanelModel {
  widget?: string;
  view: string; follow?: number; fixed?: number; offset: number; date?: { since: number; through: number };
  structure: string; cells: RowCell[];
  reply?: unknown; clocked?: boolean;
}
export interface PanelGeometry {
  rowAttribute?: string;
  layers?: string;
  extraFields?: string;
  meters?: string;
  knob?: (field: HTMLElement) => boolean;
  extendInline?: string;
  barVariable?: { host: string; name: string };
}
export interface PanelView {
  geometry?: PanelGeometry;
  nextChange?(reply: unknown, hour: number): number;
  structure?(reply: unknown, hour: number): string;
  values(reply: unknown, live: LiveValues<number> | null, hour: number, structure: string): ReadonlyMap<string, string> | null;
}
