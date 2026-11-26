import { unavailableArt } from '../../families/art/types.ts';
import { measureColour, surfaceGrey } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import { NO_CELLS, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity, type MatrixCell } from '../../families/heat/matrixModel.ts';
import { labelsAtReadings, resolveEntityFromReadView, sequentialRampColour } from '../../families/heat/scale.ts';
import { formatRational, formatScalar } from '../../families/valueFormat.ts';
import type { IntelCoverageBasis, IntelCoverageMeasure, IntelCoverageReply, IntelPair } from '../../generated/dashboard';

export const INTEL_TOP_DEFAULT = 12;
const FIXED = 100_000;

export interface IntelCoverageProjectionOptions {
  readonly measure: IntelCoverageMeasure;
  readonly surface: string;
}

export function coverageText(raw: number | bigint, scale: number | bigint, measure: IntelCoverageMeasure): string {
  if (measure === 'covered') return formatScalar(raw, 'integer');
  return formatRational(BigInt(raw) * 100n, BigInt(scale), { maxDecimals: 5, percent: true });
}

function emptyModel(hour: number, extra: Partial<HeatMatrixModel>): HeatMatrixModel {
  return { hour, rows: [], columns: [], cells: NO_CELLS, ...extra };
}

export function projectIntelCoverage(readView: BoundedReadView, options: IntelCoverageProjectionOptions): HeatMatrixModel {
  const hour: number = readView.hour;
  const refused = readView.refused('coverage');
  if (refused) return emptyModel(hour, { refused });
  const reply = readView.panel<IntelCoverageReply>('coverage');
  if (!reply || !options.surface) return emptyModel(hour, {});
  if (reply.missing) return emptyModel(hour, { empty: 'Intel networks were not recorded at this date.', emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
  if (reply.chosenMissing) return emptyModel(hour, { empty: 'The countries to show are not known at this date.', emptyDetail: reply.chosenMissing, emptyKind: 'unavailable', warnings: reply.warnings });
  const displayed = reply.displayed;
  if (displayed.length < 2) {
    return emptyModel(hour, { empty: displayed.length === 1 ? 'No intel networks of or in this country at this date.' : 'Fewer than two countries are shown at this date.', warnings: reply.warnings });
  }
  const grey = surfaceGrey(options.surface, 0.08);
  const accent = measureColour('intelCoverage', options.surface);
  const readHour = reply.readHour ?? hour;
  const shown = displayed.map(country => {
    const tag = country.tag ?? null;
    return { tag, resolved: tag ? resolveEntityFromReadView(readView, 'country', tag, hour, tag, null, options.surface) : { label: 'Country not recorded', flag: unavailableArt('The country has no tag at this date'), validFrom: -Infinity } };
  });
  const labelAt = labelsAtReadings(readView, shown.map(({ tag, resolved }) => ({ tag, label: resolved.label, validFrom: resolved.validFrom })), [readHour]);
  const entities: MatrixAxisEntity[] = displayed.map((country, index) => {
    const { tag, resolved } = shown[index]!;
    return {
      id: tag ?? `#${country.id}`,
      label: resolved.label,
      shortLabel: tag ?? '?',
      ...(tag ? { tag } : {}),
      ...('nameMissing' in resolved && resolved.nameMissing ? { nameMissing: resolved.nameMissing } : {}),
      ...(resolved.flag ? { flag: resolved.flag } : {}),
      meta: { countryId: country.id },
    };
  });
  const n = displayed.length;
  const highest = Math.max(1, Number(reply.maxValue));
  const present = new Map<number, MatrixCell>();
  const networks = new Map<number, IntelPair>();
  for (const pair of reply.pairs) {
    const at = pair.row * n + pair.column;
    networks.set(at, pair);
    present.set(at, { kind: 'value', color: sequentialRampColour(Number(pair.value), highest, grey, accent), text: coverageText(pair.value, reply.scale, options.measure), value: pair.value });
  }
  const unlisted = reply.unlisted;
  return {
    hour,
    rows: entities,
    columns: entities,
    cells: { present, absent: unlisted ? { kind: 'missing', reason: unlisted } : { kind: 'empty', label: 'No network', selectable: false } },
    legend: {
      gradient: { lowLabel: '0', highLabel: coverageText(reply.maxValue, reply.scale, options.measure), stops: [grey, accent] },
      entries: unlisted ? [{ id: 'missing', label: 'Not recorded', color: grey, hatched: true }] : [{ id: 'empty', label: 'No network', color: grey }],
    },
    tooltip: (_row, _column, item, rowIndex, columnIndex) => {
      if (item.kind === 'diagonal') return null;
      const title = `${labelAt(rowIndex, readHour)}’s network in ${labelAt(columnIndex, readHour)}`;
      if (item.kind === 'missing') return { title, hour: readHour, missingReason: item.reason };
      if (item.kind === 'empty') return { title, hour: readHour, items: [{ label: 'No intel network' }] };
      const raw = networks.get(rowIndex * n + columnIndex);
      return {
        title,
        hour: readHour,
        items: raw
          ? [
              { label: 'Controlled states covered', value: String(raw.coveredControlled) },
              { label: 'Core states covered', value: String(raw.coveredCore) },
              { label: 'Strength in controlled states', value: coverageText(raw.controlledStrength, FIXED, 'strength') },
              { label: 'Strength in core states', value: coverageText(raw.coreStrength, FIXED, 'strength') },
              { label: 'Strongest state', value: coverageText(raw.maxStrength, FIXED, 'strongest') },
            ]
          : [],
      };
    },
    warnings: reply.warnings,
  };
}

export const intelCoverageProjection: HeatMatrixProjection<IntelCoverageProjectionOptions> = {
  kind: 'intel-network-coverage',
  family: 'heat-matrix',
  project: projectIntelCoverage,
};

const measureOf = (value: unknown): IntelCoverageMeasure => (value === 'strength' || value === 'strongest' ? value : 'covered');
const basisOf = (value: unknown): IntelCoverageBasis => (value === 'core' ? 'core' : 'controlled');

export const intelCoverageDeclaration: HeatMatrixWidgetDeclaration<IntelCoverageProjectionOptions> = {
  family: 'heat-matrix',
  kind: 'intel-network-coverage',
  label: 'Intel network coverage',
  subject: 'intel networks',
  emptyCampaign: 'Load a campaign to see intel networks.',
  icon: 'eye',
  countries: { mode: 'multi' },
  sources: (ctx): SourcePlan => {
    const measure = measureOf(ctx.options.measure);
    const basis = basisOf(ctx.options.basis);
    const shown = typeof ctx.options.mostCountries === 'number' ? Math.max(2, ctx.options.mostCountries) : INTEL_TOP_DEFAULT;
    const group = ctx.countries.all || ctx.countries.top !== null ? null : groupSelection(ctx.widget.countries);
    const tags = group || ctx.countries.all || ctx.countries.top !== null ? [] : ctx.countries.tags;
    const top = tags.length || group ? undefined : ctx.countries.top ?? shown;
    const params = {
      branchId: ctx.branch.id,
      ...(group ? { group } : tags.length ? { countries: tags.map(countryFromKey) } : {}),
      ...(top !== undefined ? { top } : {}),
      touching: tags.length === 1,
      measure,
      basis,
    };
    return {
      owner: `intel-network-coverage:${JSON.stringify(params)}`,
      entries: [{
        name: 'coverage',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: group ? { mode: 'population', form: 'all' } : tags.length ? { mode: 'keyed', keys: tags } : { mode: 'population', form: 'top', limit: top ?? shown },
          product: 'intel-network-coverage',
          followQuery: { panel: 'intel-network-coverage', ...params },
          params,
        },
      }],
    };
  },
  projection: intelCoverageProjection,
  projectionOptions: (ctx, extra) => ({ measure: measureOf(ctx.options.measure), surface: extra.surface }),
  jumpToDate: true,
};
