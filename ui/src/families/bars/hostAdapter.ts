import { fieldConsumer } from '../host/adapter.ts';
import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import { parseRegionHitKey } from '../regions/regionRows.ts';
import type { FamilyBarsBinding } from './laneView.ts';
import { deriveBarsAudit, type BarsModel } from './model.ts';
import { pickAudit, pickedKey } from './pickList.ts';

function installedAuditWindow(
  nav: readonly (readonly unknown[])[],
  meta: unknown,
  scrollTop: number,
): { readonly first: number; readonly count: number } | null {
  const viewport = (meta as { readonly rowViewport?: { readonly height?: unknown } } | undefined)?.rowViewport;
  if (typeof viewport?.height !== 'number') return null;
  const topEdge = Math.max(0, scrollTop);
  const bottomEdge = topEdge + viewport.height;
  let first = Infinity;
  let last = -1;
  for (const entry of nav) {
    const [key, , index, , , , top, height] = entry;
    if (typeof key !== 'string' || pickedKey(key) !== null || typeof index !== 'number' || typeof top !== 'number' || typeof height !== 'number') continue;
    if (top + height <= topEdge || top >= bottomEdge) continue;
    first = Math.min(first, index);
    last = Math.max(last, index);
  }
  return last < 0 ? { first: 0, count: 0 } : { first, count: last - first + 1 };
}

export function parseBarsHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  const region = parseRegionHitKey(hitKey);
  if (region) return { key: region.key, subject: region.key, kind: `region-${region.part}`, toggle: false, inspectToken: `row:${region.key}` };
  if (hitKey.startsWith('right:')) {
    const key = hitKey.slice(6);
    return key ? { key, subject: key, kind: 'right-row', toggle: false, inspectToken: hitKey } : null;
  }
  if (hitKey.startsWith('pick:')) {
    const key = hitKey.slice(5);
    return key ? { key: hitKey, subject: key, kind: 'pick', toggle: false, inspectToken: hitKey, meta: { pick: key } } : null;
  }
  if (hitKey.startsWith('row:')) {
    const key = hitKey.slice(4);
    return key ? { key, subject: key, kind: 'row', toggle: false, inspectToken: hitKey } : null;
  }
  return null;
}

export const barsHostAdapter: FamilyHostAdapter<BarsModel, unknown, FamilyBarsBinding> = {
  family: 'bars',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -2,
  parseHit: parseBarsHit,
  getScrollExtent: fieldConsumer(['meta'], fields => {
    const meta = fields.meta as { readonly scrollHeight?: number } | undefined;
    return meta?.scrollHeight !== undefined ? { height: meta.scrollHeight } : null;
  }),
  deriveAuditRows: (model, _hour, { viewport, fields }) => {
    const nav = Array.isArray(fields.nav) ? fields.nav as readonly (readonly unknown[])[] : [];
    const pickShown = nav.some(entry => pickedKey(String(entry[0])) !== null);
    const window = installedAuditWindow(nav, fields.meta, viewport.scrollTop);
    return [
      ...(model.pickList && pickShown ? pickAudit(model.pickList) : []),
      ...(window ? deriveBarsAudit(model, { window }) : []),
    ];
  },
  auditOrdered: true,
  buildBinding: ctx => ({ reducedMotion: ctx.reducedMotion }),
};
