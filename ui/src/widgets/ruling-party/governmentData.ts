import type { EventBadge } from '../../charts/badges';
import type { TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { LedgerGovernment } from './partyTypes';

const NO_GOVERNMENTS: readonly LedgerGovernment[] = [];

export function governmentChanges(): readonly LedgerGovernment[] {
  return NO_GOVERNMENTS;
}

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

export function governmentEventRows(row: LedgerGovernment, name: (token: string) => string): TooltipRow[] {
  return [
    { label: `Government / ${formatDateTime(row.hour)}`, value: governmentCause(row.cause), strong: true, divider: true },
    { label: 'Before / after', value: `${row.beforeToken ? name(row.beforeToken) : 'Not identified'} / ${row.afterToken ? name(row.afterToken) : 'Not identified'}` },
    ...(row.covered ? [] : [{ label: 'Some changes may be missing.', tone: 'muted' as const }]),
  ];
}
