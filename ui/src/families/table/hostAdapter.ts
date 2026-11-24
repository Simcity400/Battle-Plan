import type { FamilyPlanContext } from '../plan.ts';
import type { FamilyHostAdapter, FamilySettingsPolicy, ParsedFamilyHit } from '../host/adapter.ts';
import type { AnyTableWidgetDeclaration } from './declaration.ts';
import type { FamilyTableBinding } from './laneView.ts';
import { buildColumnLayouts, computeRowLayout, layoutFamilyTable, tableRowWindow } from './layout.ts';
import { deriveTableAudit, resolveTableRows, tableRank, type TableColumnSpec, type TableModel } from './model.ts';
import { encodeTableSettings, MEDIA_ROW_HEIGHT, ROW_HEIGHT, TABLE_SETTINGS_KEY, tableSettingsOf, type TableSettings } from './settings.ts';

interface TableWidthsField {
  readonly widths: Readonly<Record<string, number>>;
  readonly totalWidth: number;
}

interface TableInfoField {
  readonly count: number;
  readonly totalHeight: number;
}

export interface TableChromeState {
  readonly meta: unknown;
  readonly widths: unknown;
}

function tableColumns(declaration: AnyTableWidgetDeclaration, ctx: FamilyPlanContext<unknown> | null): readonly TableColumnSpec[] {
  if (typeof declaration.columns !== 'function') return declaration.columns;
  return ctx ? declaration.columns(ctx) : [];
}

export function parseTableHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  const toggle = hitKey.startsWith('toggle:');
  if (!toggle && !hitKey.startsWith('row:')) return null;
  const key = hitKey.slice(hitKey.indexOf(':') + 1);
  return key ? { key, toggle, inspectToken: `row:${key}` } : null;
}

export const tableSettingsPolicy: FamilySettingsPolicy<TableSettings> = {
  resolve: (raw, ctx, options) => {
    const declaration = raw as AnyTableWidgetDeclaration;
    const columns = tableColumns(declaration, ctx);
    const policy = declaration.settings;
    if (policy.kind === 'fixed') return tableSettingsOf(undefined, columns, policy.base);
    if (policy.kind === 'table') return tableSettingsOf(options[policy.key ?? TABLE_SETTINGS_KEY], columns, policy.base);
    return ctx
      ? policy.resolve(ctx, columns)
      : tableSettingsOf(undefined, columns, { density: 'normal', barsBehind: 'all', sort: { column: columns[0]?.key ?? '', direction: 'descending' } });
  },
  store: (raw, next, live, ctx, setOption) => {
    const policy = (raw as AnyTableWidgetDeclaration).settings;
    if (policy.kind === 'fixed') return;
    if (policy.kind === 'table') {
      setOption(policy.key ?? TABLE_SETTINGS_KEY, encodeTableSettings(next));
      return;
    }
    policy.onChange(next, live, { ...ctx, settings: live }, setOption);
  },
  editable: raw => (raw as AnyTableWidgetDeclaration).settings.kind !== 'fixed',
};

export const tableHostAdapter: FamilyHostAdapter<TableModel, unknown, FamilyTableBinding, TableSettings> = {
  family: 'table',
  skeleton: 'table',
  scroll: 'both',
  navigation: 'rows',
  mirror: 'table',
  mirrorFocus: 'rows',
  tree: true,
  settings: tableSettingsPolicy,
  chromeState: (fields, previous) => {
    const prev = previous as TableChromeState | undefined;
    if (prev && prev.meta === fields.meta && prev.widths === fields.widths) return prev;
    const next: TableChromeState = { meta: fields.meta, widths: fields.widths };
    return next;
  },
  parseHit: parseTableHit,
  getScrollExtent: fields => {
    const widths = fields.widths as TableWidthsField | undefined;
    const table = fields.table as TableInfoField | undefined;
    return {
      ...(widths ? { width: widths.totalWidth } : {}),
      ...(table ? { height: table.totalHeight } : {}),
    };
  },
  deriveAuditRows: (model, _hour, { settings, expanded, viewport, theme, fields }) => {
    if (!settings) return [];
    const measured = (fields.widths as TableWidthsField | undefined)?.widths ?? {};
    const flatRows = resolveTableRows(model, settings, expanded);
    const mode = model.rowHeightMode ?? 'standard';
    const { shown: rankShown, width: rankWidth } = tableRank(model, settings);
    const layout = layoutFamilyTable(buildColumnLayouts(model.columns, settings, measured), Math.max(1, viewport.width), {
      rankShown,
      rankWidth,
    });
    return deriveTableAudit({
      model,
      flatRows,
      visibleColumns: layout.columns,
      rankShown,
      window: tableRowWindow(flatRows.length, computeRowLayout(flatRows, settings.density, mode), (mode === 'media' ? MEDIA_ROW_HEIGHT : ROW_HEIGHT)[settings.density], viewport.scrollTop, viewport.height),
      countryColors: theme ? { countryLooks: theme.countryLooks, country: theme.country } : undefined,
    });
  },
  buildBinding: ctx => (ctx.settings
    ? {
        settings: ctx.settings,
        expanded: ctx.expanded,
        reducedMotion: ctx.reducedMotion,
      }
    : null),
};
