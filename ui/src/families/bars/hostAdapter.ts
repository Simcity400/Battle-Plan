import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import { parseRegionHitKey } from '../regions/regionRows.ts';
import type { FamilyBarsBinding } from './laneView.ts';
import { deriveBarsAudit, type BarsModel } from './model.ts';
import { pickAudit } from './pickList.ts';

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
  getScrollExtent: fields => {
    const meta = fields.meta as { readonly totalHeight?: number } | undefined;
    return meta?.totalHeight !== undefined ? { height: meta.totalHeight } : null;
  },
  deriveAuditRows: model => [...(model.pickList ? pickAudit(model.pickList) : []), ...deriveBarsAudit(model)],
  auditOrdered: true,
  buildBinding: ctx => ({ reducedMotion: ctx.reducedMotion }),
};
