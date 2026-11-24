import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { FamilyBarsBinding } from './laneView.ts';
import { deriveBarsAudit, type BarsModel } from './model.ts';

export function parseBarsHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  if (hitKey.startsWith('add:')) return { key: '__add-region', subject: '__add-region', kind: 'add-region', toggle: false, inspectToken: hitKey };
  if (hitKey.startsWith('remove:')) {
    const key = hitKey.slice(7);
    return key ? { key, subject: key, kind: 'remove-region', toggle: true, inspectToken: hitKey } : null;
  }
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
  getRowStride: fields => (fields.meta as { readonly rowHeight?: number } | undefined)?.rowHeight ?? 28,
  getPageStep: (fields, viewport) => Math.max(1, Math.floor(viewport.height / ((fields.meta as { readonly rowHeight?: number } | undefined)?.rowHeight ?? 28))),
  deriveAuditRows: model => deriveBarsAudit(model),
  auditOrdered: true,
  buildBinding: ctx => {
    if (!ctx.theme) return null;
    return {
      options: ctx.options,
      sources: ctx.sources,
      width: ctx.viewport.width,
      height: ctx.viewport.height,
      scrollTop: ctx.viewport.scrollTop,
      theme: ctx.theme,
      selectedKey: ctx.selectedKey,
      focusedKey: ctx.focusedKey,
      reducedMotion: ctx.reducedMotion,
      audit: ctx.audit,
      ...(ctx.auditId !== undefined ? { auditId: ctx.auditId } : {}),
      pauseFlush: ctx.pauseFlush,
    };
  },
};
