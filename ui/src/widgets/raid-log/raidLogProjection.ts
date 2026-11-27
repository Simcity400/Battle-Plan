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
import { knownValue, notApplicable, notRecorded, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { RaidLogRow, RaidStatus, RaidTypeChip, RaidVictimSource } from '../../generated/dashboard';
import { RAID_LOG_COLUMNS } from './definition.ts';

export interface RaidLogProjectionOptions {
  readonly artwork: boolean;
  readonly selected: readonly string[];
}

const HEAD_LIMIT = 256;

const RAID_STATUS_INFO: Readonly<Record<RaidStatus, { readonly mark?: StatusMarkKind; readonly label: string; readonly muted?: boolean; readonly notRecorded?: boolean }>> = {
  assembling: { mark: 'waiting', label: 'Assembling' },
  preparing: { mark: 'running', label: 'Preparing' },
  prepared: { mark: 'waiting', label: 'Ready to launch' },
  inProgress: { mark: 'running', label: 'In progress' },
  succeeded: { mark: 'succeeded', label: 'Succeeded' },
  criticalSuccess: { mark: 'succeeded', label: 'Critical success' },
  limitedSuccess: { mark: 'finished', label: 'Limited success' },
  failed: { mark: 'ended', label: 'Failed', muted: true },
  cancelled: { mark: 'ended', label: 'Cancelled', muted: true },
  outcomeNotRecorded: { label: 'Outcome not recorded', muted: true, notRecorded: true },
  phaseNotRecorded: { label: 'Phase not recorded', muted: true, notRecorded: true },
  notRead: { label: 'Not recorded', muted: true, notRecorded: true },
};

function isSuccessStatus(status: RaidStatus): boolean {
  return status === 'succeeded' || status === 'limitedSuccess' || status === 'criticalSuccess';
}

function victimSourceLabel(source: RaidVictimSource | null): string {
  switch (source) {
    case 'completionCountry': return 'Country at completion';
    case 'fixedTarget': return 'Fixed target country';
    case 'controllerAtFirstEndedRead': return 'Target controller at first ended read';
    case 'controllerAtObservedRead': return 'Target controller';
    case 'captorAtFirstEndedRead': return 'Leader captor at first ended read';
    case 'captorAtObservedRead': return 'Leader captor';
    default: return 'Target country';
  }
}

export function raidTypeFilterItems(
  chips: readonly RaidTypeChip[],
  selectedSpellings: readonly string[],
  atHour: number,
): LogKindFilterItem[] {
  const visible = chips.filter(chip => chip.firstHour <= atHour);
  const baseLabels = visible.map(chip => contentTitle(chip.name, chip.spelling, 'Raid name unavailable'));
  const counts = new Map<string, number>();
  for (const label of baseLabels) counts.set(label, (counts.get(label) ?? 0) + 1);
  const seen = new Map<string, number>();
  const allSelected = selectedSpellings.length === 0;
  const selected = new Set(selectedSpellings);
  return visible.map((chip, index) => {
    const base = baseLabels[index]!;
    const ordinal = (seen.get(base) ?? 0) + 1;
    seen.set(base, ordinal);
    const label = counts.get(base)! > 1 ? `${base} (${ordinal})` : base;
    const value = setToken(chip.spelling);
    return {
      value,
      label,
      checked: allSelected || selected.has(value),
    };
  });
}

function resolvePlace(
  readView: BoundedReadView,
  row: RaidLogRow,
  boundary: ReturnType<typeof newIdentityBoundary>,
): { readonly cell: TableCell; readonly value: Value<string> | null } {
  const place = row.place;
  if (!place) {
    return { cell: { value: notRecorded(row.placeMissing ?? "The raid's target place is not recorded") }, value: null };
  }
  if (place.state !== null && place.stateHour !== null) {
    const value = resolveName(readView, 'state', String(place.state), { atHour: place.stateHour, boundary }).value;
    return { cell: { value }, value };
  }
  if (place.province !== null && place.provinceHour !== null) {
    const value = resolveName(readView, 'province', String(place.province), { atHour: place.provinceHour, boundary }).value;
    return { cell: { value }, value };
  }
  return { cell: { value: notRecorded(row.placeMissing ?? "The raid's target place is not recorded") }, value: null };
}

function preparationCell(
  row: RaidLogRow,
  hour: number,
): { readonly cell: TableCell; readonly nextChangeAt: number; readonly text: string | null } {
  const prep = row.preparation;
  if (!prep) {
    if (row.status === 'preparing' || row.status === 'prepared') {
      return {
        cell: { value: notRecorded('Preparation hours were not recorded'), missingFigure: 'Preparation' },
        nextChangeAt: Infinity,
        text: null,
      };
    }
    return { cell: { value: notApplicable('Not preparing'), blank: true }, nextChangeAt: Infinity, text: null };
  }
  const effHour = prep.through !== null ? Math.min(hour, prep.through) : hour;
  const elapsed = Math.max(0, effHour - prep.since);
  const totalHours = Math.max(0, prep.hours + elapsed * prep.perHour);
  const canAdvance = prep.perHour > 0 && (prep.through === null || hour < prep.through);
  const suffixWord = row.status === 'prepared' ? 'prepared' : 'preparing';
  if (totalHours >= 24) {
    const days = Math.floor(totalHours / 24);
    const unitSuffix = `${days === 1 ? 'day' : 'days'} ${suffixWord}`;
    const valueText = formatScalar(days, 'integer');
    let nextChangeAt = Infinity;
    if (canAdvance) {
      const targetHours = (days + 1) * 24;
      const candidate = prep.since + Math.ceil((targetHours - prep.hours) / prep.perHour);
      if (candidate > hour && (prep.through === null || candidate <= prep.through)) nextChangeAt = candidate;
    }
    return {
      cell: { value: knownValue(valueText), unitSuffix },
      nextChangeAt,
      text: `${valueText} ${unitSuffix}`,
    };
  }
  const unitSuffix = `${totalHours === 1 ? 'hour' : 'hours'} ${suffixWord}`;
  const valueText = formatScalar(totalHours, 'integer');
  let nextChangeAt = Infinity;
  if (canAdvance) {
    const candidate = prep.since + Math.ceil((totalHours + 1 - prep.hours) / prep.perHour);
    if (candidate > hour && (prep.through === null || candidate <= prep.through)) nextChangeAt = candidate;
  }
  return {
    cell: { value: knownValue(valueText), unitSuffix },
    nextChangeAt,
    text: `${valueText} ${unitSuffix}`,
  };
}

function buildStatusCell(row: RaidLogRow): { readonly cell: TableCell; readonly label: string; readonly muted: boolean } {
  const info = RAID_STATUS_INFO[row.status];
  if (info.notRecorded) {
    const reason = row.status === 'outcomeNotRecorded'
      ? (row.outcomeMissing ?? 'Raid outcome was not recorded')
      : row.status === 'notRead'
        ? 'Raids were not recorded at this hour'
        : (row.phaseName ?? 'Raid phase was not recorded');
    const missingFigure = row.status === 'outcomeNotRecorded' ? 'Outcome' : row.status === 'notRead' ? 'Status' : 'Phase';
    return {
      cell: { value: notRecorded(reason), missingFigure },
      label: info.label,
      muted: true,
    };
  }
  const label = info.label;
  return {
    cell: logStatusCell(label, info.mark!, info.muted ? 'muted' : undefined),
    label,
    muted: info.muted === true,
  };
}

function buildRaidRow(
  readView: BoundedReadView,
  row: RaidLogRow,
  hour: number,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
): { readonly row: TableRow; readonly nextChangeAt: number } {
  const shownHour = row.end !== null
    ? row.end.endedHour
    : row.disappearedHour !== null
      ? row.disappearedHour
      : row.hour;

  const raiderEntry = row.end !== null
    ? {
        tag: row.end.raider,
        atHour: row.end.endedHour,
        missingLabel: "The raid's raider is not recorded",
      }
    : {
        tag: row.raider.tag,
        atHour: row.hour,
        missingLabel: row.raider.missing ?? "The raid's raider is not recorded",
      };
  const victimEntry = {
    tag: row.victim.tag,
    atHour: row.victim.hour,
    dimmed: isSuccessStatus(row.status),
    missingLabel: row.victim.missing ?? "The raid's target is not recorded",
  };
  const countryCell = logCountryCell(readView, [raiderEntry, victimEntry], shownHour, boundary);

  const itemTitle = row.raidType !== null
    ? contentTitle(row.raidTypeName, row.raidType, 'Raid name unavailable')
    : 'Raid type not recorded';
  const itemCell: TableCell = row.raidType !== null
    ? { value: knownValue(itemTitle) }
    : { value: notRecorded(row.typeMissing ?? "The raid's type is not recorded") };

  const artUrl = artwork ? drawableIcon(row.icon) : null;
  const artHandle = urlArtHandle(artUrl);
  const kindCell: TableCell = artwork
    ? artHandle
      ? {
          value: row.raidType !== null ? knownValue(itemTitle) : notRecorded(row.typeMissing ?? 'Raid icon not recorded'),
          iconSlot: true,
          artShape: 'square',
          artName: itemTitle,
          ...cellArt(readView, artHandle, row.iconMissing ?? 'Raid icon not recorded'),
        }
      : {
          value: notRecorded(row.iconMissing ?? 'Raid icon not recorded'),
          iconSlot: true,
          artShape: 'square',
          artName: itemTitle,
        }
    : { value: notApplicable('Game artwork hidden'), blank: true };

  const place = resolvePlace(readView, row, boundary);
  const prep = preparationCell(row, hour);
  const status = buildStatusCell(row);

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Status', value: knownValue(status.label) },
      { label: victimSourceLabel(row.victim.source), value: row.victim.tag !== null ? knownValue(row.victim.tag) : notRecorded(row.victim.missing ?? "The raid's target is not recorded") },
    ];
    if (place.value && place.value.kind !== 'notRecorded') out.push({ label: 'Target', value: place.value });
    else if (row.placeMissing) out.push({ label: 'Target', value: notRecorded(row.placeMissing) });
    if (prep.text) out.push({ label: 'Preparation', value: knownValue(prep.text) });
    if (row.outcomeMissing) out.push({ label: 'Outcome', value: notRecorded(row.outcomeMissing) });
    out.push({ label: 'First seen', value: knownValue(formatDateTime(row.hour)) });
    if (row.launchedHour !== null) {
      out.push({ label: 'Launched', value: knownValue(formatDateTime(row.launchedHour)) });
    }
    if (row.end !== null) {
      out.push({ label: 'Ended', value: knownValue(formatDateTime(row.end.endedHour)) });
    } else if (row.disappearedHour !== null) {
      out.push({
        label: 'No longer seen',
        value: knownValue(formatDateTime(row.disappearedHour)),
      });
    } else if (row.lastSeen !== null) {
      out.push({ label: 'Last seen', value: knownValue(formatDateTime(row.lastSeen)) });
    }
    if (row.raidType) out.push({ label: 'Raid type', value: knownValue(row.raidType) });
    return out;
  };

  return {
    nextChangeAt: prep.nextChangeAt,
    row: {
      key: row.key,
      name: itemTitle,
      rowHour: shownHour,
      live: row.live,
      muted: status.muted,
      cells: {
        date: logDateCell(shownHour),
        kind: kindCell,
        country: countryCell,
        item: itemCell,
        place: place.cell,
        preparation: prep.cell,
        status: status.cell,
      },
      tooltipTitle: itemTitle,
      tooltipExtra,
    },
  };
}

export const raidLogProjection: TableProjection<RaidLogProjectionOptions> = registerFamilyProjection({
  kind: 'raid-log',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: RAID_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused };
    }
    const source = readView.rows<RaidLogRow & DatedLogRow>('main');
    if (!source) {
      return { columns: RAID_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true };
    }
    if (source.refused !== null) {
      return { columns: RAID_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused: source.refused };
    }
    const boundary = newIdentityBoundary();
    let nextPrepChange = Infinity;
    const readAt = source.at ?? hour;
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawRow => {
        const built = buildRaidRow(readView, rawRow, readAt, options.artwork, boundary);
        if (!source.frozen && built.nextChangeAt > hour && built.nextChangeAt < nextPrepChange) nextPrepChange = built.nextChangeAt;
        return built.row;
      },
    );

    const rawHead = source.rawHead as {
      readonly unavailable?: string | null;
      readonly warnings?: readonly string[];
      readonly raidTypes?: readonly RaidTypeChip[];
    } | undefined;
    const activeCount = source.facetTotals?.get('active') ?? null;
    const titleSummary = activeCount !== null && activeCount > 0
      ? `${formatScalar(activeCount, 'integer')} in preparation`
      : null;
    const warnings = rawHead?.warnings?.length ? [...rawHead.warnings] : undefined;
    const unavailable = typeof rawHead?.unavailable === 'string' && rawHead.unavailable.length > 0
      ? rawHead.unavailable
      : null;

    return {
      columns: RAID_LOG_COLUMNS,
      rows,
      slots,
      hour,
      hideHeader: true,
      unsorted: true,
      dateGrouped: true,
      windowOffset,
      totalRows,
      ...(gapBefore?.length ? { gapBefore } : {}),
      nextChangeAt: Math.min(boundary.next, nextPrepChange),
      rowTimeBoundaries: false,
      kindFilter: { key: 'types', noun: 'raid types', allLabel: 'All raid types', items: raidTypeFilterItems(rawHead?.raidTypes ?? [], options.selected, Math.min(hour, source.at ?? hour)), selected: options.selected },
      titleSummary,
      ...(source.missing.length > 0 && titleSummary ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      ...(warnings ? { warnings } : {}),
      ...(slots.length === 0
        ? unavailable
          ? { empty: 'No raid types available.', emptyDetail: unavailable, emptyKind: 'unavailable' as const }
          : { empty: 'No recorded raids match these settings.' }
        : {}),
    };
  },
});

export const raidLogDeclaration: TableWidgetDeclaration<RaidLogProjectionOptions> = {
  family: 'table',
  kind: 'raid-log',
  label: 'Raid log',
  subject: 'raids',
  emptyCampaign: 'Load a campaign to see its raid log.',
  icon: 'target',
  countries: { mode: 'filter', content: 'history' },
  columns: RAID_LOG_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const selectedTypes = contentSet(ctx.options.types ?? '').map(contentKey);
    const selection = occurrenceSelection(ctx);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:raid-log:${selectedTypes.join(',')}:${selection ? JSON.stringify(selection) : 'all'}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'raid-log-tail',
            branchId: ctx.branch.id,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ask: { ...(selectedTypes.length ? { raidTypes: selectedTypes } : {}), ...selection },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
        ),
      ],
    };
  },
  projection: raidLogProjection,
  projectionOptions(ctx): RaidLogProjectionOptions {
    return { artwork: ctx.options.artwork !== false, selected: contentSet(ctx.options.types ?? '') };
  },
};
