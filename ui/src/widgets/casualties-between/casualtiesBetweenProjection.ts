import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsDataRow, BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { countryRowPresentation } from '../../families/bars/model.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { activeGroupMembers, nextDatedMembershipChange, type DatedMembershipOptions } from '../../families/groupFilter.ts';
import type { HourRanges } from '../../families/holders.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import { NO_CELLS, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity, type MatrixCell, type MatrixMarginCell } from '../../families/heat/matrixModel.ts';
import { resolveEntityFromReadView } from '../../families/heat/scale.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import { knownValue, notRecorded, partialValue, type Value } from '../../families/value.ts';
import { formatCompactScalar, formatScalar } from '../../families/valueFormat.ts';
import type { CasualtyMatrixReply } from '../../generated/dashboard';

export interface CasualtyMatrixOptions extends DatedMembershipOptions {
  readonly between?: boolean;
  readonly identityMembership?: Readonly<Record<string, HourRanges>> | null;
}

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

export function projectCasualtyMatrix(readView: BoundedReadView, options?: CasualtyMatrixOptions): HeatMatrixModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return blank(hour, { refused });
  const reply = readView.panel<CasualtyMatrixReply>('main');
  if (!reply) return blank(hour, { empty: NOT_RECORDED, emptyKind: 'unavailable' });
  const byIdentity = options?.identityMembership != null;
  const group = activeGroupMembers(byIdentity ? { ...options, membership: options.identityMembership } : options, hour);
  if (group.kind === 'unavailable') return blank(hour, { empty: group.reason, emptyKind: 'unavailable', warnings: reply.warnings });
  if (group.kind === 'empty') return blank(hour, { empty: group.reason, warnings: reply.warnings });
  if (reply.missing) return blank(hour, { empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
  if (!reply.order.length) return blank(hour, { empty: NONE_YET, warnings: reply.warnings });

  const memberKey = (country: CasualtyMatrixReply['countries'][number]) => (byIdentity ? countryIdentityKey(country) : country.tag);
  let activeOrder = reply.order;
  let activeCells = reply.cells;
  let mostText = reply.most;
  let largestText = reply.largestDealt;
  const between = options?.between === true;
  if (group.kind === 'filtered') {
    const active = group.active;
    const keptPositions = new Set<number>();
    const filteredCells: CasualtyMatrixReply['cells'][number][] = [];
    for (const cell of reply.cells) {
      const from = memberKey(reply.countries[reply.order[cell.row]!]!);
      const to = memberKey(reply.countries[reply.order[cell.column]!]!);
      const keep = between ? (active.has(from) && active.has(to)) : (active.has(from) || active.has(to));
      if (!keep) continue;
      keptPositions.add(cell.row);
      keptPositions.add(cell.column);
      filteredCells.push(cell);
    }
    const posMap = new Map<number, number>();
    const keptOrder: number[] = [];
    for (let pos = 0; pos < reply.order.length; pos++) {
      const countryIndex = reply.order[pos]!;
      const country = reply.countries[countryIndex]!;
      if (!keptPositions.has(pos) && !active.has(memberKey(country))) continue;
      posMap.set(pos, keptOrder.length);
      keptOrder.push(countryIndex);
    }
    if (!keptOrder.length) return blank(hour, { empty: NONE_YET, warnings: reply.warnings });
    activeCells = filteredCells.map(cell => ({ ...cell, row: posMap.get(cell.row)!, column: posMap.get(cell.column)! }));
    activeOrder = keptOrder;

    let largestBig = 0n;
    for (const index of activeOrder) {
      const country = reply.countries[index]!;
      if (country.dealt === null) continue;
      const dealt = BigInt(country.dealt);
      if (dealt > largestBig) largestBig = dealt;
    }
    let mostBig = 0n;
    for (const cell of activeCells) {
      if (cell.row === cell.column || cell.dealt === null) continue;
      const dealt = BigInt(cell.dealt);
      if (dealt > mostBig) mostBig = dealt;
    }
    mostText = mostBig.toString();
    largestText = largestBig.toString();
  }

  const tagCounts = new Map<string, number>();
  for (const index of activeOrder) {
    const tag = reply.countries[index]!.tag;
    tagCounts.set(tag, (tagCounts.get(tag) ?? 0) + 1);
  }
  const entities: MatrixAxisEntity[] = activeOrder.map(index => {
    const country = reply.countries[index]!;
    const shown = resolveEntityFromReadView(readView, 'country', country.tag, hour, country.tag);
    const id = (tagCounts.get(country.tag) ?? 0) > 1 ? `${country.tag}#${country.slot}:${country.countryId}` : country.tag;
    return {
      id,
      label: shown.label,
      shortLabel: country.tag,
      tag: country.tag,
      ...(shown.nameMissing ? { nameMissing: shown.nameMissing } : {}),
      ...(shown.color ? { color: shown.color } : {}),
      ...(shown.flag ? { flag: shown.flag } : {}),
    };
  });
  const width = entities.length;
  const byPair = new Map<number, CasualtyMatrixReply['cells'][number]>();
  for (const cell of activeCells) byPair.set(cell.row * width + cell.column, cell);
  const most = Number(BigInt(mostText)) || 1;
  const largest = Number(BigInt(largestText)) || 1;
  const hasMissing = activeCells.some(cell => cell.dealt === null)
    || activeOrder.some(index => reply.countries[index]!.missing !== null);

  const present = new Map<number, MatrixCell>();
  for (const [at, found] of byPair) {
    const r = Math.floor(at / width);
    const c = at % width;
    if (r === c) continue;
    present.set(at, found.dealt === null
      ? { kind: 'missing', reason: found.missing ?? NOT_RECORDED }
      : found.dealt === '0'
        ? { kind: 'empty', label: '0 casualties', audit: '0' }
        : { kind: 'value', intensity: Number(BigInt(found.dealt)) / most, text: compact(found.dealt), value: BigInt(found.dealt), audit: found.dealt, selectable: true });
  }
  const notAtWar: MatrixCell = { kind: 'empty', label: 'Not at war: no casualties', audit: '0' };
  const outsideSelection: MatrixCell = { kind: 'empty', label: 'Outside selected group', audit: 'Outside selection' };
  const activeSet = group.kind === 'filtered' ? group.active : null;
  const absent: MatrixCell | ((r: number, c?: number) => MatrixCell) = activeSet
    ? (r, c) => {
        if (c === undefined) return notAtWar;
        const from = memberKey(reply.countries[activeOrder[r]!]!);
        const to = memberKey(reply.countries[activeOrder[c]!]!);
        const inSelection = between ? (activeSet.has(from) && activeSet.has(to)) : (activeSet.has(from) || activeSet.has(to));
        return inSelection ? notAtWar : outsideSelection;
      }
    : notAtWar;
  const cells = { present, absent, self: { kind: 'diagonal', text: '—' } as MatrixCell };

  const margin = (r: number): MatrixMarginCell => {
    const country = reply.countries[activeOrder[r]!]!;
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
      gradient: { lowLabel: '0', highLabel: `${compact(mostText)} casualties`, stops: [] },
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

export const casualtyMatrixProjection: HeatMatrixProjection<CasualtyMatrixOptions> = {
  kind: 'casualties-between',
  family: 'heat-matrix',
  project: (readView, options) => projectCasualtyMatrix(readView, options),
  nextChange: (readView, options) => nextDatedMembershipChange(
    options?.identityMembership != null ? { ...options, membership: options.identityMembership } : options,
    readView.hour,
  ),
};

export const casualtyMatrixDeclaration: HeatMatrixWidgetDeclaration<CasualtyMatrixOptions> = {
  family: 'heat-matrix',
  kind: 'casualties-between',
  label: 'Casualties between countries',
  subject: 'casualties',
  emptyCampaign: 'Load a campaign to see its casualties.',
  icon: 'target',
  countries: { mode: 'multi', datedAtHour: true },
  sources: ctx => plan(ctx, null),
  projection: casualtyMatrixProjection,
  projectionOptions: ctx => ({
    membership: ctx.widget.countries.kind === 'group' && ctx.widget.countries.membership === 'ever'
      ? (ctx.countries.cumulativeMembership ?? ctx.countries.membership)
      : ctx.countries.membership,
    identityMembership: ctx.countries.identityMembership,
    unobserved: ctx.countries.unobserved,
    membershipMode: ctx.widget.countries.kind === 'group' ? ctx.widget.countries.membership : null,
    between: ctx.options.between === 'between',
  }),
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
  const tagCounts = new Map<string, number>();
  for (const opponent of reply.opponents) {
    const tag = reply.countries[opponent.country]!.tag;
    tagCounts.set(tag, (tagCounts.get(tag) ?? 0) + 1);
  }
  const rows: BarsDataRow[] = reply.opponents.map(opponent => {
    const country = reply.countries[opponent.country]!;
    const tag = country.tag;
    const key = (tagCounts.get(tag) ?? 0) > 1 ? `${tag}#${country.slot}:${country.countryId}` : tag;
    const shown = countryRowPresentation(readView, tag);
    const left = side(opponent.taken, opponent.missing, opponent.incomplete);
    const right = side(opponent.dealt, opponent.missing, opponent.incomplete);
    return {
      kind: 'data',
      key,
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
