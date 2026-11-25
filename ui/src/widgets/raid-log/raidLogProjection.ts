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
import type { RaidLogRow, RaidStatus, RaidTypeChip, RaidVictimSource } from '../../generated/dashboard';
import { RAID_LOG_COLUMNS } from './definition.ts';

export interface RaidLogProjectionOptions {
  readonly artwork: boolean;
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
  notRead: { mark: 'ended', label: 'Not read', muted: true },
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
  atHour?: number,
): LogKindFilterItem[] {
  const visible = atHour === undefined ? chips : chips.filter(chip => chip.firstHour <= atHour);
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
    return {
      value: chip.spelling,
      label,
      checked: allSelected || selected.has(chip.spelling),
    };
  });
}

function resolvePlace(readView: BoundedReadView, row: RaidLogRow): { readonly cell: TableCell; readonly label: string | null } {
  const place = row.place;
  if (!place) {
    return { cell: { value: notRecorded(row.placeMissing ?? "The raid's target place is not recorded") }, label: null };
  }
  if (place.state !== null && place.stateHour !== null) {
    const res = readView.who('state', String(place.state), place.stateHour);
    if (res.name.status === 'ready' && res.name.value.trim()) {
      return { cell: { value: knownValue(res.name.value) }, label: res.name.value };
    }
    if (res.name.status === 'pending') {
      const label = `State ${place.state}`;
      return { cell: { value: knownValue(label) }, label };
    }
    return { cell: { value: notRecorded(res.name.status === 'ready' ? 'State name not recorded' : res.name.reason) }, label: null };
  }
  if (place.province !== null && place.provinceHour !== null) {
    const res = readView.who('province', String(place.province), place.provinceHour);
    if (res.name.status === 'ready' && res.name.value.trim()) {
      return { cell: { value: knownValue(res.name.value) }, label: res.name.value };
    }
    if (res.name.status === 'pending') {
      const label = `Province ${place.province}`;
      return { cell: { value: knownValue(label) }, label };
    }
    return { cell: { value: notRecorded(res.name.status === 'ready' ? 'Province name not recorded' : res.name.reason) }, label: null };
  }
  return { cell: { value: notRecorded(row.placeMissing ?? "The raid's target place is not recorded") }, label: null };
}

function preparationCell(
  row: RaidLogRow,
  hour: number,
): { readonly cell: TableCell; readonly nextChangeAt: number; readonly text: string | null } {
  const prep = row.preparation;
  if (!prep) {
    if (row.status === 'preparing') {
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
      : (row.phaseName ?? 'Raid phase was not recorded');
    const missingFigure = row.status === 'outcomeNotRecorded' ? 'Outcome' : 'Phase';
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

  const place = resolvePlace(readView, row);
  const prep = preparationCell(row, hour);
  const status = buildStatusCell(row);

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Status', value: knownValue(status.label) },
      { label: victimSourceLabel(row.victim.source), value: row.victim.tag !== null ? knownValue(row.victim.tag) : notRecorded(row.victim.missing ?? "The raid's target is not recorded") },
    ];
    if (place.label) out.push({ label: 'Target', value: knownValue(place.label) });
    else if (row.placeMissing) out.push({ label: 'Target', value: notRecorded(row.placeMissing) });
    if (prep.text) out.push({ label: 'Preparation', value: knownValue(prep.text) });
    if (row.outcomeMissing) out.push({ label: 'Outcome', value: notRecorded(row.outcomeMissing) });
    out.push({ label: row.exact ? 'First seen' : 'First seen by', value: knownValue(formatDateTime(row.hour)) });
    if (row.launchedHour !== null) {
      out.push({ label: row.launchedExact ? 'Launched' : 'Launched by', value: knownValue(formatDateTime(row.launchedHour)) });
    }
    if (row.end !== null) {
      const endLabel = row.end.source === 'nativeEndDate'
        ? 'Ended (recorded)'
        : row.end.exact ? 'Ended (observed)' : 'Ended by (observed)';
      out.push({ label: endLabel, value: knownValue(formatDateTime(row.end.endedHour)) });
    } else if (row.disappearedHour !== null) {
      out.push({
        label: row.disappearedExact ? 'No longer seen' : 'No longer seen by',
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
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawRow => {
        const built = buildRaidRow(readView, rawRow, hour, options.artwork, boundary);
        if (built.nextChangeAt > hour && built.nextChangeAt < nextPrepChange) nextPrepChange = built.nextChangeAt;
        return built.row;
      },
    );

    const rawHead = source.rawHead as {
      readonly unavailable?: string | null;
      readonly warnings?: readonly string[];
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
  countries: { mode: 'filter' },
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
    return { artwork: ctx.options.artwork !== false };
  },
};
