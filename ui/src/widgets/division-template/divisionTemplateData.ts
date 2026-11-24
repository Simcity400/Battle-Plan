import type { DivisionTemplatesReply, TemplateEntry } from '../../generated/dashboard';

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

const compositionKnown = (entry: TemplateEntry) => entry.unitsMissing === null || entry.units.length > 0;

export const battalionCount = (entry: TemplateEntry): number | null =>
  compositionKnown(entry) ? entry.units.reduce((sum, unit) => unit.support ? sum : sum + unit.count, 0) : null;

export const priorityLabel = (entry: TemplateEntry): string | null => entry.priorityName;
