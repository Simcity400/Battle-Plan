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
import type { EspionageLogRow, EspionageStatus, EspionageTypeChip } from '../../generated/dashboard';
import { ESPIONAGE_LOG_COLUMNS } from './definition.ts';

export interface EspionageLogProjectionOptions {
  readonly artwork: boolean;
}

const HEAD_LIMIT = 256;
const HOUR_EPOCH = 0;

function gameDay(hour: number): number {
  return Math.floor((hour - HOUR_EPOCH) / 24);
}

const STATUS_INFO: Readonly<Record<EspionageStatus, { readonly mark?: StatusMarkKind; readonly label: string; readonly muted?: boolean; readonly notRecorded?: boolean }>> = {
  running: { mark: 'running', label: 'Running' },
  preparing: { mark: 'running', label: 'Preparing' },
  completed: { mark: 'succeeded', label: 'Completed' },
  ended: { mark: 'ended', label: 'Ended', muted: true },
  outcomeNotRecorded: { label: 'Outcome not recorded', muted: false, notRecorded: true },
  notRead: { label: 'Not recorded', muted: true, notRecorded: true },
};

export function espionageTypeFilterItems(
  chips: readonly EspionageTypeChip[],
  selectedTokens: readonly string[],
  atHour?: number,
): LogKindFilterItem[] {
  const visible = atHour === undefined ? chips : chips.filter(chip => chip.firstHour <= atHour);
  const baseLabels = visible.map(chip => contentTitle(chip.name, chip.token, 'Operation name unavailable'));
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

function progressCell(
  row: EspionageLogRow,
  hour: number,
): { readonly cell: TableCell; readonly nextChangeAt: number; readonly text: string | null } {
  if (row.status === 'running') {
    if (row.startHour !== null && row.durationDays !== null && row.durationDays > 0) {
      const elapsed = Math.max(0, gameDay(hour) - gameDay(row.startHour));
      const remaining = Math.max(0, row.durationDays - elapsed);
      const fraction = Math.max(0, Math.min(1, elapsed / row.durationDays));
      const valueText = formatScalar(remaining, 'integer');
      const unitSuffix = remaining === 1 ? 'day left' : 'days left';
      const nextDay = HOUR_EPOCH + (gameDay(hour) + 1) * 24;
      return {
        cell: {
          value: knownValue(valueText),
          unitSuffix,
          bar: { fraction: knownValue(fraction) },
        },
        nextChangeAt: nextDay > hour ? nextDay : Infinity,
        text: `${elapsed} / ${row.durationDays} days`,
      };
    }
    return {
      cell: { value: notRecorded('Execution progress was not recorded'), missingFigure: 'Progress' },
      nextChangeAt: Infinity,
      text: null,
    };
  }

  if (row.status === 'preparing') {
    if (row.preparationRatio !== null) {
      const pct = Math.trunc(row.preparationRatio / 1000);
      const valueText = `${pct}%`;
      return {
        cell: { value: knownValue(valueText), unitSuffix: 'prepared' },
        nextChangeAt: Infinity,
        text: `${valueText} prepared`,
      };
    }
    return {
      cell: { value: notRecorded('Preparation was not recorded'), missingFigure: 'Preparation' },
      nextChangeAt: Infinity,
      text: null,
    };
  }

  if (row.durationDays !== null && row.durationDays > 0) {
    const valueText = formatScalar(row.durationDays, 'integer');
    const unitSuffix = row.durationDays === 1 ? 'day' : 'days';
    return {
      cell: { value: knownValue(valueText), unitSuffix },
      nextChangeAt: Infinity,
      text: `${valueText} ${unitSuffix}`,
    };
  }

  if (row.startHour !== null && row.endedHour !== null && row.endedHour >= row.startHour) {
    const days = gameDay(row.endedHour) - gameDay(row.startHour);
    if (days > 0) {
      const valueText = formatScalar(days, 'integer');
      const unitSuffix = days === 1 ? 'day' : 'days';
      return {
        cell: { value: knownValue(valueText), unitSuffix },
        nextChangeAt: Infinity,
        text: `${valueText} ${unitSuffix}`,
      };
    }
  }

  if (row.status === 'ended' && row.startHour === null) {
    return {
      cell: { value: notApplicable('Ended before starting execution'), blank: true },
      nextChangeAt: Infinity,
      text: null,
    };
  }

  return {
    cell: { value: notRecorded('Operation length was not recorded'), missingFigure: 'Length' },
    nextChangeAt: Infinity,
    text: null,
  };
}

function buildStatusCell(row: EspionageLogRow): { readonly cell: TableCell; readonly label: string; readonly muted: boolean } {
  const info = STATUS_INFO[row.status];
  if (info.notRecorded) {
    return {
      cell: {
        value: notRecorded(row.outcomeMissing ?? 'Operation outcome was not recorded'),
        missingFigure: row.status === 'outcomeNotRecorded' ? 'Outcome' : 'Status',
      },
      label: info.label,
      muted: info.muted === true,
    };
  }
  return {
    cell: logStatusCell(info.label, info.mark!, info.muted ? 'muted' : undefined),
    label: info.label,
    muted: info.muted === true,
  };
}

function buildEspionageRow(
  readView: BoundedReadView,
  row: EspionageLogRow,
  hour: number,
  artwork: boolean,
  boundary: ReturnType<typeof newIdentityBoundary>,
): { readonly row: TableRow; readonly nextChangeAt: number } {
  const identityHour = row.live ? hour : (row.endedHour ?? row.lastSeen ?? row.hour);
  const countryCell = logCountryCell(
    readView,
    [
      {
        tag: row.owner.tag,
        atHour: identityHour,
        missingLabel: row.owner.missing ?? "The operation's owner is not recorded",
      },
      {
        tag: row.target.tag,
        atHour: identityHour,
        missingLabel: row.target.missing ?? "The operation's target is not recorded",
      },
    ],
    identityHour,
    boundary,
  );

  const hasName = row.operationName !== null || row.operationToken !== null;
  const itemTitle = hasName
    ? contentTitle(row.operationName, row.operationToken, 'Operation name unavailable')
    : 'Operation not recorded';
  const itemCell: TableCell = hasName
    ? { value: knownValue(itemTitle) }
    : { value: notRecorded(row.operationMissing ?? 'Operation type was not recorded') };

  const artUrl = artwork ? drawableIcon(row.icon) : null;
  const artHandle = urlArtHandle(artUrl);
  const kindCell: TableCell = artwork
    ? artHandle
      ? {
          value: hasName ? knownValue(itemTitle) : notRecorded(row.iconMissing ?? 'Operation artwork not recorded'),
          iconSlot: true,
          artShape: 'wide',
          artName: itemTitle,
          ...cellArt(readView, artHandle, row.iconMissing ?? 'Operation artwork not recorded'),
        }
      : {
          value: notRecorded(row.iconMissing ?? row.operationMissing ?? 'Operation artwork not recorded'),
          iconSlot: true,
          artShape: 'wide',
          artName: itemTitle,
        }
    : { value: notApplicable('Game artwork hidden'), blank: true };

  const progress = progressCell(row, hour);
  const status = buildStatusCell(row);

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Status', value: row.outcomeMissing ? notRecorded(row.outcomeMissing) : knownValue(status.label) },
    ];
    if (progress.text) out.push({ label: row.live ? 'Progress' : 'Duration', value: knownValue(progress.text) });
    if (row.firstSeen !== null) out.push({ label: 'First seen', value: knownValue(formatDateTime(row.firstSeen)) });
    if (row.startHour !== null) out.push({ label: 'Execution started', value: knownValue(formatDateTime(row.startHour)) });
    if (row.endedHour !== null) {
      out.push({
        label: row.status === 'completed' ? 'Completed' : 'Ended',
        value: knownValue(row.exact ? formatDateTime(row.endedHour) : `by ${formatDateTime(row.endedHour)}`),
      });
    }
    if (row.completion) {
      const c = row.completion;
      if (c.description !== null) out.push({ label: 'Outcome', value: knownValue(c.description) });
      else if (c.descriptionUnrecorded !== null) out.push({ label: 'Outcome', value: notRecorded(c.descriptionUnrecorded) });
      if (c.bonusRefused !== null) out.push({ label: 'Bonus result', value: notRecorded(c.bonusRefused) });
      else if (c.bonus !== null) out.push({ label: 'Bonus result', value: knownValue(c.bonus ? 'Yes' : 'No') });
      if (c.detectedRefused !== null) out.push({ label: 'Detected', value: notRecorded(c.detectedRefused) });
      else if (c.detected !== null) out.push({ label: 'Detected', value: knownValue(c.detected ? 'Yes' : 'No') });
    }
    if (row.phaseCount !== null && row.phaseCount > 0) {
      out.push({
        label: 'Phases',
        value: knownValue(
          row.currentPhase !== null
            ? `Phase ${row.currentPhase + 1} of ${row.phaseCount}`
            : `${row.phaseCount} phases`,
        ),
      });
    }
    if (row.assignedSlots !== null) {
      out.push({ label: 'Operative slots', value: knownValue(formatScalar(row.assignedSlots, 'integer')) });
    }
    if (row.autoCommence !== null) {
      out.push({ label: 'Auto-start', value: knownValue(row.autoCommence ? 'Enabled' : 'Disabled') });
    }
    if (row.autoRepeat !== null) {
      out.push({ label: 'Auto-repeat', value: knownValue(row.autoRepeat ? 'Enabled' : 'Disabled') });
    }
    return out;
  };

  return {
    nextChangeAt: progress.nextChangeAt,
    row: {
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
        progress: progress.cell,
        status: status.cell,
      },
      tooltipTitle: itemTitle,
      tooltipExtra,
    },
  };
}

export const espionageLogProjection: TableProjection<EspionageLogProjectionOptions> = registerFamilyProjection({
  kind: 'espionage-log',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: ESPIONAGE_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused };
    }
    const source = readView.rows<EspionageLogRow & DatedLogRow>('main');
    if (!source) {
      return { columns: ESPIONAGE_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true };
    }
    if (source.refused !== null) {
      return { columns: ESPIONAGE_LOG_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused: source.refused };
    }
    const boundary = newIdentityBoundary();
    let nextProgressChange = Infinity;
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      rawRow => {
        const built = buildEspionageRow(readView, rawRow, hour, options.artwork, boundary);
        if (built.nextChangeAt > hour && built.nextChangeAt < nextProgressChange) nextProgressChange = built.nextChangeAt;
        return built.row;
      },
    );
    const activeCount = source.facetTotals?.get('active') ?? null;
    const titleSummary = activeCount !== null && activeCount > 0
      ? `${formatScalar(activeCount, 'integer')} under way`
      : null;
    const rawHead = source.rawHead as {
      readonly unavailable?: string | null;
      readonly warnings?: readonly string[];
    } | undefined;
    const warnings = rawHead?.warnings?.length ? [...rawHead.warnings] : undefined;
    const unavailable = typeof rawHead?.unavailable === 'string' && rawHead.unavailable.length > 0
      ? rawHead.unavailable
      : null;
    const nextChangeAt = Math.min(boundary.next, nextProgressChange);

    return {
      columns: ESPIONAGE_LOG_COLUMNS,
      rows,
      slots,
      hour,
      hideHeader: true,
      unsorted: true,
      dateGrouped: true,
      windowOffset,
      totalRows,
      ...(gapBefore?.length ? { gapBefore } : {}),
      nextChangeAt,
      rowTimeBoundaries: false,
      titleSummary,
      ...(source.missing.length > 0 && titleSummary ? { titleSummaryPartial: 'Count from recorded hours' } : {}),
      ...(warnings ? { warnings } : {}),
      ...(slots.length === 0
        ? unavailable
          ? { empty: 'Espionage operations were not recorded.', emptyDetail: unavailable, emptyKind: 'unavailable' as const }
          : { empty: 'No recorded espionage operations match these settings.' }
        : {}),
    };
  },
});

export const espionageLogDeclaration: TableWidgetDeclaration<EspionageLogProjectionOptions> = {
  family: 'table',
  kind: 'espionage-log',
  label: 'Espionage operations',
  subject: 'espionage operations',
  emptyCampaign: 'Load a campaign to see its espionage operations.',
  icon: 'eye',
  countries: { mode: 'filter' },
  columns: ESPIONAGE_LOG_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const selectedTypes = contentSet(ctx.options.types ?? '').map(contentKey);
    const countryRole = ctx.options.countryRole === 'target' || ctx.options.countryRole === 'either' ? ctx.options.countryRole : 'owner';
    const runningOrder = ctx.options.runningOrder === 'preparation' ? 'preparation' : 'start';
    const selection = occurrenceSelection(ctx);
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:espionage-log:${selectedTypes.join(',')}:${countryRole}:${runningOrder}:${selection ? JSON.stringify(selection) : 'all'}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'espionage-log-tail',
            branchId: ctx.branch.id,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ask: {
              ...(selectedTypes.length ? { operationTypes: selectedTypes } : {}),
              ...(countryRole !== 'owner' ? { countryRole } : {}),
              ...(runningOrder !== 'start' ? { runningOrder } : {}),
              ...selection,
            },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
        ),
      ],
    };
  },
  projection: espionageLogProjection,
  projectionOptions(ctx): EspionageLogProjectionOptions {
    return { artwork: ctx.options.artwork !== false };
  },
};
