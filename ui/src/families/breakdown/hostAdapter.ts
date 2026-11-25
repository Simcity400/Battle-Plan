import { fieldConsumer } from '../host/adapter.ts';
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
  exclusiveExpansion: fields => (fields.meta as { readonly view?: string } | undefined)?.view === 'ring',
  getScrollExtent: fieldConsumer(['meta'], fields => {
    const meta = fields.meta as { readonly contentHeight?: number } | undefined;
    return meta?.contentHeight !== undefined ? { height: meta.contentHeight } : null;
  }),
  deriveAuditRows: (model, _hour, context) => deriveBreakdownAudit(model, auditLayout(model, context.viewport.width, context.viewport.height, context.expanded, context.theme)),
  auditOrdered: true,
  buildBinding: ctx => ({ expanded: ctx.expanded }),
};
