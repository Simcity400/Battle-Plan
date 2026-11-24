import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { FamilyFlowBinding } from './laneView.ts';
import { deriveFlowAudit, type FlowModel } from './model.ts';

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
  deriveAuditRows: model => deriveFlowAudit(model),
  auditOrdered: true,
};
