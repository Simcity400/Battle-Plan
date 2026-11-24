import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { FamilyMeterBinding } from './laneView.ts';
import { deriveMeterAudit, type MeterModel } from './model.ts';

const ROW_STRIDE_PX = 32;

export function parseMeterHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey?.startsWith('row:')) return null;
  const key = hitKey.slice(4);
  return key ? { key, subject: key, inspectToken: hitKey } : null;
}

export const meterHostAdapter: FamilyHostAdapter<MeterModel, unknown, FamilyMeterBinding> = {
  family: 'meters',
  skeleton: 'table',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -6,
  parseHit: parseMeterHit,
  inspectToken: key => `row:${key}`,
  getScrollExtent: fields => {
    const extent = fields.extent as { readonly height?: number } | undefined;
    return extent?.height !== undefined ? { height: extent.height } : null;
  },
  getRowStride: () => ROW_STRIDE_PX,
  getPageStep: (_fields, viewport) => Math.max(1, Math.floor(viewport.height / ROW_STRIDE_PX) - 1),
  deriveAuditRows: model => deriveMeterAudit(model),
  auditOrdered: true,
  buildBinding: ctx => {
    if (!ctx.theme) return null;
    return {
      family: 'meters',
      kind: ctx.kind,
      view: ctx.view,
      options: ctx.options,
      sources: ctx.sources,
      ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
      ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
      ...(ctx.artResolutions ? { artResolutions: ctx.artResolutions } : {}),
      width: ctx.viewport.width,
      height: ctx.viewport.height,
      scrollTop: ctx.viewport.scrollTop,
      theme: ctx.theme,
      ...(ctx.identitySnapshot ? { identitySnapshot: ctx.identitySnapshot } : {}),
      selectedKey: ctx.selectedKey,
      focusedKey: ctx.focusedKey,
      audit: ctx.audit,
      ...(ctx.auditId !== undefined ? { auditId: ctx.auditId } : {}),
      pauseFlush: ctx.pauseFlush,
    };
  },
};
