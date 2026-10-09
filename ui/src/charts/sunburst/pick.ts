import type { SunburstNode } from './layout';

/** Activation uses the tree and callback that supplied the drawn slice. Content
 * and playback may update the callback without relabelling that old geometry. */
export interface SunburstPick {
  owner: unknown;
  nodes: readonly SunburstNode[];
  onPick?: (node: SunburstNode) => boolean;
  canPick?: (node: SunburstNode) => boolean;
}
export function pickSunburst(frame: SunburstPick | null, owner: unknown, active: boolean, id: string): boolean {
  if (!active || !frame || !Object.is(frame.owner, owner) || !frame.onPick) return false;
  const node = frame.nodes.find(node => node.id === id);
  return !!node && frame.canPick?.(node) !== false && frame.onPick(node);
}
