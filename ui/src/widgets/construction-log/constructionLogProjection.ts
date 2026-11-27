import { formatAmount } from '../../charts/valueAxis.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { contentKey, contentSet, setToken } from '../../dashboard/options.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { cellArt, newIdentityBoundary, resolveName } from '../../families/table/identityCell.ts';
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
import { knownValue, notApplicable, notRecorded, partialValue, pendingValue, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ConstructionLogRow, ConstructionStatus, ConstructionTypeChip } from '../../generated/dashboard';
import { CONSTRUCTION_LOG_COLUMNS } from './definition.ts';

export interface ConstructionLogProjectionOptions {
  readonly artwork: boolean;
  readonly selected: readonly string[];
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
  atHour: number,
): LogKindFilterItem[] {
  const visible = chips.filter(chip => chip.firstHour <= atHour);
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
    const value = setToken(chip.token);
    return {
      value,
      label,
      checked: allSelected || selected.has(value),
    };
  });
}

function combinePlaceValues(endpoints: readonly Value<string>[], fallbackMissing: string): Value<string> {
  const known: string[] = [];
  const reasons: string[] = [];
  let hasPending = false;
  for (const value of endpoints) {
    if (value.kind === 'known') {
      known.push(value.value);
    } else if (value.kind === 'pending') {
      hasPending = true;
      if (!reasons.includes(value.reason)) reasons.push(value.reason);
    } else if (value.kind === 'notRecorded') {
      if (!reasons.includes(value.reason)) reasons.push(value.reason);
    }
  }
  if (!known.length) {
    if (hasPending) return pendingValue(reasons.join('; '));
    return notRecorded(reasons.join('; ') || fallbackMissing);
  }
  const text = known.join(' – ');
  return reasons.length ? partialValue(text, reasons.join('; ')) : knownValue(text);
}

function resolvePlace(
  readView: BoundedReadView,
  row: ConstructionLogRow,
  boundary: ReturnType<typeof newIdentityBoundary>,
): { readonly cell: TableCell; readonly value: Value<string> | null } {
  const place = row.place;
  if (!place) {
    return {
      cell: { value: notRecorded(row.placeMissing ?? 'Construction place was not recorded'), missingFigure: 'Place' },
      value: null,
    };
  }
  if (place.kind === 'railway') {
    if (place.state === null && place.toState === null) {
      const value = notRecorded(row.placeMissing ?? 'Railway route was not recorded');
      return { cell: { value, missingFigure: 'Place' }, value };
    }
    const endpointMissing = row.placeMissing ?? 'Railway endpoint was not recorded';
    const endpoints: Value<string>[] = [
      place.state !== null
        ? resolveName(readView, 'state', String(place.state), { atHour: place.atHour, boundary }).value
        : notRecorded(endpointMissing),
    ];
    if (place.toState !== place.state) {
      endpoints.push(
        place.toState !== null
          ? resolveName(readView, 'state', String(place.toState), { atHour: place.atHour, boundary }).value
          : notRecorded(endpointMissing),
      );
    }
    const value = combinePlaceValues(endpoints, row.placeMissing ?? 'Railway route was not recorded');
    return { cell: { value, ...(value.kind === 'notRecorded' ? { missingFigure: 'Place' } : {}) }, value };
  }
  if (place.state !== null) {
    const value = resolveName(readView, 'state', String(place.state), { atHour: place.atHour, boundary }).value;
    return { cell: { value, ...(value.kind === 'notRecorded' ? { missingFigure: 'Place' } : {}) }, value };
  }
  if (place.province !== null) {
    const value = resolveName(readView, 'province', String(place.province), { atHour: place.atHour, boundary }).value;
    return { cell: { value, ...(value.kind === 'notRecorded' ? { missingFigure: 'Place' } : {}) }, value };
  }
  return {
    cell: { value: notRecorded(row.placeMissing ?? 'Construction place was not recorded'), missingFigure: 'Place' },
    value: null,
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

export interface FactoryCap {
  readonly cap: number | null;
  readonly missing: string | null;
}

const LINE_CAP_UNKNOWN = 'The recording does not establish the most factories this line takes while it has none';

function lineCap(row: ConstructionLogRow, cap: FactoryCap): FactoryCap {
  return row.kind !== 'building' && row.factories === 0 && cap.cap !== null ? { cap: null, missing: LINE_CAP_UNKNOWN } : cap;
}

const factoriesText = (count: number, cap: FactoryCap) =>
  cap.cap === null ? formatScalar(count, 'integer') : `${formatScalar(count, 'integer')} / ${formatScalar(cap.cap, 'integer')}`;

function factoriesCell(readView: BoundedReadView, row: ConstructionLogRow, cap: FactoryCap, artwork: boolean): TableCell {
  if (!row.live) {
    return { value: notApplicable('No longer in queue'), blank: true };
  }
  if (row.factories === null) {
    return {
      value: notRecorded(row.factoriesMissing ?? 'Assigned factories were not recorded'),
      missingFigure: 'Factories',
    };
  }
  return {
    value: knownValue(factoriesText(row.factories, cap)),
    unitSuffix: (cap.cap ?? row.factories) === 1 ? 'factory' : 'factories',
    ...(artwork ? { unitArt: readView.art({ kind: 'sprite', key: 'GFX_civ_factory', frame: 0 }) } : {}),
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
  cap: FactoryCap,
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

  const place = resolvePlace(readView, row, boundary);
  const levels = levelsCell(row);
  const rowCap = lineCap(row, cap);
  const factories = factoriesCell(readView, row, rowCap, artwork);
  const status = buildStatusCell(row);

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Status', value: knownValue(status.label) },
    ];
    if (place.value && place.value.kind !== 'notRecorded') out.push({ label: 'Place', value: place.value });
    else if (row.placeMissing) out.push({ label: 'Place', value: notRecorded(row.placeMissing) });
    if (levels.text) out.push({ label: 'Levels', value: knownValue(levels.text) });
    else if (row.levelsMissing) out.push({ label: 'Levels', value: notRecorded(row.levelsMissing) });
    if (row.live && rowCap.missing !== null) out.push({ label: 'Most factories', value: notRecorded(rowCap.missing) });
    if (row.live && row.factories !== null) {
      out.push({ label: 'Factories', value: knownValue(factoriesText(row.factories, rowCap)) });
    } else if (!row.live && row.lastFactories !== null) {
      out.push({ label: 'Factories at last read', value: knownValue(formatScalar(row.lastFactories, 'integer')) });
    }
    out.push({ label: 'Started', value: knownValue(formatDateTime(row.createdHour ?? row.firstRead)) });
    if (row.end !== null) {
      const endLabel = row.end.kind === 'finished' ? 'Finished' : row.end.kind === 'removed' ? 'Removed' : 'Left the queue';
      out.push({ label: endLabel, value: knownValue(formatDateTime(row.end.hour ?? row.end.knownAt)) });
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
      readonly factoryCap?: number | null;
      readonly factoryCapMissing?: string | null;
      readonly unavailable?: string | null;
      readonly warnings?: readonly string[];
      readonly buildingTypes?: readonly ConstructionTypeChip[];
    } | undefined;
    const items = constructionTypeFilterItems(rawHead?.buildingTypes ?? [], options.selected, Math.min(hour, source.at ?? hour));
    const costScale = typeof rawHead?.costScale === 'number' && rawHead.costScale > 0 ? rawHead.costScale : 100_000;
    const cap: FactoryCap = Number.isSafeInteger(rawHead?.factoryCap) && rawHead!.factoryCap! > 0
      ? { cap: rawHead!.factoryCap!, missing: null }
      : { cap: null, missing: rawHead?.factoryCapMissing ?? 'The most factories a construction line takes is not recorded' };
    const boundary = newIdentityBoundary();
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawRow => buildConstructionRow(readView, rawRow, options.artwork, costScale, cap, boundary),
    );
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
      kindFilter: { key: 'types', noun: 'buildings', allLabel: 'All buildings', items, selected: options.selected },
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
  countries: { mode: 'filter', content: 'history' },
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
    return { artwork: ctx.options.artwork !== false, selected: contentSet(ctx.options.types ?? '') };
  },
};
