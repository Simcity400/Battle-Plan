import type { LedgerGovernment, PoliticsLedgerReply } from '../../generated/dashboard';
import type { EventBadge } from '../../charts/badges';
import type { TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { object, opaqueTag, signed32, unsigned32, unsigned64 } from '../../dashboard/validation.ts';

const optionalText = (value: unknown) => value == null || typeof value === 'string';

/** Government-only projection of the shared native power ledger scan. */
export function validateGovernmentLedger(data: unknown, expected?: Pick<PoliticsLedgerReply, 'branchId' | 'tag'>): PoliticsLedgerReply {
  const reply = data as PoliticsLedgerReply;
  if (!object(reply) || !unsigned64(reply.branchId) || !opaqueTag(reply.tag)
    || (expected && (reply.branchId !== expected.branchId || reply.tag !== expected.tag))
    || !Array.isArray(reply.warnings) || !reply.warnings.every(value => typeof value === 'string')
    || (reply.governments !== undefined && (!Array.isArray(reply.governments) || !reply.governments.every(row => object(row)
      && signed32(row.hour) && signed32(row.readHour) && row.readHour >= row.hour
      && unsigned32(row.country) && unsigned32(row.instance) && unsigned64(row.epoch) && unsigned64(row.sequence)
      && typeof row.cause === 'string' && optionalText(row.beforeToken) && optionalText(row.afterToken) && typeof row.covered === 'boolean')))) {
    throw new Error('The government history reply is malformed.');
  }
  const marks = reply.governmentMarks;
  if (marks != null && (!object(marks) || !Number.isFinite(marks.from) || !Number.isFinite(marks.to) || marks.from > marks.to
    || !unsigned32(marks.width) || marks.width === 0 || !Array.isArray(marks.clusters) || marks.clusters.length > Math.ceil(marks.width / 18)
    || !marks.clusters.every(cluster => object(cluster) && signed32(cluster.firstHour) && signed32(cluster.lastHour)
      && cluster.firstHour <= cluster.lastHour && unsigned64(cluster.firstOrdinal) && unsigned64(cluster.count) && cluster.count !== '0'))) {
    throw new Error('The government marks are malformed.');
  }
  return reply;
}

/** Neither a census change nor an election date supplies an execution cause. */
export function governmentCause(cause: string): string {
  switch (cause) {
    case 'ElectionOnAction': return 'Election';
    case 'CoupCompletion': return 'Coup';
    case 'CivilWar': return 'Civil war';
    case 'SetPoliticsEffect': return 'Government change';
    case 'PeaceForceGovernment': return 'Peace settlement';
    case 'SubjectGovernment': return 'Subject government';
    case 'Console': return 'Console';
    case 'ResetSetup': return 'Setup';
    case 'NotIdentified': return 'Unknown cause';
    default: return 'Unknown cause';
  }
}

export function visibleGovernmentChanges(rows: readonly LedgerGovernment[], through: number): LedgerGovernment[] {
  return rows.filter(row => row.hour <= through && row.readHour <= through);
}

export function governmentBadges(rows: readonly LedgerGovernment[], name: (token: string) => string, through: number): EventBadge[] {
  return visibleGovernmentChanges(rows, through).map(row => {
    const change = row.beforeToken && row.afterToken && row.beforeToken !== row.afterToken
      ? `${name(row.afterToken)} took power from ${name(row.beforeToken)}` : 'Government updated';
    return { hour: row.hour, glyph: '\u21c6', label: `${change} / ${governmentCause(row.cause)}${row.covered ? '' : ' / Some changes may be missing'}` };
  });
}

export function governmentRows(rows: readonly LedgerGovernment[], hour: number, through: number, name: (token: string) => string): TooltipRow[] {
  const day = Math.floor(hour / 24);
  return visibleGovernmentChanges(rows, Math.min(hour, through)).filter(row => Math.floor(row.hour / 24) === day).flatMap(row => governmentEventRows(row, name));
}

/** A reached event page has already proven the supplying read independently of its native date. */
export function governmentEventRows(row: LedgerGovernment, name: (token: string) => string): TooltipRow[] {
  return [
    { label: `Government / ${formatDateTime(row.hour)}`, value: governmentCause(row.cause), strong: true, divider: true },
    { label: 'Before / after', value: `${row.beforeToken ? name(row.beforeToken) : 'Not identified'} / ${row.afterToken ? name(row.afterToken) : 'Not identified'}` },
    ...(row.covered ? [] : [{ label: 'Some changes may be missing.', tone: 'muted' as const }]),
  ];
}
