import type { InstalledAction } from '../contract.ts';
import type { FamilyHostAdapter, FamilyInteraction, ParsedFamilyHit } from '../host/adapter.ts';
import type { BreakdownViewOptions } from './declaration.ts';
import { NO_INTERACTION, type BreakdownInteraction, type FamilyBreakdownBinding } from './laneView.ts';
import { deriveBreakdownAudit, type BreakdownActionMeta, type BreakdownModel } from './model.ts';

export const BACK = '__back__';
export const PARTIAL = '__partial__';

export function parseBreakdownHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  if (hitKey.startsWith('back:')) return { key: BACK, subject: BACK, kind: 'back', inspectToken: hitKey };
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

function actionMeta(action: InstalledAction): BreakdownActionMeta | null {
  const meta = action.meta as Partial<BreakdownActionMeta> | null | undefined;
  return meta && typeof meta.act === 'string' ? (meta as BreakdownActionMeta) : null;
}

export function reduceBreakdown(state: BreakdownInteraction, action: InstalledAction): BreakdownInteraction | null {
  const key = action.subject;
  if (key === BACK) return state.drill.length ? { ...state, drill: state.drill.slice(0, -1) } : state;
  if (key === PARTIAL) return state;
  const meta = actionMeta(action);
  if (!meta) return null;
  switch (meta.act) {
    case 'toggle':
      return { ...state, expanded: { ...state.expanded, [key]: !meta.open } };
    case 'ring':
      return { ...state, ringOpen: state.ringOpen === key ? '' : key };
    case 'drill':
      return { ...state, drill: [...state.drill, key] };
  }
}

const interaction: FamilyInteraction<BreakdownInteraction> = { initial: NO_INTERACTION, reduce: reduceBreakdown };

export const breakdownHostAdapter: FamilyHostAdapter<BreakdownModel, BreakdownViewOptions, FamilyBreakdownBinding> = {
  family: 'breakdown',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -4,
  interaction,
  parseHit: parseBreakdownHit,
  inspectToken: key => `row:${key}`,
  getScrollExtent: fields => {
    const meta = fields.meta as { readonly contentHeight?: number } | undefined;
    return meta?.contentHeight !== undefined ? { height: meta.contentHeight } : null;
  },
  getRowStride: () => 23,
  getPageStep: (_fields, viewport) => Math.max(1, Math.floor(viewport.height / 23)),
  deriveAuditRows: model => deriveBreakdownAudit(model),
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
      interaction: (ctx.interaction as BreakdownInteraction | undefined) ?? NO_INTERACTION,
      selectedKey: ctx.selectedKey,
      focusedKey: ctx.focusedKey,
      ...(ctx.audit ? { audit: true, auditId: ctx.auditId } : {}),
    };
  },
};
