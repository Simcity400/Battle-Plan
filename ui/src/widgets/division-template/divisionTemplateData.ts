import { formatNativeFixed5 } from '../../charts/valueAxis.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { usNumber } from '../../charts/numberFormat.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { DivisionTemplatesReply, TemplateEntry, TemplateUnit } from '../../generated/dashboard';

export type TemplateRow =
  | { kind: 'template'; key: string; entry: TemplateEntry }
  | { kind: 'obsolete'; key: string; count: number; children: TemplateRow[] }
  | { kind: 'status-unknown'; key: string; count: number; reason: string; children: TemplateRow[] };

const isArray = Array.isArray;
export function validateDivisionTemplates(value: unknown): DivisionTemplatesReply {
  const reply = value as DivisionTemplatesReply;
  if (!reply || typeof reply.tag !== 'string' || (reply.countFor !== 'owner' && reply.countFor !== 'host')
    || !isArray(reply.templates) || !isArray(reply.warnings)
    || reply.templates.some(entry => typeof entry.template !== 'string' || !isArray(entry.units)
      || (entry.fielded !== null && !Number.isInteger(entry.fielded))))
    throw new Error('Invalid division templates reply');
  return reply;
}

export const templateStructure = (reply: DivisionTemplatesReply) => JSON.stringify([reply.templates, reply.fieldedMissing, reply.warnings]);

export function templateRows(reply: DivisionTemplatesReply): TemplateRow[] {
  const current: TemplateRow[] = [], obsolete: TemplateRow[] = [], unknown: TemplateRow[] = [];
  let unknownReason: string | null = null;
  for (const entry of reply.templates) {
    const row: TemplateRow = { kind: 'template', key: entry.template, entry };
    if (entry.obsolete === true) obsolete.push(row);
    else if (entry.obsolete === null) {
      unknown.push(row);
      unknownReason ??= entry.settingsMissing ?? 'Template status was not recorded.';
    } else current.push(row);
  }
  const out: TemplateRow[] = [...current];
  if (unknown.length) {
    out.push({ kind: 'status-unknown', key: 'status-unknown', count: unknown.length, reason: unknownReason ?? 'Template status was not recorded.', children: unknown });
  }
  if (obsolete.length) {
    out.push({ kind: 'obsolete', key: 'obsolete', count: obsolete.length, children: obsolete });
  }
  return out;
}

export const templateName = (entry: TemplateEntry) => entry.name ?? 'Template name unavailable';
export const unitName = (unit: TemplateUnit) => unit.name ?? 'Unit name unavailable';

const compositionKnown = (entry: TemplateEntry) => entry.unitsMissing === null || entry.units.length > 0;

export function battalionSummary(entry: TemplateEntry): string | null {
  if (!compositionKnown(entry)) return null;
  const line = entry.units.filter(unit => !unit.support).map(unit => `${unitName(unit)} ×${unit.count}`);
  const support = entry.units.filter(unit => unit.support).map(unit => unit.count === 1 ? unitName(unit) : `${unitName(unit)} ×${unit.count}`);
  if (!line.length && !support.length) return 'Empty';
  return [line.join(', '), support.join(', ')].filter(Boolean).join(' · ');
}

export const battalionCount = (entry: TemplateEntry): number | null =>
  compositionKnown(entry) ? entry.units.reduce((sum, unit) => unit.support ? sum : sum + unit.count, 0) : null;

export const priorityLabel = (entry: TemplateEntry): string | null => entry.priorityName;

export function baseWidth(entry: TemplateEntry): string | null {
  return entry.baseWidthRaw === null ? null : formatNativeFixed5(entry.baseWidthRaw, 1);
}

const yesNo = (value: boolean | null) => value === null ? 'Not recorded' : value ? 'Yes' : 'No';

export function templateTooltip(entry: TemplateEntry, hour: number, canOpen: boolean): TooltipContent {
  const rows: TooltipRow[] = [
    { label: 'Fielded', value: entry.fielded === null ? 'Not recorded' : usNumber.format(entry.fielded) },
    { label: 'Base combat width', value: baseWidth(entry) ?? 'Not recorded' },
  ];
  const line = entry.units.filter(unit => !unit.support), support = entry.units.filter(unit => unit.support);
  if (!compositionKnown(entry)) rows.push({ label: 'Battalions', value: 'Not recorded' });
  if (line.length) rows.push({ label: 'Battalions', strong: true, divider: true });
  for (const unit of line) rows.push({ label: unitName(unit), lookupKey: unit.key ?? undefined, value: `×${unit.count}`, indent: true });
  if (support.length) rows.push({ label: 'Support companies', strong: true });
  for (const unit of support) rows.push({ label: unitName(unit), lookupKey: unit.key ?? undefined, value: unit.count === 1 ? undefined : `×${unit.count}`, indent: true });
  rows.push({ label: 'Priority', value: priorityLabel(entry) ?? 'Not recorded' });
  if (entry.settingsMissing === null) {
    rows.push({ label: 'Locked', value: yesNo(entry.locked), divider: true },
      { label: 'Recruits regardless of manpower', value: yesNo(entry.forceRecruiting) },
      { label: 'Accepts new equipment', value: yesNo(entry.newEquipment) },
      { label: 'Accepts foreign equipment', value: yesNo(entry.foreignEquipment) });
    if (entry.manpower !== null) rows.push({ label: 'Manpower', value: usNumber.format(entry.manpower) });
    if (entry.obsoleteHour !== null) rows.push({ label: 'Obsolete since', value: formatDateTime(entry.obsoleteHour) });
  } else rows.push({ label: 'Settings', value: 'Not recorded', divider: true });
  return { title: templateName(entry), subtitle: `As of ${formatDateTime(hour)}`, rows, note: canOpen ? undefined : 'Game view not available yet' };
}
