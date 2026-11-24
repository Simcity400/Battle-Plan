import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { FamilyTimelineBinding, TimelineExtentField } from './laneView.ts';
import { deriveTimelineAudit, type TimelineModel } from './model.ts';
import { decodeTimelineHit } from './painter.ts';

export function parseTimelineHit(token: string | null): ParsedFamilyHit | null {
  const hit = decodeTimelineHit(token);
  if (!hit) return null;
  return {
    key: hit.laneKey,
    subject: hit.laneKey,
    ...(hit.hour !== undefined ? { hour: hit.hour } : {}),
    kind: hit.kind,
    toggle: hit.kind === 'toggle',
    inspectToken: token!,
  };
}

function extentOf(fields: Readonly<Record<string, unknown>>): TimelineExtentField | undefined {
  return fields.extent as TimelineExtentField | undefined;
}

export const timelineHostAdapter: FamilyHostAdapter<TimelineModel, unknown, FamilyTimelineBinding> = {
  family: 'timeline',
  navigation: 'rows',
  scroll: 'vertical',
  mirror: 'listbox',
  auditOwnerId: -4,
  defaultSize: [480, 240],
  parseHit: parseTimelineHit,
  inspectToken: key => `lane:${key}\u001f\u001f`,
  getScrollExtent: fields => {
    const extent = extentOf(fields);
    return extent ? { height: extent.height } : null;
  },
  deriveAuditRows: model => deriveTimelineAudit(model),
  auditOrdered: true,
  buildBinding: ctx => ({ expanded: ctx.expanded }),
};
