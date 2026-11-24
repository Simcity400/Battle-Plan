import { countryFromKey } from '../../dashboard/countryKey.ts';
import { gameWindow } from '../../dashboard/gameWindows';
import { object } from '../../dashboard/validation.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artNameCell, urlArtHandle } from '../../families/table/identityCell.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import type { EquipmentDesignsReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export interface DesignsProjectionOptions {
  readonly canOpen: boolean;
}

export interface DesignMeta {
  readonly tag: string;
  readonly variant: string;
  readonly name: string;
}

const TEXT = (value: unknown) => value === null || typeof value === 'string';

export function validateDesigns(value: unknown): EquipmentDesignsReply {
  const reply = value as EquipmentDesignsReply;
  if (!object(reply) || typeof reply.tag !== 'string' || !TEXT(reply.missing) || !Array.isArray(reply.warnings)
    || !Array.isArray(reply.icons) || !reply.icons.every(icon => object(icon) && typeof icon.key === 'string' && typeof icon.image === 'string')
    || !Array.isArray(reply.designs) || !reply.designs.every(design => object(design) && typeof design.obsolete === 'boolean' && object(design.name)
      && /^\d{1,20}$/.test(design.name.id) && typeof design.name.token === 'string'
      && [design.name.name, design.name.typeName, design.name.nameUnknown, design.name.icon, design.name.iconUnknown].every(TEXT))) {
    throw new Error('Invalid equipment designs reply');
  }
  return reply;
}

const PRIORITY: Readonly<Record<string, number>> = { design: 0, state: 1, type: 2, view: 3 };
export const DESIGN_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  text: true,
  ...(column.key === 'design' ? { cellKind: 'item' as const, pinned: true } : {}),
  ...(column.key === 'state' ? { cellKind: 'status' as const } : {}),
  ...(column.key === 'view' ? { cellKind: 'action' as const, sortable: false } : {}),
}));

export const designsProjection: TableProjection<DesignsProjectionOptions> = registerTableProjection({
  kind: 'equipment-designs',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: DESIGN_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: DESIGN_COLUMNS, rows: [], hour };
    const reply = validateDesigns(raw);
    if (reply.missing !== null) {
      return { columns: DESIGN_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }
    const images = new Map(reply.icons.map(icon => [icon.key, icon.image]));
    const rows: TableRow[] = reply.designs.map(({ name: design, obsolete }) => {
      const image = design.icon ? images.get(design.icon) ?? null : null;
      const meta: DesignMeta = { tag: reply.tag, variant: design.id, name: design.name ?? design.id };
      return {
        key: design.id,
        name: design.name ?? design.id,
        cells: {
          design: artNameCell(readView, design.name, urlArtHandle(image), design.nameUnknown ?? 'Design name not recorded', 'wide'),
          type: { value: design.typeName ? knownValue(design.typeName) : notRecorded(design.nameUnknown ?? 'Equipment type name not recorded') },
          state: { value: knownValue(obsolete ? 'Obsolete' : 'Current'), statusKind: obsolete ? 'ended' : 'running', sortValue: knownValue(obsolete ? 1 : 0) },
          view: { value: options.canOpen ? knownValue('View') : notApplicable('The equipment designer is not available') },
        },
        ...(options.canOpen ? { tooltipAction: 'Open in the designer' } : {}),
        meta,
      };
    });
    return {
      columns: DESIGN_COLUMNS,
      rows,
      hour,
      rowTimeBoundaries: false,
      ...(rows.length === 0 ? { empty: 'No saved designs', emptyKind: 'empty' as const } : {}),
      warnings: reply.warnings,
    };
  },
});

export const designsDeclaration: TableWidgetDeclaration<DesignsProjectionOptions> = {
  family: 'table',
  kind: 'equipment-designs',
  label: 'Equipment designs',
  subject: 'designs',
  emptyCampaign: 'Load a campaign to see designs.',
  icon: 'layers',
  countries: { mode: 'single', choose: 'Choose a country to see its designs.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) } : null;
    return {
      owner: `equipment-designs:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
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
              product: 'equipment-designs',
              followQuery: { panel: 'equipment-designs', ...params },
              params,
              validate: validateDesigns,
            },
          }]
        : [],
    };
  },
  projection: designsProjection,
  columns: DESIGN_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: () => ({ canOpen: gameWindow('equipmentDesigner') !== null }),
};
