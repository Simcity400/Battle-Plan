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
import { knownValue, notApplicable, notRecorded, partialValue, valueFromNullable, type Value } from '../../families/value.ts';
import type { DivisionTemplatesReply, TemplateEntry } from '../../generated/dashboard';
import { gameWindow } from '../../dashboard/gameWindows.ts';
import { DEFAULT_SORT, OPTIONS } from './definition.ts';
import {
  battalionCount,
  priorityLabel,
  templateName,
  templateRows,
  validateDivisionTemplates,
  type TemplateRow,
} from './divisionTemplateData.ts';

function compositionValue(entry: TemplateEntry): Value<string> {
  if (entry.unitsMissing !== null && !entry.units.length) {
    return notRecorded(entry.unitsMissing);
  }
  if (!entry.units.length) return knownValue('Empty');
  const missingReasons: string[] = [];
  const lineKnown: string[] = [];
  const supportKnown: string[] = [];
  for (const unit of entry.units) {
    const trimmed = unit.name?.trim();
    if (!trimmed) {
      missingReasons.push(unit.nameMissing ?? 'Unit name unavailable');
      continue;
    }
    if (unit.support) supportKnown.push(unit.count === 1 ? trimmed : `${trimmed} \u00d7${unit.count}`);
    else lineKnown.push(`${trimmed} \u00d7${unit.count}`);
  }
  const knownText = [lineKnown.join(', '), supportKnown.join(', ')].filter(Boolean).join(' \u00b7 ');
  if (!missingReasons.length) return knownValue(knownText || 'Empty');
  const reason = [...new Set(missingReasons)].join('; ');
  return knownText ? partialValue(knownText, reason) : notRecorded(reason);
}

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
    value: row => row.kind === 'obsolete'
      ? knownValue(`Obsolete templates (${row.count})`)
      : row.kind === 'status-unknown'
        ? notRecorded(row.reason)
        : valueFromNullable(
          row.entry.name && row.entry.name.trim() ? row.entry.name : null,
          { kind: 'notRecorded', reason: row.entry.nameMissing ?? 'Template name unavailable' },
        ),
    sortValue: row => row.kind === 'obsolete'
      ? knownValue(Number.MAX_SAFE_INTEGER)
      : row.kind === 'status-unknown'
        ? knownValue(Number.MAX_SAFE_INTEGER - 1)
        : valueFromNullable(row.entry.rosterOrdinal, { kind: 'notRecorded', reason: 'Roster order not recorded' }),
  },
  {
    key: 'fielded',
    label: 'Fielded',
    as: 'faint',
    barEligible: true,
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
    value: row => row.kind === 'template'
      ? compositionValue(row.entry)
      : notRecorded('Composition not recorded'),
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
  {
    key: 'view',
    label: '',
    as: 'plain',
    cellKind: 'action',
    offered: OFFERED.text,
    priority: 5,
    sortable: false,
    hideable: false,
    value: (row, _live, _hour, options) => (row.kind === 'template' && options.canOpenDesigner && row.entry.designer !== null
      ? knownValue('View')
      : notApplicable('This row has no designer window')),
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
    naWhen: row => (row.kind === 'obsolete'
      ? 'Summary row for obsolete templates'
      : row.kind === 'status-unknown'
        ? 'Summary row for templates with unrecorded status'
        : null),
    columns: TEMPLATE_COLUMNS,
    rows: (reply, _live, _hour, options) => {
      const rows = templateRows(reply);
      if (!rows.length) {
        return {
          rows: [],
          empty: `${options.countryName} has no division templates at this date.`,
          emptyKind: 'empty',
          warnings: reply.warnings,
        };
      }
      return { rows, warnings: reply.warnings };
    },
    rowKey: row => row.key,
    rowName: row => (row.kind === 'obsolete'
      ? `Obsolete templates (${row.count})`
      : row.kind === 'status-unknown'
        ? `Status not recorded (${row.count})`
        : templateName(row.entry)),
    rowMeta: (row, reply) => (row.kind === 'template'
      ? { designer: row.entry.designer, designerMissing: row.entry.settingsMissing, tag: reply.tag }
      : null),
    children: row => (row.kind === 'obsolete' || row.kind === 'status-unknown' ? row.children : undefined),
    initiallyOpen: () => false,
    tooltipAction: (row, options) => (row.kind === 'template' && options.canOpenDesigner && row.entry.designer !== null
      ? 'Click for division designer'
      : undefined),
    tooltipNote: (row, _reply, _live, _hour, options) => {
      if (row.kind !== 'template') return undefined;
      if (!options.canOpenDesigner) return 'Game view not available yet';
      if (row.entry.designer === null) return row.entry.settingsMissing ?? 'Division designer type not recorded';
      return undefined;
    },
    tooltipExtra: (row): TooltipExtraRow[] | undefined => {
      if (row.kind !== 'template') return undefined;
      const entry = row.entry;
      const extra: TooltipExtraRow[] = [];
      const line = entry.units.filter(unit => !unit.support);
      const support = entry.units.filter(unit => unit.support);
      if (line.length) {
        extra.push({ label: 'Line battalions', value: knownValue(String(line.reduce((s, u) => s + u.count, 0))), strong: true, divider: true });
        for (const unit of line) {
          const name = unit.name?.trim();
          extra.push(name
            ? { label: name, value: knownValue(`\u00d7${unit.count}`), indent: true, lookupKey: unit.key ?? undefined }
            : { label: `Line unit (\u00d7${unit.count})`, value: notRecorded(unit.nameMissing ?? 'Unit name unavailable'), indent: true, lookupKey: unit.key ?? undefined });
        }
      }
      if (support.length) {
        extra.push({ label: 'Support companies', value: knownValue(String(support.length)), strong: true });
        for (const unit of support) {
          const name = unit.name?.trim();
          extra.push(name
            ? { label: name, value: knownValue(unit.count === 1 ? '1' : `\u00d7${unit.count}`), indent: true, lookupKey: unit.key ?? undefined }
            : { label: `Support unit (${unit.count === 1 ? '1' : `\u00d7${unit.count}`})`, value: notRecorded(unit.nameMissing ?? 'Unit name unavailable'), indent: true, lookupKey: unit.key ?? undefined });
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

export const divisionTemplateDeclaration: TableWidgetDeclaration<DivisionTemplateProjectionOptions> = {
  family: 'table',
  kind: 'division-template',
  label: 'Division templates',
  subject: 'templates',
  emptyCampaign: 'Load a campaign to see templates.',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its templates.' },
  sources: ctx => {
    const params = ctx.countryTag
      ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), countFor: ctx.options.countFor === 'host' ? 'host' : 'owner' }
      : null;
    return {
      owner: `division-template:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'panel',
              coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
              product: 'division-templates',
              followQuery: { panel: 'division-templates', ...params },
              params,
              validate: validateDivisionTemplates,
            },
          }]
        : [],
    };
  },
  projection: divisionTemplateProjection,
  columns: ctx => divisionTemplateColumns(ctx.options.cellBars === true),
  settings: { kind: 'options', defaultSort: DEFAULT_SORT },
  projectionOptions: ctx => ({
    countryName: ctx.countryName,
    cellBars: ctx.options.cellBars === true,
    canOpenDesigner: Boolean(gameWindow('divisionDesigner')),
  }),
};
