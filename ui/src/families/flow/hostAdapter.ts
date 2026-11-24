import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { FamilyFlowBinding } from './laneView.ts';
import { deriveFlowAudit, type FlowModel } from './model.ts';

const ROW_STRIDE_PX = 20;

export function parseFlowHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey || !(hitKey.startsWith('node:') || hitKey.startsWith('ribbon:'))) return null;
  const subject = hitKey.slice(hitKey.indexOf(':') + 1);
  return subject ? { key: hitKey, subject, inspectToken: hitKey } : null;
}

export const flowHostAdapter: FamilyHostAdapter<FlowModel, unknown, FamilyFlowBinding> = {
  family: 'flow',
  scroll: 'none',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -8,
  parseHit: parseFlowHit,
  inspectToken: key => key,
  getRowStride: () => ROW_STRIDE_PX,
  getPageStep: (_fields, viewport) => Math.max(1, Math.floor(viewport.height / ROW_STRIDE_PX) - 1),
  deriveAuditRows: model => deriveFlowAudit(model),
  auditOrdered: true,
  buildBinding: ctx => {
    if (!ctx.theme) return null;
    return {
      options: ctx.options,
      sources: ctx.sources,
      ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
      ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
      ...(ctx.artResolutions ? { artResolutions: ctx.artResolutions } : {}),
      width: ctx.viewport.width,
      height: ctx.viewport.height,
      scrollTop: 0,
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
