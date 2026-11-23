import { usNumber } from '../../charts/numberFormat.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableColumnSpec,
  type TableProjection,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED } from '../../families/table/settings.ts';
import { knownValue, notRecorded, valueFromNullable, type Value } from '../../families/value.ts';
import type { DivisionTemplatesReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition.ts';
import { designerSnapshot } from './designerSnapshot.ts';
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

const TEMPLATE_COLUMNS: readonly DeclarativeColumnSpec<TemplateRow, DivisionTemplateProjectionOptions>[] = [
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
    iconSlot: true,
    icon: row => (row.kind === 'template' ? row.entry.icon ?? undefined : undefined),
    value: row => knownValue(row.kind === 'obsolete' ? `Obsolete templates (${row.count})` : templateName(row.entry)),
    sortValue: row => row.kind === 'obsolete'
      ? knownValue(Number.MAX_SAFE_INTEGER)
      : valueFromNullable(row.entry.rosterOrdinal, { kind: 'notRecorded', reason: 'Roster order not recorded' }),
  },
  {
    key: 'fielded',
    label: 'Fielded',
    as: 'plain',
    cellBarWhen: opt => opt.cellBars,
    tone: 'blue',
    offered: OFFERED.number,
    priority: 1,
    numeric: true,
    format: 'integer',
    widest: ['999'],
    path: 'entry.fielded',
    whenNull: { kind: 'notRecorded', reason: 'Fielded count not recorded' },
  },
  {
    key: 'battalions',
    label: 'Battalions',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    text: true,
    widest: ['Infantry ×9 · Artillery'],
    value: row => valueFromNullable(
      row.kind === 'template' ? battalionSummary(row.entry) : null,
      { kind: 'notRecorded', reason: row.kind === 'template' ? (row.entry.unitsMissing ?? 'Composition not recorded') : 'Composition not recorded' },
    ),
    sortValue: row => valueFromNullable(
      row.kind === 'template' ? battalionCount(row.entry) : null,
      { kind: 'notRecorded', reason: row.kind === 'template' ? (row.entry.unitsMissing ?? 'Composition not recorded') : 'Composition not recorded' },
    ),
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
    path: 'entry.baseWidthRaw',
    rawBigInt: true,
    whenNull: { kind: 'notRecorded', reason: 'Base combat width not recorded' },
  },
  {
    key: 'priority',
    label: 'Priority',
    as: 'plain',
    offered: OFFERED.text,
    priority: 4,
    text: true,
    widest: ['Reserve'],
    value: row => valueFromNullable(
      row.kind === 'template' ? priorityLabel(row.entry) : null,
      { kind: 'notRecorded', reason: row.kind === 'template' ? (row.entry.priorityMissing ?? 'Priority not recorded') : 'Priority not recorded' },
    ),
    sortPath: 'entry.priority',
    whenNull: { kind: 'notRecorded', reasonField: 'entry.priorityMissing', reason: 'Priority not recorded' },
  },
];

export function divisionTemplateColumns(cellBars = false): TableColumnSpec[] {
  return TEMPLATE_COLUMNS.map(col => (col.cellBarWhen ? { ...col, as: cellBars ? 'faint' : 'plain' } : col));
}

const yesNoValue = (value: boolean | null): Value<string> =>
  value === null ? notRecorded('Setting not recorded') : knownValue(value ? 'Yes' : 'No');

export const divisionTemplateProjection: TableProjection<DivisionTemplateProjectionOptions> = registerTableProjection(
  declareTableProjection<DivisionTemplatesReply, TemplateRow, DivisionTemplateProjectionOptions>({
    kind: 'division-template',
    validate: validateDivisionTemplates,
    naWhen: row => (row.kind === 'obsolete' ? 'Summary row for obsolete templates' : null),
    columns: TEMPLATE_COLUMNS,
    rows: (reply, _live, _hour, options) => {
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
    rowMeta: (row, reply) => (row.kind === 'template' ? { designer: row.entry.designer ?? 'ordinary', tag: reply.tag } : null),
    children: row => (row.kind === 'obsolete' ? row.children : undefined),
    initiallyOpen: () => false,
    tooltipAction: (row, options) => (row.kind === 'template' && options.canOpenDesigner ? 'Click for division designer' : undefined),
    tooltipNote: (row, _reply, _live, _hour, options) => (row.kind === 'template' && !options.canOpenDesigner ? 'Game view not available yet' : undefined),
    tooltipExtra: (row): TooltipExtraRow[] | undefined => {
      if (row.kind === 'obsolete') return undefined;
      const entry = row.entry;
      const extra: TooltipExtraRow[] = [];
      const line = entry.units.filter(unit => !unit.support);
      const support = entry.units.filter(unit => unit.support);
      if (line.length) {
        extra.push({ label: 'Line battalions', value: knownValue(String(line.reduce((s, u) => s + u.count, 0))), strong: true, divider: true });
        for (const unit of line) extra.push({ label: unitName(unit), value: knownValue(`\u00d7${unit.count}`), indent: true, lookupKey: unit.key ?? undefined });
      }
      if (support.length) {
        extra.push({ label: 'Support companies', value: knownValue(String(support.length)), strong: true });
        for (const unit of support) extra.push({ label: unitName(unit), value: knownValue(unit.count === 1 ? '1' : `\u00d7${unit.count}`), indent: true, lookupKey: unit.key ?? undefined });
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

export const divisionTemplateDeclaration: TableWidgetDeclaration<DivisionTemplateProjectionOptions> = {
  family: 'table',
  kind: 'division-template',
  label: 'Division templates',
  subject: 'templates',
  emptyCampaign: 'Load a campaign to see templates.',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its templates.' },
  source: {
    kind: 'panel',
    product: 'division-templates',
    params: ctx => (ctx.countryTag
      ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), countFor: ctx.options.countFor === 'host' ? 'host' : 'owner' }
      : null),
    validate: validateDivisionTemplates,
  },
  projection: divisionTemplateProjection,
  columns: ctx => divisionTemplateColumns(ctx.options.cellBars === true),
  settings: { kind: 'options', defaultSort: DEFAULT_SORT },
  projectionOptions: ctx => ({
    countryName: ctx.countryName,
    cellBars: ctx.options.cellBars === true,
    canOpenDesigner: Boolean(designerSnapshot()),
  }),
};
