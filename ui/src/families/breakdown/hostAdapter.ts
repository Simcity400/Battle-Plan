import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { BreakdownViewOptions } from './declaration.ts';
import type { FamilyBreakdownBinding } from './laneView.ts';
import type { BreakdownModel } from './model.ts';
import { auditLayout, deriveBreakdownAudit } from './structure.ts';

export function parseBreakdownHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  if (hitKey.startsWith('toggle:')) {
    const key = hitKey.slice(7);
    return key ? { key, subject: key, kind: 'toggle', toggle: true, inspectToken: `row:${key}` } : null;
  }
  if (hitKey.startsWith('row:')) {
    const key = hitKey.slice(4);
    return key ? { key, subject: key, kind: 'row', inspectToken: hitKey } : null;
  }
  return null;
}

export const breakdownHostAdapter: FamilyHostAdapter<BreakdownModel, BreakdownViewOptions, FamilyBreakdownBinding> = {
  family: 'breakdown',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -4,
  parseHit: parseBreakdownHit,
  inspectToken: key => `row:${key}`,
  exclusiveExpansion: fields => (fields.meta as { readonly view?: string } | undefined)?.view === 'ring',
  getScrollExtent: fields => {
    const meta = fields.meta as { readonly contentHeight?: number } | undefined;
    return meta?.contentHeight !== undefined ? { height: meta.contentHeight } : null;
  },
  getRowStride: () => 23,
  getPageStep: (_fields, viewport) => Math.max(1, Math.floor(viewport.height / 23)),
  deriveAuditRows: (model, _hour, context) => deriveBreakdownAudit(model, auditLayout(model, context.viewport.width, context.viewport.height, context.expanded, context.theme)),
  auditOrdered: true,
  buildBinding: ctx => {
    if (!ctx.theme) return null;
    return {
      family: 'breakdown',
      kind: ctx.kind,
      options: ctx.options,
      sources: ctx.sources,
      ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
      ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
      width: ctx.viewport.width,
      height: ctx.viewport.height,
      scrollTop: ctx.viewport.scrollTop,
      theme: ctx.theme,
      ...(ctx.identitySnapshot ? { identitySnapshot: ctx.identitySnapshot } : {}),
      expanded: ctx.expanded,
      selectedKey: ctx.selectedKey,
      focusedKey: ctx.focusedKey,
      ...(ctx.audit ? { audit: true, auditId: ctx.auditId } : {}),
    };
  },
};
