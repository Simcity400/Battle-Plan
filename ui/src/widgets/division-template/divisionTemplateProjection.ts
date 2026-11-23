import { usNumber } from '../../charts/numberFormat.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { OFFERED } from '../../draw/tables/settings.ts';
import type { DivisionTemplatesReply } from '../../generated/dashboard';
import {
  declareTableProjection,
  type TableColumnSpec,
  type TableProjection,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { knownValue, notApplicable, notRecorded, valueFromNullable, valueFromRawBigInt, type Value } from '../../families/value.ts';
import {
  battalionCount,
  battalionSummary,
  priorityLabel,
  templateName,
  templateRows,
  unitName,
  validateDivisionTemplates,
  type TemplateRow,
} from './divisionTemplateData.ts';

export interface DivisionTemplateProjectionOptions {
  readonly countryName: string;
  readonly cellBars: boolean;
  readonly canOpenDesigner: boolean;
}

const validated = new WeakMap<object, DivisionTemplatesReply>();

function checkedTemplates(raw: unknown): DivisionTemplatesReply {
  if (!raw || typeof raw !== 'object') return validateDivisionTemplates(raw);
  let held = validated.get(raw);
  if (!held) {
    held = validateDivisionTemplates(raw);
    validated.set(raw, held);
  }
  return held;
}

export function divisionTemplateColumns(cellBars = false): TableColumnSpec[] {
  return [
    {
      key: 'template',
      label: 'Template',
      as: 'icon',
      offered: OFFERED.text,
      priority: 0,
      text: true,
      hideable: false,
      widest: ['Infantry Division'],
      hint: 'Sorted by the game’s roster order',
    },
    {
      key: 'fielded',
      label: 'Fielded',
      as: cellBars ? 'faint' : 'plain',
      tone: 'blue',
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      format: 'integer',
      widest: ['999'],
    },
    {
      key: 'battalions',
      label: 'Battalions',
      as: 'plain',
      offered: OFFERED.text,
      priority: 2,
      text: true,
      widest: ['Infantry ×9 · Artillery'],
    },
    {
      key: 'width',
      label: 'Base combat width',
      as: 'plain',
      offered: OFFERED.number,
      priority: 3,
      numeric: true,
      format: 'fixed5Width',
      widest: ['99.9'],
    },
    {
      key: 'priority',
      label: 'Priority',
      as: 'plain',
      offered: OFFERED.text,
      priority: 4,
      text: true,
      widest: ['Reserve'],
    },
  ];
}

const yesNoValue = (value: boolean | null): Value<string> =>
  value === null ? notRecorded('Setting not recorded') : knownValue(value ? 'Yes' : 'No');

export const divisionTemplateProjection: TableProjection<DivisionTemplateProjectionOptions> = registerTableProjection(
  declareTableProjection<unknown, TemplateRow, DivisionTemplateProjectionOptions>({
    kind: 'division-template',
    columns: (_raw, options) => {
      const specs = divisionTemplateColumns(options.cellBars);
      return [
        {
          ...specs[0]!,
          iconSlot: true,
          icon: row => (row.kind === 'template' ? row.entry.icon ?? undefined : undefined),
          value: row => knownValue(row.kind === 'obsolete' ? `Obsolete templates (${row.count})` : templateName(row.entry)),
          sortValue: row => row.kind === 'obsolete'
            ? knownValue(Number.MAX_SAFE_INTEGER)
            : valueFromNullable(row.entry.rosterOrdinal, { kind: 'notRecorded', reason: 'Roster order not recorded' }),
        },
        {
          ...specs[1]!,
          value: row => row.kind === 'obsolete'
            ? notApplicable('Summary row for obsolete templates')
            : valueFromNullable(row.entry.fielded, { kind: 'notRecorded', reason: 'Fielded count not recorded' }),
          bar: row => (row.kind === 'obsolete' ? null : undefined),
        },
        {
          ...specs[2]!,
          value: row => {
            if (row.kind === 'obsolete') return notApplicable('Summary row for obsolete templates');
            const summary = battalionSummary(row.entry);
            return valueFromNullable(summary, { kind: 'notRecorded', reason: row.entry.unitsMissing ?? 'Composition not recorded' });
          },
          sortValue: row => {
            if (row.kind === 'obsolete') return notApplicable('Summary row for obsolete templates');
            return valueFromNullable(battalionCount(row.entry), { kind: 'notRecorded', reason: row.entry.unitsMissing ?? 'Composition not recorded' });
          },
        },
        {
          ...specs[3]!,
          value: row => row.kind === 'obsolete'
            ? notApplicable('Summary row for obsolete templates')
            : valueFromRawBigInt(row.entry.baseWidthRaw, { kind: 'notRecorded', reason: 'Base combat width not recorded' }),
        },
        {
          ...specs[4]!,
          value: row => row.kind === 'obsolete'
            ? notApplicable('Summary row for obsolete templates')
            : valueFromNullable(priorityLabel(row.entry), { kind: 'notRecorded', reason: row.entry.priorityMissing ?? 'Priority not recorded' }),
          sortValue: row => row.kind === 'obsolete'
            ? notApplicable('Summary row for obsolete templates')
            : valueFromNullable(row.entry.priority, { kind: 'notRecorded', reason: row.entry.priorityMissing ?? 'Priority not recorded' }),
        },
      ];
    },
    rows: (raw, _live, _hour, options) => {
      const reply = checkedTemplates(raw);
      const rows = templateRows(reply);
      const current = reply.templates.filter(entry => !entry.obsolete).length;
      const summaryPill = `${current} ${current === 1 ? 'template' : 'templates'}`;
      if (!rows.length) {
        return {
          rows: [],
          empty: `${options.countryName} has no division templates at this date.`,
          emptyKind: 'empty',
          warnings: reply.warnings,
          summaryPill,
        };
      }
      return { rows, warnings: reply.warnings, summaryPill };
    },
    rowKey: row => row.key,
    rowName: row => (row.kind === 'obsolete' ? `Obsolete templates (${row.count})` : templateName(row.entry)),
    rowMeta: (row, raw) => row.kind === 'template' ? { designer: row.entry.designer ?? 'ordinary', tag: checkedTemplates(raw).tag } : null,
    children: row => (row.kind === 'obsolete' ? row.children : undefined),
    initiallyOpen: () => false,
    tooltipAction: (row, options) => (row.kind === 'template' && options.canOpenDesigner ? 'Click for division designer' : undefined),
    tooltipNote: (row, _raw, _live, _hour, options) => (row.kind === 'template' && !options.canOpenDesigner ? 'Game view not available yet' : undefined),
    tooltipExtra: (row): TooltipExtraRow[] | undefined => {
      if (row.kind === 'obsolete') return undefined;
      const entry = row.entry;
      const extra: TooltipExtraRow[] = [];
      const line = entry.units.filter(unit => !unit.support);
      const support = entry.units.filter(unit => unit.support);
      if (line.length) {
        extra.push({ label: 'Line battalions', value: knownValue(String(line.reduce((s, u) => s + u.count, 0))), strong: true, divider: true });
        for (const unit of line) {
          extra.push({ label: unitName(unit), value: knownValue(`\u00d7${unit.count}`), indent: true, lookupKey: unit.key ?? undefined });
        }
      }
      if (support.length) {
        extra.push({ label: 'Support companies', value: knownValue(String(support.length)), strong: true });
        for (const unit of support) {
          extra.push({ label: unitName(unit), value: knownValue(unit.count === 1 ? '1' : `\u00d7${unit.count}`), indent: true, lookupKey: unit.key ?? undefined });
        }
      }
      if (entry.settingsMissing === null) {
        extra.push(
          { label: 'Locked', value: yesNoValue(entry.locked), divider: true },
          { label: 'Recruits regardless of manpower', value: yesNoValue(entry.forceRecruiting) },
          { label: 'Accepts new equipment', value: yesNoValue(entry.newEquipment) },
          { label: 'Accepts foreign equipment', value: yesNoValue(entry.foreignEquipment) },
        );
        if (entry.manpower !== null) extra.push({ label: 'Manpower', value: knownValue(usNumber.format(entry.manpower)) });
        if (entry.obsoleteHour !== null) extra.push({ label: 'Obsolete since', value: knownValue(formatDateTime(entry.obsoleteHour)) });
      } else {
        extra.push({ label: 'Settings', value: notRecorded(entry.settingsMissing), divider: true });
      }
      return extra;
    },
  }),
);
