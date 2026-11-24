import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import { parseRegionHitKey } from '../regions/regionRows.ts';
import type { FamilyBarsBinding } from './laneView.ts';
import { deriveBarsAudit, type BarsModel } from './model.ts';

export function parseBarsHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  const region = parseRegionHitKey(hitKey);
  if (region) return { key: region.key, subject: region.key, kind: `region-${region.part}`, toggle: false, inspectToken: `row:${region.key}` };
  if (hitKey.startsWith('right:')) {
    const key = hitKey.slice(6);
    return key ? { key, subject: key, kind: 'right-row', toggle: false, inspectToken: hitKey } : null;
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
  inspectToken: key => `row:${key}`,
  getScrollExtent: fields => {
    const meta = fields.meta as { readonly totalHeight?: number } | undefined;
    return meta?.totalHeight !== undefined ? { height: meta.totalHeight } : null;
  },
  deriveAuditRows: model => deriveBarsAudit(model),
  auditOrdered: true,
  buildBinding: ctx => {
    if (!ctx.theme) return null;
    return {
      options: ctx.options,
      sources: ctx.sources,
      ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
      ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
      width: ctx.viewport.width,
      height: ctx.viewport.height,
      scrollTop: ctx.viewport.scrollTop,
      theme: ctx.theme,
      ...(ctx.identitySnapshot ? { identitySnapshot: ctx.identitySnapshot } : {}),
      selectedKey: ctx.selectedKey,
      focusedKey: ctx.focusedKey,
      reducedMotion: ctx.reducedMotion,
      audit: ctx.audit,
      ...(ctx.auditId !== undefined ? { auditId: ctx.auditId } : {}),
      pauseFlush: ctx.pauseFlush,
    };
  },
};
