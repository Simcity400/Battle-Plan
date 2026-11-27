import { displayedHour } from '../boundedView.ts';
import type { FamilyHostAdapter } from '../host/adapter.ts';
import type { FamilyCardBinding } from './laneView.ts';
import { deriveCardAudit, type CardModel } from './model.ts';

export const cardHostAdapter: FamilyHostAdapter<CardModel, unknown, FamilyCardBinding> = {
  family: 'card',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'listbox',
  auditOwnerId: -7,
  resolveHitAction: (hit, installed, fields) => {
    const target = installed.actionKeys.get(hit.key);
    const hours = fields.rowHours as Readonly<Record<string, number>> | undefined;
    const own = hours && Object.hasOwn(hours, hit.key) ? hours[hit.key] : undefined;
    return {
      installedId: installed.id,
      owner: installed.owner,
      target: { kind: 'row', key: hit.key },
      subject: hit.subject ?? target?.subject ?? hit.key,
      hour: own === undefined ? installed.hour : displayedHour(own),
      ...(target?.label !== undefined ? { label: target.label } : {}),
      ...(target?.meta !== undefined ? { meta: target.meta } : {}),
    };
  },
  getScrollExtent: fields => {
    const meta = fields.meta as { readonly totalHeight?: number } | undefined;
    return meta?.totalHeight !== undefined ? { height: meta.totalHeight } : null;
  },
  deriveAuditRows: model => deriveCardAudit(model),
  auditOrdered: true,
};
