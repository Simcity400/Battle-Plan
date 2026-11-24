import type { LiveValues } from '../../charts/live';

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
