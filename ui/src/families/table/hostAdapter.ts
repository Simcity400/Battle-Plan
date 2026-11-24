import type { FamilyPlanContext } from '../plan.ts';
import type { FamilyHostAdapter, FamilyRowGeometry, FamilySettingsPolicy, ParsedFamilyHit } from '../host/adapter.ts';
import type { AnyTableWidgetDeclaration } from './declaration.ts';
import type { FamilyTableBinding } from './laneView.ts';
import { buildColumnLayouts, layoutFamilyTable } from './layout.ts';
import { deriveTableAudit, resolveTableRows, type TableColumnSpec, type TableModel } from './model.ts';
import { combinedPaneRows, paneGeometry, resolvePanes } from './panes.ts';
import { encodeTableSettings, MEDIA_ROW_HEIGHT, ROW_HEIGHT, TABLE_SETTINGS_KEY, tableSettingsOf, type TableSettings } from './settings.ts';

const OVERSCAN = 4;

interface TableMetaField {
  readonly rowHeightMode?: 'standard' | 'media';
}

interface TableWidthsField {
  readonly widths: Readonly<Record<string, number>>;
  readonly totalWidth: number;
}

interface TableInfoField {
  readonly count: number;
  readonly totalHeight?: number;
}

export interface TableChromeState {
  readonly meta: unknown;
  readonly widths: unknown;
  readonly panes: unknown;
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
  return key ? { key, toggle, inspectToken: hitKey } : null;
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

function rowGeometry(settings: TableSettings | undefined, fields: Readonly<Record<string, unknown>>): FamilyRowGeometry | null {
  if (!settings) return null;
  const media = (fields.meta as TableMetaField | undefined)?.rowHeightMode === 'media';
  return { rowHeight: (media ? MEDIA_ROW_HEIGHT : ROW_HEIGHT)[settings.density], overscan: OVERSCAN };
}

export const tableHostAdapter: FamilyHostAdapter<TableModel, unknown, FamilyTableBinding, TableSettings> = {
  family: 'table',
  skeleton: 'table',
  scroll: 'both',
  navigation: 'rows',
  mirror: 'table',
  mirrorFocus: 'rows',
  tree: true,
  settings: tableSettingsPolicy,
  rowGeometry,
  chromeState: (fields, previous) => {
    const prev = previous as TableChromeState | undefined;
    if (prev && prev.meta === fields.meta && prev.widths === fields.widths && prev.panes === fields.panes) return prev;
    const next: TableChromeState = { meta: fields.meta, widths: fields.widths, panes: fields.panes };
    return next;
  },
  parseHit: parseTableHit,
  inspectToken: key => `row:${key}`,
  getScrollExtent: (fields, rows) => {
    const widths = fields.widths as TableWidthsField | undefined;
    const table = fields.table as TableInfoField | undefined;
    return {
      ...(widths ? { width: widths.totalWidth } : {}),
      ...(table ? { height: table.totalHeight ?? table.count * (rows?.rowHeight ?? ROW_HEIGHT.normal) } : {}),
    };
  },
  deriveAuditRows: (model, _hour, { settings, expanded, window, viewport, theme, fields }) => {
    if (!settings) return [];
    const measured = (fields.widths as TableWidthsField | undefined)?.widths ?? {};
    const panes = model.panes ? resolvePanes(model, settings, expanded, model.rowHeightMode ?? 'standard') : null;
    const flatRows = panes ? combinedPaneRows(panes) : resolveTableRows(model, settings, expanded);
    const rankShown = model.rank === true;
    const rankWidth = rankShown ? (model.rankMarkers ? 64 : 36) : 0;
    const layoutWidth = panes ? paneGeometry(viewport.width, panes.length).width : viewport.width;
    const layout = layoutFamilyTable(buildColumnLayouts(model.columns, settings, measured), Math.max(1, layoutWidth), {
      rankShown,
      rankWidth,
    });
    return deriveTableAudit({
      model,
      flatRows,
      visibleColumns: layout.columns,
      rankShown,
      window: window ?? { first: 0, count: flatRows.length },
      countryColors: theme ? { countryLooks: theme.countryLooks, country: theme.country } : undefined,
    });
  },
  buildBinding: ctx => (ctx.settings
    ? {
        family: 'table',
        kind: ctx.kind,
        view: ctx.view,
        options: ctx.options,
        settings: ctx.settings,
        sources: ctx.sources,
        ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
        ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
        width: ctx.viewport.width,
        height: ctx.viewport.height,
        scrollLeft: ctx.viewport.scrollLeft,
        scrollTop: ctx.viewport.scrollTop,
        theme: ctx.theme,
        ...(ctx.identitySnapshot ? { identitySnapshot: ctx.identitySnapshot } : {}),
        ...(ctx.artResolutions ? { artResolutions: ctx.artResolutions } : {}),
        selectedKey: ctx.selectedKey,
        focusedKey: ctx.focusedKey,
        expanded: ctx.expanded,
        reducedMotion: ctx.reducedMotion,
        ...(ctx.window ? { window: ctx.window } : {}),
        pauseFlush: ctx.pauseFlush,
        ...(ctx.audit ? { audit: true, auditId: ctx.auditId } : {}),
      }
    : null),
};
