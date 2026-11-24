import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsDataRow, BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { countryRowPresentation } from '../../families/bars/model.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import { NO_CELLS, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity, type MatrixCell, type MatrixMarginCell } from '../../families/heat/matrixModel.ts';
import { resolveEntityFromReadView } from '../../families/heat/scale.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue } from '../../families/value.ts';
import type { Operative, OperativeMatrixForm, OperativeMatrixReply } from '../../generated/dashboard';

const EMPTY_OPTIONS: Readonly<Record<string, never>> = {};
const NOT_RECORDED = 'Operatives were not recorded at this date.';
const NONE_PLACED = 'No operatives in the field at this date.';

export const operativesLabel = (count: number) => `${count} ${count === 1 ? 'operative' : 'operatives'}`;

function operativeItems(operatives: readonly Operative[]): TooltipListItem[] {
  return operatives.map(operative => operative.name
    ? { label: operative.name }
    : { label: 'Unnamed operative', tone: 'muted' as const, ...(operative.nameMissing ? { missingReason: operative.nameMissing } : {}) });
}

function sources(form: OperativeMatrixForm) {
  return (ctx: FamilyPlanContext): SourcePlan => {
    const top = ctx.countries.top;
    const tags = ctx.countries.all || top !== null ? [] : ctx.countries.tags;
    const params: Record<string, unknown> = {
      branchId: ctx.branch.id,
      countries: tags.map(countryFromKey),
      ...(top !== null ? { top } : {}),
      form,
    };
    return {
      owner: `who-spies-on-whom:${form}:${ctx.branch.id}:${top ?? (ctx.countries.all ? 'all' : tags.join(','))}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: top !== null
            ? { mode: 'population', form: 'top', limit: top }
            : tags.length ? { mode: 'keyed', keys: tags } : { mode: 'population', form: 'all' },
          product: 'operative-matrix',
          followQuery: { panel: 'operative-matrix', ...params },
          params,
        },
      }],
    };
  };
}

function blank(hour: number, extra: Partial<HeatMatrixModel>): HeatMatrixModel {
  return { hour, rows: [], columns: [], cells: NO_CELLS, ...extra };
}

export function projectOperativeMatrix(readView: BoundedReadView): HeatMatrixModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return blank(hour, { refused });
  const reply = readView.panel<OperativeMatrixReply>('main');
  if (!reply) return blank(hour, { empty: NOT_RECORDED, emptyKind: 'unavailable' });
  if (reply.missing) return blank(hour, { empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
  if (!reply.rows.length) return blank(hour, { empty: NONE_PLACED, warnings: reply.warnings });

  const entity = (index: number): MatrixAxisEntity => {
    const country = reply.countries[index]!;
    const shown = resolveEntityFromReadView(readView, 'country', country.tag, hour, country.tag);
    return {
      id: country.tag,
      label: shown.label,
      shortLabel: country.tag,
      tag: country.tag,
      ...(shown.nameMissing ? { nameMissing: shown.nameMissing } : {}),
      ...(shown.color ? { color: shown.color } : {}),
      ...(shown.flag ? { flag: shown.flag } : {}),
    };
  };
  const rows = reply.rows.map(row => entity(row.country));
  const columns = reply.columns.map(entity);
  const width = columns.length;
  const byPair = new Map<number, OperativeMatrixReply['cells'][number]>();
  for (const cell of reply.cells) byPair.set(cell.row * width + cell.column, cell);
  const most = Math.max(1, reply.most);
  const hasMissing = reply.rows.some(row => row.missing !== null || row.capacity.kind === 'missing');
  const readHour = reply.readHour ?? hour;

  const present = new Map<number, MatrixCell>();
  for (const [at, found] of byPair) {
    const r = Math.floor(at / width);
    const c = at % width;
    const count = found.operatives.length;
    present.set(at, rows[r]!.id === columns[c]!.id
      ? { kind: 'diagonal', outlined: true, text: String(count) }
      : { kind: 'value', intensity: count / most, text: String(count), value: count, selectable: true });
  }
  const noOperatives: MatrixCell = { kind: 'empty', label: 'No operatives', audit: '0' };
  const home: MatrixCell = { kind: 'diagonal', outlined: true };
  const unknownOr = (known: MatrixCell) => (r: number): MatrixCell => {
    const missing = reply.rows[r]!.missing;
    return missing ? { kind: 'missing', reason: missing } : known;
  };
  const cells = { present, absent: unknownOr(noOperatives), self: unknownOr(home) };

  const margin = (r: number): MatrixMarginCell => {
    const capacity = reply.rows[r]!.capacity;
    if (capacity.kind === 'missing') return { kind: 'missing', reason: capacity.reason };
    if (capacity.kind === 'noAgency') return { kind: 'none', label: 'No agency' };
    return {
      kind: 'value',
      fraction: reply.largestMaximum > 0 ? capacity.active / reply.largestMaximum : 0,
      text: `${capacity.active} / ${capacity.maximum}`,
    };
  };

  return {
    hour,
    rows,
    columns,
    cells,
    shape: 'wide',
    ramp: 'accent',
    corner: 'Spy ↓ · target →',
    margin: { header: 'Active / maximum', cell: margin },
    legend: {
      gradient: { lowLabel: '0', highLabel: operativesLabel(reply.most), stops: [] },
      entries: [
        { id: 'home', label: 'Own country: counter-intelligence', color: '', outlined: true },
        ...(hasMissing ? [{ id: 'missing', label: 'Not recorded', color: '', hatched: true }] : []),
      ],
    },
    tooltip: (row, column, item, r, c) => {
      const found = byPair.get(r * width + c);
      if (item.kind === 'diagonal') {
        if (!found) return { title: row.label, hour: readHour, items: [{ label: 'No operatives on counter-intelligence', tone: 'muted' }] };
        return { title: `${row.label}: ${operativesLabel(found.operatives.length)} on counter-intelligence at home`, hour: readHour, items: operativeItems(found.operatives) };
      }
      if (item.kind === 'missing') return { title: `${row.label} → ${column.label}`, hour: readHour, missingReason: item.reason };
      if (!found) return { title: `${row.label} → ${column.label}`, hour: readHour, items: [{ label: 'No operatives', tone: 'muted' }] };
      return { title: `${row.label} has ${operativesLabel(found.operatives.length)} in ${column.label}`, hour: readHour, items: operativeItems(found.operatives) };
    },
    warnings: reply.warnings,
  };
}

export const operativeMatrixProjection: HeatMatrixProjection = {
  kind: 'who-spies-on-whom',
  family: 'heat-matrix',
  project: readView => projectOperativeMatrix(readView),
};

export const operativeMatrixDeclaration: HeatMatrixWidgetDeclaration = {
  family: 'heat-matrix',
  kind: 'who-spies-on-whom',
  label: 'Who spies on whom',
  subject: 'operatives',
  emptyCampaign: 'Load a campaign to see its operatives.',
  icon: 'target',
  countries: { mode: 'multi' },
  sources: sources('matrix'),
  projection: operativeMatrixProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};

export function projectOperativePairs(readView: BoundedReadView): BarsModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return { form: 'ordered', hour, rows: [], empty: refused, emptyKind: 'unavailable' };
  const reply = readView.panel<OperativeMatrixReply>('main');
  if (!reply) return { form: 'ordered', hour, rows: [], empty: NOT_RECORDED, emptyKind: 'unavailable' };
  if (reply.missing) return { form: 'ordered', hour, rows: [], empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
  if (!reply.pairs.length) return { form: 'ordered', hour, rows: [], empty: NONE_PLACED, warnings: reply.warnings };
  const readHour = reply.readHour ?? hour;
  let scaleMax = 0;
  const rows: BarsDataRow[] = reply.pairs.map(pair => {
    const spy = reply.countries[pair.spy]!.tag;
    const target = reply.countries[pair.target]!.tag;
    const shownSpy = countryRowPresentation(readView, spy);
    const shownTarget = countryRowPresentation(readView, target);
    const count = pair.operatives.length;
    scaleMax = Math.max(scaleMax, count);
    const label = `${shownSpy.label} → ${shownTarget.label}`;
    return {
      kind: 'data',
      key: `${spy}>${target}`,
      subject: spy,
      label,
      labelMissing: shownSpy.labelMissing || shownTarget.labelMissing,
      media: shownSpy.media,
      ...(shownSpy.color ? { color: shownSpy.color } : {}),
      value: { value: knownValue(count), format: 'integer' },
      numeric: count,
      tooltip: { title: `${shownSpy.label} has ${operativesLabel(count)} in ${shownTarget.label}`, hour: readHour, items: operativeItems(pair.operatives) },
    };
  });
  return { form: 'ordered', hour, rows, scaleMax, animateOrder: true, warnings: reply.warnings };
}

export const operativePairsProjection: BarsProjection = {
  kind: 'who-spies-on-whom',
  family: 'bars',
  project: readView => projectOperativePairs(readView),
};

export const operativePairsDeclaration: BarsWidgetDeclaration = {
  family: 'bars',
  kind: 'who-spies-on-whom',
  label: 'Who spies on whom',
  subject: 'operatives',
  emptyCampaign: 'Load a campaign to see its operatives.',
  icon: 'target',
  countries: { mode: 'multi' },
  sources: sources('pairs'),
  projection: operativePairsProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};
