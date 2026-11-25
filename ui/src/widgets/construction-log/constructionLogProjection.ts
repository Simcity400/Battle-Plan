import { formatAmount } from '../../charts/valueAxis.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { cellArt, newIdentityBoundary } from '../../families/table/identityCell.ts';
import {
  LOG_TABLE_BASE,
  logCountryCell,
  logDateCell,
  logStatusCell,
  occurrenceSelection,
  pagedTailEntry,
  projectPagedSlots,
  urlArtHandle,
} from '../../families/table/logHelpers.ts';
import type { LogKindFilterItem } from '../../families/table/LogKindFilter.tsx';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ConstructionLogRow, ConstructionStatus, ConstructionTypeChip } from '../../generated/dashboard';
import { CONSTRUCTION_LOG_COLUMNS } from './definition.ts';

export interface ConstructionLogProjectionOptions {
  readonly artwork: boolean;
}

const HEAD_LIMIT = 256;

const CONSTRUCTION_STATUS_INFO: Readonly<Record<ConstructionStatus, { readonly mark?: StatusMarkKind; readonly label: string; readonly muted?: boolean; readonly notRecorded?: boolean }>> = {
  building: { mark: 'running', label: 'Building' },
  repairing: { mark: 'running', label: 'Repairing' },
  waiting: { mark: 'waiting', label: 'Waiting' },
  observed: { mark: 'waiting', label: 'Observed' },
  unread: { label: 'Not recorded', muted: true, notRecorded: true },
  built: { mark: 'finished', label: 'Built' },
  repaired: { mark: 'finished', label: 'Repaired' },
  repairedAndBuilt: { mark: 'finished', label: 'Repaired and built' },
  completed: { mark: 'finished', label: 'Completed' },
  removed: { mark: 'ended', label: 'Removed', muted: true },
  leftQueue: { mark: 'ended', label: 'Left the queue', muted: true },
};

export function constructionTypeFilterItems(
  chips: readonly ConstructionTypeChip[],
  selectedTokens: readonly string[],
  atHour?: number,
): LogKindFilterItem[] {
  const visible = atHour === undefined ? chips : chips.filter(chip => chip.firstHour <= atHour);
  const baseLabels = visible.map(chip => contentTitle(chip.name, chip.token, 'Building name unavailable'));
  const counts = new Map<string, number>();
  for (const label of baseLabels) counts.set(label, (counts.get(label) ?? 0) + 1);
  const seen = new Map<string, number>();
  const allSelected = selectedTokens.length === 0;
  const selected = new Set(selectedTokens);
  return visible.map((chip, index) => {
    const base = baseLabels[index]!;
    const ordinal = (seen.get(base) ?? 0) + 1;
    seen.set(base, ordinal);
    const label = counts.get(base)! > 1 ? `${base} (${ordinal})` : base;
    return {
      value: chip.token,
      label,
      checked: allSelected || selected.has(chip.token),
    };
  });
}

function resolveStateLabel(
  readView: BoundedReadView,
  stateId: number,
  atHour: number,
): { readonly label: string | null; readonly reason: string | null } {
  const res = readView.who('state', String(stateId), atHour);
  if (res.name.status === 'ready' && res.name.value.trim()) {
    return { label: res.name.value, reason: null };
  }
  if (res.name.status === 'pending') {
    return { label: `State ${stateId}`, reason: null };
  }
  return { label: null, reason: res.name.status === 'ready' ? 'State name not recorded' : res.name.reason };
}

function resolvePlace(
  readView: BoundedReadView,
  row: ConstructionLogRow,
): { readonly cell: TableCell; readonly label: string | null } {
  const place = row.place;
  if (!place) {
    return {
      cell: { value: notRecorded(row.placeMissing ?? 'Construction place was not recorded'), missingFigure: 'Place' },
      label: null,
    };
  }
  if (place.kind === 'railway') {
    const from = place.state !== null ? resolveStateLabel(readView, place.state, place.atHour) : null;
    const to = place.toState !== null && place.toState !== place.state
      ? resolveStateLabel(readView, place.toState, place.atHour)
      : null;
    if (from?.label && to?.label) {
      const label = `${from.label} – ${to.label}`;
      return { cell: { value: knownValue(label) }, label };
    }
    const single = from?.label ?? to?.label ?? null;
    if (single) {
      return { cell: { value: knownValue(single) }, label: single };
    }
    const reason = from?.reason ?? to?.reason ?? row.placeMissing ?? 'Railway route was not recorded';
    return { cell: { value: notRecorded(reason), missingFigure: 'Place' }, label: null };
  }
  if (place.state !== null) {
    const resolved = resolveStateLabel(readView, place.state, place.atHour);
    if (resolved.label) {
      return { cell: { value: knownValue(resolved.label) }, label: resolved.label };
    }
    return { cell: { value: notRecorded(resolved.reason ?? 'State name not recorded'), missingFigure: 'Place' }, label: null };
  }
  if (place.province !== null) {
    const res = readView.who('province', String(place.province), place.atHour);
    if (res.name.status === 'ready' && res.name.value.trim()) {
      return { cell: { value: knownValue(res.name.value) }, label: res.name.value };
    }
    if (res.name.status === 'pending') {
      const label = `Province ${place.province}`;
      return { cell: { value: knownValue(label) }, label };
    }
    return {
      cell: { value: notRecorded(res.name.status === 'ready' ? 'Province name not recorded' : res.name.reason), missingFigure: 'Place' },
      label: null,
    };
  }
  return {
    cell: { value: notRecorded(row.placeMissing ?? 'Construction place was not recorded'), missingFigure: 'Place' },
    label: null,
  };
}

function levelsCell(row: ConstructionLogRow): { readonly cell: TableCell; readonly text: string | null } {
  const levels = row.levels;
  if (!levels) {
    return {
      cell: { value: notRecorded(row.levelsMissing ?? 'Construction levels were not recorded'), missingFigure: 'Levels' },
      text: null,
    };
  }
  const more = levels.earlierUnknown || levels.countersUnread ? '+' : '';
  const extra = levels.unknown + levels.possible;
  if (extra > 0 && levels.built === 0 && extra === 1 && levels.total <= 1 && !more) {
    return {
      cell: { value: knownValue('0 or 1'), unitSuffix: 'level' },
      text: '0 or 1 level',
    };
  }
  const builtStr = extra > 0
    ? `${formatScalar(levels.built, 'integer')}–${formatScalar(levels.built + extra, 'integer')}${more}`
    : `${formatScalar(levels.built, 'integer')}${more}`;
  const totalStr = levels.possible > 0
    ? `${formatScalar(levels.total, 'integer')}–${formatScalar(levels.total + levels.possible, 'integer')}${more}`
    : `${formatScalar(levels.total, 'integer')}${more}`;
  const unitSuffix = (levels.total + levels.possible) === 1 && !more ? 'level' : 'levels';
  const valueText = `${builtStr} / ${totalStr}`;
  return {
    cell: { value: knownValue(valueText), unitSuffix },
    text: `${valueText} ${unitSuffix}`,
  };
}

function factoriesCell(row: ConstructionLogRow): TableCell {
  if (!row.live) {
    return { value: notApplicable('No longer in queue'), blank: true };
  }
  if (row.factories === null) {
    return {
      value: notRecorded(row.factoriesMissing ?? 'Assigned factories were not recorded'),
      missingFigure: 'Factories',
    };
  }
  const valueText = formatScalar(row.factories, 'integer');
  return {
    value: knownValue(valueText),
    unitSuffix: row.factories === 1 ? 'factory' : 'factories',
  };
}

function buildStatusCell(row: ConstructionLogRow): { readonly cell: TableCell; readonly label: string; readonly muted: boolean } {
  const info = CONSTRUCTION_STATUS_INFO[row.status];
  if (info.notRecorded) {
    return {
      cell: { value: notRecorded('Construction status was not recorded at this hour'), missingFigure: 'Status' },
      label: info.label,
      muted: true,
    };
  }
  return {
    cell: logStatusCell(info.label, info.mark!, info.muted ? 'muted' : undefined),
    label: info.label,
    muted: info.muted === true,
  };
}

function buildConstructionRow(
  readView: BoundedReadView,
  row: ConstructionLogRow,
  artwork: boolean,
  costScale: number,
  boundary: ReturnType<typeof newIdentityBoundary>,
): TableRow {
  const countryCell = logCountryCell(
    readView,
    [{
      tag: row.owner.tag,
      atHour: row.firstRead,
      missingLabel: row.owner.missing ?? "The construction line's owner is not recorded",
    }],
    row.firstRead,
    boundary,
  );

  const hasBuilding = row.buildingName !== null || row.buildingToken !== null;
  const itemTitle = hasBuilding
    ? contentTitle(row.buildingName, row.buildingToken, 'Building name unavailable')
    : 'Building not recorded';
  const itemCell: TableCell = hasBuilding
    ? { value: knownValue(itemTitle) }
    : { value: notRecorded(row.buildingMissing ?? 'Building type was not recorded') };

  const artUrl = artwork ? drawableIcon(row.icon) : null;
  const artHandle = urlArtHandle(artUrl);
  const kindCell: TableCell = artwork
    ? artHandle
      ? {
          value: hasBuilding ? knownValue(itemTitle) : notRecorded(row.buildingMissing ?? 'Building icon not recorded'),
          iconSlot: true,
          artShape: 'square',
          artName: itemTitle,
          ...cellArt(readView, artHandle, row.buildingMissing ?? 'Building icon not recorded'),
        }
      : {
          value: notRecorded(row.buildingMissing ?? 'Building icon not recorded'),
          iconSlot: true,
          artShape: 'square',
          artName: itemTitle,
        }
    : { value: notApplicable('Game artwork hidden'), blank: true };

  const place = resolvePlace(readView, row);
  const levels = levelsCell(row);
  const factories = factoriesCell(row);
  const status = buildStatusCell(row);

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Status', value: knownValue(status.label) },
    ];
    if (place.label) out.push({ label: 'Place', value: knownValue(place.label) });
    else if (row.placeMissing) out.push({ label: 'Place', value: notRecorded(row.placeMissing) });
    if (levels.text) out.push({ label: 'Levels', value: knownValue(levels.text) });
    else if (row.levelsMissing) out.push({ label: 'Levels', value: notRecorded(row.levelsMissing) });
    if (row.live && row.factories !== null) {
      out.push({ label: 'Factories', value: knownValue(formatScalar(row.factories, 'integer')) });
    } else if (!row.live && row.lastFactories !== null) {
      out.push({ label: 'Factories at last read', value: knownValue(formatScalar(row.lastFactories, 'integer')) });
    }
    if (row.createdHour !== null) {
      out.push({ label: 'Started', value: knownValue(formatDateTime(row.createdHour)) });
    } else {
      out.push({ label: 'First seen', value: knownValue(formatDateTime(row.firstRead)) });
    }
    if (row.end !== null) {
      const endLabel = row.end.kind === 'finished' ? 'Finished' : row.end.kind === 'removed' ? 'Removed' : 'Left the queue';
      const dateStr = row.end.hour !== null ? formatDateTime(row.end.hour) : `by ${formatDateTime(row.end.knownAt)}`;
      out.push({ label: endLabel, value: knownValue(dateStr) });
    }
    if (row.costRaw !== null && costScale > 0) {
      const scale = Math.round(Math.log10(costScale));
      out.push({ label: row.kind === 'railway' ? 'Route cost' : 'Cost per level', value: knownValue(formatAmount(row.costRaw, scale, 0)) });
    }
    if (row.output && costScale > 0) {
      const scale = Math.round(Math.log10(costScale));
      out.push({
        label: 'Construction output',
        value: row.output.constructionRaw !== null
          ? knownValue(formatAmount(row.output.constructionRaw, scale, 0))
          : notRecorded('Construction output not recorded'),
      });
      if (row.output.repairRaw !== null && row.output.repairRaw !== '0') {
        out.push({ label: 'Repair output', value: knownValue(formatAmount(row.output.repairRaw, scale, 0)) });
      }
    }
    if (row.kind === 'railway' && row.source === 'census') {
      if (row.baseLevel !== null || row.targetLevel !== null) {
        out.push({ label: 'Railway level', value: knownValue(`${row.baseLevel ?? '?'} → ${row.targetLevel ?? '?'}`) });
      }
      if (row.routeProvinces !== null) {
        out.push({ label: 'Provinces on route', value: knownValue(formatScalar(row.routeProvinces, 'integer')) });
      }
    }
    return out;
  };

  return {
    key: row.key,
    name: itemTitle,
    rowHour: row.hour,
    live: row.live,
    muted: status.muted,
    cells: {
      date: logDateCell(row.hour),
      kind: kindCell,
      country: countryCell,
      item: itemCell,
      place: place.cell,
      levels: levels.cell,
      factories,
      status: status.cell,
    },
    tooltipTitle: itemTitle,
    tooltipExtra,
  };
}

export const constructionLogProjection: TableProjection<ConstructionLogProjectionOptions> = registerFamilyProjection({
  kind: 'construction-log',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: CONSTRUCTION_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused };
    }
    const source = readView.rows<ConstructionLogRow & DatedLogRow>('main');
    if (!source) {
      return { columns: CONSTRUCTION_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true };
    }
    if (source.refused !== null) {
      return { columns: CONSTRUCTION_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused: source.refused };
    }
    const rawHead = source.rawHead as {
      readonly costScale?: number;
      readonly unavailable?: string | null;
      readonly warnings?: readonly string[];
    } | undefined;
    const costScale = typeof rawHead?.costScale === 'number' && rawHead.costScale > 0 ? rawHead.costScale : 100_000;
    const boundary = newIdentityBoundary();
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawRow => buildConstructionRow(readView, rawRow, options.artwork, costScale, boundary),
    );
    const activeCount = source.facetTotals?.get('active') ?? null;
    const titleSummary = activeCount !== null && activeCount > 0
      ? `${formatScalar(activeCount, 'integer')} building`
      : null;
    const warnings = rawHead?.warnings?.length ? [...rawHead.warnings] : undefined;
    const unavailable = typeof rawHead?.unavailable === 'string' && rawHead.unavailable.length > 0
      ? rawHead.unavailable
      : null;

    return {
      columns: CONSTRUCTION_LOG_COLUMNS,
      rows,
      slots,
      hour,
      hideHeader: true,
      unsorted: true,
      dateGrouped: true,
      windowOffset,
      totalRows,
      ...(gapBefore?.length ? { gapBefore } : {}),
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      titleSummary,
      ...(source.missing.length > 0 && titleSummary ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      ...(warnings ? { warnings } : {}),
      ...(slots.length === 0
        ? unavailable
          ? { empty: 'Construction was not recorded.', emptyDetail: unavailable, emptyKind: 'unavailable' as const }
          : { empty: 'No recorded construction projects match these settings.' }
        : {}),
    };
  },
});

export const constructionLogDeclaration: TableWidgetDeclaration<ConstructionLogProjectionOptions> = {
  family: 'table',
  kind: 'construction-log',
  label: 'Construction log',
  subject: 'construction projects',
  emptyCampaign: 'Load a campaign to see its construction log.',
  icon: 'hammer',
  countries: { mode: 'filter' },
  columns: CONSTRUCTION_LOG_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const selectedTypes = contentSet(ctx.options.types ?? '').map(contentKey);
    const projects = ctx.options.projects === 'new' || ctx.options.projects === 'repair' ? ctx.options.projects : 'all';
    const inQueue = ctx.options.queue === true;
    const order = ctx.options.order === 'start' ? 'start' : 'change';
    const selection = occurrenceSelection(ctx);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:construction-log:${selectedTypes.join(',')}:${projects}:${inQueue ? 1 : 0}:${order}:${selection ? JSON.stringify(selection) : 'all'}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'construction-log-tail',
            branchId: ctx.branch.id,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ask: {
              ...(selectedTypes.length ? { buildingTypes: selectedTypes } : {}),
              ...(projects !== 'all' ? { projects } : {}),
              ...(inQueue ? { inQueue: true } : {}),
              ...(order === 'start' ? { order: 'start' } : {}),
              ...selection,
            },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
        ),
      ],
    };
  },
  projection: constructionLogProjection,
  projectionOptions(ctx): ConstructionLogProjectionOptions {
    return { artwork: ctx.options.artwork !== false };
  },
};
