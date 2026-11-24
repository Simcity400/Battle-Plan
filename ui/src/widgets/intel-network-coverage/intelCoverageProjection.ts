import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { IntelCoverageBasis, IntelCoverageMeasure, IntelCoverageReply } from '../../generated/dashboard';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import {
  registerHeatMatrixProjection,
  resolveEntityFromReadView,
  sequentialRampColour,
  type HeatMatrixModel,
  type HeatMatrixProjection,
  type HeatMatrixWidgetDeclaration,
  type MatrixAxisEntity,
  type MatrixCell,
} from '../../families/heat/index.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import { OPTIONS } from './definition.ts';

export interface IntelCoverageProjectionOptions {
  readonly measure: IntelCoverageMeasure;
  readonly basis: IntelCoverageBasis;
  readonly surface: string;
  readonly grey: string;
  readonly accent: string;
}

function formatCoverageValue(raw: bigint, scale: bigint, measure: IntelCoverageMeasure): string {
  if (measure === 'covered' || scale <= 1n) return raw.toLocaleString('en-US');
  const num = Number(raw) / Number(scale);
  return `${Math.round(num)}%`;
}

export function projectIntelCoverage(
  readView: BoundedReadView,
  options: IntelCoverageProjectionOptions,
): HeatMatrixModel {
  const refused = readView.refused('main');
  if (refused) {
    return { hour: readView.hour, rows: [], columns: [], cell: () => ({ kind: 'diagonal' }), refused };
  }
  const reply = readView.panel<IntelCoverageReply>('main');
  if (!reply) {
    return { hour: readView.hour, rows: [], columns: [], cell: () => ({ kind: 'diagonal' }), empty: 'No intel networks recorded at this date.' };
  }
  if (reply.missing) {
    return {
      hour: readView.hour,
      rows: [],
      columns: [],
      cell: () => ({ kind: 'diagonal' }),
      empty: 'Intel networks were not recorded at this time.',
      emptyDetail: reply.missing,
      emptyKind: 'unavailable',
      warnings: reply.warnings,
    };
  }
  const displayed = reply.displayed ?? [];
  if (displayed.length < 2) {
    return {
      hour: readView.hour,
      rows: [],
      columns: [],
      cell: () => ({ kind: 'diagonal' }),
      empty: displayed.length === 1 ? 'No intel networks of or in this country at this date.' : 'Fewer than two countries are shown at this date.',
      warnings: reply.warnings,
    };
  }

  const entities: MatrixAxisEntity[] = displayed.map(country => {
    const tag = country.tag ?? `#${country.id}`;
    const resolved = country.tag
      ? resolveEntityFromReadView(readView, 'country', country.tag, readView.hour, tag, null, options.surface)
      : { label: tag };
    return {
      id: tag,
      label: resolved.label,
      shortLabel: tag,
      tag: country.tag ?? undefined,
      ...(resolved.nameMissing ? { nameMissing: resolved.nameMissing } : {}),
      flag: resolved.flag ?? null,
      flagPending: resolved.pending === true,
      meta: { countryId: country.id },
    };
  });

  const n = displayed.length;
  const cells = reply.cells ?? [];
  const maxVal = Number(reply.maxValue ?? 0n);
  const scale = reply.scale ?? 1n;
  let hasMissing = false;
  for (let i = 0; i < cells.length; i++) {
    if (cells[i]?.kind === 'missing') {
      hasMissing = true;
      break;
    }
  }

  const cell = (rowIndex: number, columnIndex: number): MatrixCell => {
    if (rowIndex === columnIndex) return { kind: 'diagonal' };
    const raw = cells[rowIndex * n + columnIndex];
    if (!raw) return { kind: 'missing', reason: 'Intel network not recorded' };
    if (raw.kind === 'diagonal') return { kind: 'diagonal' };
    if (raw.kind === 'noNetwork') return { kind: 'empty', label: 'No network', selectable: false };
    if (raw.kind === 'missing') return { kind: 'missing', reason: raw.reason };
    const text = formatCoverageValue(raw.value, scale, options.measure);
    const color = sequentialRampColour(Number(raw.value), Math.max(1, maxVal), options.grey, options.accent);
    return { kind: 'value', color, text, value: raw.value, selectable: true };
  };

  return {
    hour: readView.hour,
    rows: entities,
    columns: entities,
    cell,
    legend: {
      gradient: {
        lowLabel: '0',
        highLabel: formatCoverageValue(reply.maxValue ?? 0n, scale, options.measure),
        stops: [options.grey, options.accent],
      },
      entries: [
        { id: 'empty', label: 'No network', color: options.grey },
        ...(hasMissing ? [{ id: 'missing', label: 'Not recorded', color: options.grey, hatched: true }] : []),
      ],
    },
    tooltip: (row, col, item, rowIndex, colIndex) => {
      if (item.kind === 'diagonal') return null;
      const readNote = reply.readHour !== null ? `Networks read ${formatDateOnly(reply.readHour)}` : undefined;
      if (item.kind === 'missing') {
        return {
          title: `${row.label} → ${col.label}`,
          hour: readView.hour,
          missingReason: item.reason,
          ...(readNote ? { note: readNote } : {}),
        };
      }
      if (item.kind === 'empty') {
        return {
          title: `${row.label} → ${col.label}`,
          hour: readView.hour,
          items: [{ label: 'No intel network', tone: 'muted' }],
          ...(readNote ? { note: readNote } : {}),
        };
      }
      const raw = cells[rowIndex * n + colIndex];
      const items = raw?.kind === 'present'
        ? [
            { label: 'Controlled states covered', value: String(raw.covered_controlled) },
            { label: 'Core states covered', value: String(raw.covered_core) },
            { label: 'Controlled strength sum', value: `${Math.round(Number(raw.controlled_strength) / 100_000)}%` },
            { label: 'Strongest state', value: `${Math.round(Number(raw.max_strength) / 100_000)}%` },
          ]
        : [];
      return {
        title: `${row.label} network in ${col.label}`,
        hour: readView.hour,
        exactValue: item.kind === 'value' ? item.text : undefined,
        items,
        ...(readNote ? { note: readNote } : {}),
      };
    },
    warnings: reply.warnings,
  };
}

export const INTEL_COVERAGE_PROJECTION: HeatMatrixProjection<IntelCoverageProjectionOptions> = {
  kind: 'intel-network-coverage',
  family: 'heat-matrix',
  project: projectIntelCoverage,
};

registerHeatMatrixProjection(INTEL_COVERAGE_PROJECTION);

export const INTEL_COVERAGE_DECLARATION: HeatMatrixWidgetDeclaration<IntelCoverageProjectionOptions> = {
  family: 'heat-matrix',
  kind: 'intel-network-coverage',
  label: 'Intel network coverage',
  subject: 'intel networks',
  emptyCampaign: 'Load a campaign to see intel networks.',
  icon: 'eye',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: false, colours: false, factions: false },
  sources: (ctx): SourcePlan => {
    const measure: IntelCoverageMeasure = ctx.options.measure === 'strength' || ctx.options.measure === 'strongest' ? ctx.options.measure : 'covered';
    const basis: IntelCoverageBasis = ctx.options.basis === 'core' ? 'core' : 'controlled';
    const isAll = ctx.countries.all;
    const isTop = ctx.countries.top !== null;
    const mostCountries = Math.max(2, Number(ctx.options.mostCountries) || 12);
    const explicitTags = isAll || isTop ? [] : ctx.countries.tags;
    const top = isAll ? mostCountries : (isTop ? (ctx.countries.top ?? mostCountries) : undefined);
    const touching = explicitTags.length === 1 || top === 1;
    const baseParams: Record<string, unknown> = {
      branchId: ctx.branch.id,
      ...(explicitTags.length ? { countries: explicitTags.map(countryFromKey) } : {}),
      ...(top !== undefined ? { top } : {}),
      touching,
      measure,
      basis,
    };
    return {
      owner: `intel-network-coverage:${ctx.branch.id}:${measure}:${basis}:${top ?? explicitTags.join(',')}`,
      entries: [
        {
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: explicitTags.length
              ? { mode: 'keyed', keys: explicitTags }
              : { mode: 'population', form: 'top', limit: top ?? mostCountries },
            product: 'intel-network-coverage',
            followQuery: { product: 'intel-network-coverage', params: baseParams },
            params: baseParams,
          },
        },
      ],
    };
  },
  projection: INTEL_COVERAGE_PROJECTION,
  projectionOptions: ctx => ({
    measure: ctx.options.measure === 'strength' || ctx.options.measure === 'strongest' ? ctx.options.measure : 'covered',
    basis: ctx.options.basis === 'core' ? 'core' : 'controlled',
    surface: '#11161f',
    grey: '#2f3847',
    accent: '#4f86f7',
  }),
  jumpToDate: true,
};
