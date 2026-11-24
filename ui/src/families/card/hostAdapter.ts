import { displayedHour } from '../boundedView.ts';
import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { FamilyCardBinding } from './laneView.ts';
import { deriveCardAudit, type CardModel } from './model.ts';

export function parseCardHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey?.startsWith('row:')) return null;
  const key = hitKey.slice(4);
  return key ? { key, subject: key, kind: 'row', toggle: false, inspectToken: hitKey } : null;
}

export const cardHostAdapter: FamilyHostAdapter<CardModel, unknown, FamilyCardBinding> = {
  family: 'card',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -7,
  parseHit: parseCardHit,
  resolveHitAction: (hit, installed, fields) => {
    const target = installed.actionKeys.get(hit.key);
    const hours = fields.rowHours as Readonly<Record<string, number>> | undefined;
    const own = hours && Object.hasOwn(hours, hit.key) ? hours[hit.key] : undefined;
    return {
      installedId: installed.id,
      owner: installed.owner,
      subject: hit.subject ?? target?.subject ?? hit.key,
      hour: own === undefined ? installed.hour : displayedHour(own),
      ...(target?.label !== undefined ? { label: target.label } : {}),
      ...(target?.meta !== undefined ? { meta: target.meta } : {}),
    };
  },
  inspectToken: key => `row:${key}`,
  getScrollExtent: fields => {
    const meta = fields.meta as { readonly totalHeight?: number } | undefined;
    return meta?.totalHeight !== undefined ? { height: meta.totalHeight } : null;
  },
  getRowStride: () => 36,
  getPageStep: (_fields, viewport) => Math.max(1, Math.floor(viewport.height / 36)),
  deriveAuditRows: model => deriveCardAudit(model),
  auditOrdered: true,
  buildBinding: ctx => {
    if (!ctx.theme) return null;
    return {
      family: 'card',
      kind: ctx.kind,
      view: ctx.view,
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
      audit: ctx.audit,
      ...(ctx.auditId !== undefined ? { auditId: ctx.auditId } : {}),
      pauseFlush: ctx.pauseFlush,
    };
  },
};
