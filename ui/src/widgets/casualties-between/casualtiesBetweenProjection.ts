import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsDataRow, BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { countryRowPresentation } from '../../families/bars/model.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import { NO_CELLS, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity, type MatrixCell, type MatrixMarginCell } from '../../families/heat/matrixModel.ts';
import { resolveEntityFromReadView } from '../../families/heat/scale.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { knownValue, notRecorded, partialValue, type Value } from '../../families/value.ts';
import { formatCompactScalar, formatScalar } from '../../families/valueFormat.ts';
import type { CasualtyMatrixReply } from '../../generated/dashboard';

const EMPTY_OPTIONS: Readonly<Record<string, never>> = {};
const NOT_RECORDED = 'Casualties were not recorded at this date.';
const NONE_YET = 'No casualties between these countries yet.';

const compact = (amount: string) => formatCompactScalar(BigInt(amount)).text;
const exact = (amount: string) => formatScalar(BigInt(amount), 'integer');

function plan(ctx: FamilyPlanContext, subject: string | null): SourcePlan {
  const top = ctx.countries.top;
  const tags = subject !== null || ctx.countries.all || top !== null ? [] : ctx.countries.tags;
  const between = ctx.options.between === 'between';
  const params: Record<string, unknown> = {
    branchId: ctx.branch.id,
    countries: tags.map(countryFromKey),
    between,
    ...(top !== null && subject === null ? { top } : {}),
    ...(subject !== null ? { subject: countryFromKey(subject) } : {}),
  };
  return {
    owner: `casualties-between:${ctx.branch.id}:${subject ?? ''}:${between}:${top ?? (ctx.countries.all ? 'all' : tags.join(','))}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: subject !== null
          ? { mode: 'keyed', keys: [subject] }
          : top !== null
            ? { mode: 'population', form: 'top', limit: top }
            : tags.length ? { mode: 'keyed', keys: tags } : { mode: 'population', form: 'all' },
        product: 'casualty-matrix',
        followQuery: { panel: 'casualty-matrix', ...params },
        params,
      },
    }],
  };
}

function blank(hour: number, extra: Partial<HeatMatrixModel>): HeatMatrixModel {
  return { hour, rows: [], columns: [], cells: NO_CELLS, ...extra };
}

export function projectCasualtyMatrix(readView: BoundedReadView): HeatMatrixModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return blank(hour, { refused });
  const reply = readView.panel<CasualtyMatrixReply>('main');
  if (!reply) return blank(hour, { empty: NOT_RECORDED, emptyKind: 'unavailable' });
  if (reply.missing) return blank(hour, { empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
  if (!reply.order.length) return blank(hour, { empty: NONE_YET, warnings: reply.warnings });

  const entities: MatrixAxisEntity[] = reply.order.map(index => {
    const country = reply.countries[index]!;
    const shown = resolveEntityFromReadView(readView, 'country', country.tag, hour, country.tag);
    return {
      id: country.tag,
      label: shown.label,
      shortLabel: country.tag,
      tag: country.tag,
      ...(shown.nameMissing ? { nameMissing: shown.nameMissing } : {}),
      ...(shown.color ? { color: shown.color } : {}),
      flag: shown.flag ?? null,
      flagPending: shown.pending === true,
    };
  });
  const width = entities.length;
  const byPair = new Map<number, CasualtyMatrixReply['cells'][number]>();
  for (const cell of reply.cells) byPair.set(cell.row * width + cell.column, cell);
  const most = Number(BigInt(reply.most)) || 1;
  const largest = Number(BigInt(reply.largestDealt)) || 1;
  const hasMissing = reply.cells.some(cell => cell.dealt === null) || reply.order.some(index => reply.countries[index]!.missing !== null);

  const present = new Map<number, MatrixCell>();
  for (const [at, found] of byPair) {
    if (Math.floor(at / width) === at % width) continue;
    present.set(at, found.dealt === null
      ? { kind: 'missing', reason: found.missing ?? NOT_RECORDED }
      : found.dealt === '0'
        ? { kind: 'empty', label: '0 casualties', audit: '0' }
        : { kind: 'value', intensity: Number(BigInt(found.dealt)) / most, text: compact(found.dealt), value: BigInt(found.dealt), audit: found.dealt, selectable: true });
  }
  const cells = { present, absent: { kind: 'empty', label: 'Not at war: no casualties', audit: '0' } as MatrixCell, self: { kind: 'diagonal', text: '—' } as MatrixCell };

  const margin = (r: number): MatrixMarginCell => {
    const country = reply.countries[reply.order[r]!]!;
    if (country.dealt === null) return { kind: 'missing', reason: country.missing ?? NOT_RECORDED };
    return { kind: 'value', fraction: Number(BigInt(country.dealt)) / largest, text: compact(country.dealt), exact: exact(country.dealt) };
  };

  return {
    hour,
    rows: entities,
    columns: entities,
    cells,
    shape: 'wide',
    ramp: 'red',
    corner: 'Dealt by ↓ · to →',
    margin: { header: 'Dealt in total', cell: margin },
    legend: {
      gradient: { lowLabel: '0', highLabel: `${compact(reply.most)} casualties`, stops: [] },
      ...(hasMissing ? { entries: [{ id: 'missing', label: 'Not recorded', color: '', hatched: true }] } : {}),
    },
    tooltip: (row, column, item, r, c) => {
      if (item.kind === 'diagonal') return null;
      const title = `${row.label} → ${column.label}`;
      if (item.kind === 'missing') return { title, hour, missingReason: item.reason };
      const found = byPair.get(r * width + c);
      if (!found || found.dealt === null) return { title, hour, items: [{ label: item.kind === 'empty' ? (item.label ?? 'No casualties') : 'No casualties', tone: 'muted' }] };
      return {
        title: `Casualties ${row.label} dealt to ${column.label}`,
        hour,
        exactValue: exact(found.dealt),
        ...(found.incomplete ? { note: found.incomplete } : {}),
      };
    },
    warnings: reply.warnings,
  };
}

export const casualtyMatrixProjection: HeatMatrixProjection = {
  kind: 'casualties-between',
  family: 'heat-matrix',
  project: readView => projectCasualtyMatrix(readView),
};

export const casualtyMatrixDeclaration: HeatMatrixWidgetDeclaration = {
  family: 'heat-matrix',
  kind: 'casualties-between',
  label: 'Casualties between countries',
  subject: 'casualties',
  emptyCampaign: 'Load a campaign to see its casualties.',
  icon: 'target',
  countries: { mode: 'multi', labels: false, colours: false },
  sources: ctx => plan(ctx, null),
  projection: casualtyMatrixProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};

function side(amount: string | null, reason: string | null, incomplete: string | null): { value: Value; numeric: number | null } {
  if (amount === null) return { value: notRecorded(reason ?? NOT_RECORDED), numeric: null };
  const big = BigInt(amount);
  return { value: incomplete ? partialValue(big, incomplete, { atLeast: true }) : knownValue(big), numeric: Number(big) };
}

export function projectCasualtyOpponents(readView: BoundedReadView): BarsModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return { form: 'two-sided', hour, rows: [], empty: refused, emptyKind: 'unavailable' };
  const reply = readView.panel<CasualtyMatrixReply>('main');
  if (!reply) return { form: 'two-sided', hour, rows: [], empty: NOT_RECORDED, emptyKind: 'unavailable' };
  if (reply.missing) return { form: 'two-sided', hour, rows: [], empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
  if (!reply.opponents.length || reply.subject === null) return { form: 'two-sided', hour, rows: [], empty: 'No casualties with any opponent yet.', warnings: reply.warnings };
  const subject = reply.countries[reply.subject]!;
  const rows: BarsDataRow[] = reply.opponents.map(opponent => {
    const tag = reply.countries[opponent.country]!.tag;
    const shown = countryRowPresentation(readView, tag);
    const left = side(opponent.taken, opponent.missing, opponent.incomplete);
    const right = side(opponent.dealt, opponent.missing, opponent.incomplete);
    return {
      kind: 'data',
      key: tag,
      subject: tag,
      label: shown.label,
      labelMissing: shown.labelMissing,
      media: shown.media,
      ...(shown.color ? { color: shown.color } : {}),
      left: { value: { value: left.value, format: 'integer', compact: true }, numeric: left.numeric },
      right: { value: { value: right.value, format: 'integer', compact: true }, numeric: right.numeric },
    };
  });
  return {
    form: 'two-sided',
    hour,
    rows,
    leftHeader: '◀ Taken from them',
    rightHeader: 'Dealt to them ▶',
    scaleMax: Number(BigInt(reply.largestSide)) || 1,
    ...(subject.dealt !== null ? { summaryText: `Dealt in total: ${exact(subject.dealt)}` } : {}),
    warnings: reply.warnings,
  };
}

export const casualtyOpponentsProjection: BarsProjection = {
  kind: 'casualties-between',
  family: 'bars',
  project: readView => projectCasualtyOpponents(readView),
};

export const casualtyOpponentsDeclaration: BarsWidgetDeclaration = {
  family: 'bars',
  kind: 'casualties-between',
  label: 'Casualties between countries',
  subject: 'casualties',
  emptyCampaign: 'Load a campaign to see its casualties.',
  icon: 'target',
  countries: { mode: 'single', choose: 'Choose one country to see its casualties by opponent.' },
  sources: ctx => (ctx.countryTag !== null ? plan(ctx, ctx.countryTag) : { owner: 'casualties-between:choose', entries: [] }),
  projection: casualtyOpponentsProjection,
  projectionOptions: () => EMPTY_OPTIONS,
  jumpToDate: true,
};
