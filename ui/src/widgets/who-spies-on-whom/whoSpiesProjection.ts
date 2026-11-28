import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsDataRow, BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { barsLabelText, countryRowPresentation } from '../../families/bars/model.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { activeGroupMembers, nextDatedMembershipChange, type DatedMembershipOptions } from '../../families/groupFilter.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import { NO_CELLS, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity, type MatrixCell, type MatrixMarginCell } from '../../families/heat/matrixModel.ts';
import { resolveEntityFromReadView } from '../../families/heat/scale.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, notRecorded, partialValue, pendingValue, type Value } from '../../families/value.ts';
import type { Operative, OperativeMatrixForm, OperativeMatrixReply } from '../../generated/dashboard';

export type OperativeMatrixOptions = DatedMembershipOptions;

const NOT_RECORDED = 'Operatives were not recorded at this date.';
const NONE_PLACED = 'No operatives in the field at this date.';
const TARGET_NOT_RECORDED = 'An operative\'s target country was not recorded at this hour';
const UNKNOWN_TARGET_COLUMN_ID = '__unknown_target__';

function optionsOf(ctx: FamilyPlanContext): OperativeMatrixOptions {
  return {
    membership: ctx.countries.membership,
    unobserved: ctx.countries.unobserved,
    membershipMode: ctx.widget.countries.kind === 'group' ? ctx.widget.countries.membership : null,
  };
}

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

export function projectOperativeMatrix(readView: BoundedReadView, options?: OperativeMatrixOptions): HeatMatrixModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return blank(hour, { refused });
  const reply = readView.panel<OperativeMatrixReply>('main');
  if (!reply) return blank(hour, { empty: NOT_RECORDED, emptyKind: 'unavailable' });
  const group = activeGroupMembers(options, hour);
  if (group.kind === 'unavailable') return blank(hour, { empty: group.reason, emptyKind: 'unavailable', warnings: reply.warnings });
  if (group.kind === 'empty') return blank(hour, { empty: group.reason, warnings: reply.warnings });
  if (reply.missing) return blank(hour, { empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
  if (!reply.rows.length) return blank(hour, { empty: NONE_PLACED, warnings: reply.warnings });

  let activeRows = reply.rows;
  let activeColumns = reply.columns;
  let activeCells = reply.cells;
  let mostRaw = reply.most;
  let largestMaximum = reply.largestMaximum;
  if (group.kind === 'filtered') {
    const active = group.active;
    if (!active.size) return blank(hour, { empty: NONE_PLACED, warnings: reply.warnings });
    const keptRowIndices = new Set<number>();
    const keptColIndices = new Set<number>();
    const filteredCells: OperativeMatrixReply['cells'][number][] = [];
    for (const cell of reply.cells) {
      const owner = reply.countries[reply.rows[cell.row]!.country]!.tag;
      const target = reply.countries[reply.columns[cell.column]!]!.tag;
      if (!active.has(owner) && !active.has(target)) continue;
      keptRowIndices.add(cell.row);
      keptColIndices.add(cell.column);
      filteredCells.push(cell);
    }
    const rowMap = new Map<number, number>();
    const keptRows: OperativeMatrixReply['rows'][number][] = [];
    largestMaximum = 0;
    for (let r = 0; r < reply.rows.length; r++) {
      const row = reply.rows[r]!;
      const owner = reply.countries[row.country]!.tag;
      if (!active.has(owner) && !keptRowIndices.has(r) && row.missing === null && !row.unknownTarget.length) continue;
      rowMap.set(r, keptRows.length);
      keptRows.push(row);
      if (row.capacity.kind === 'known') largestMaximum = Math.max(largestMaximum, row.capacity.maximum);
    }
    if (!keptRows.length) return blank(hour, { empty: NONE_PLACED, warnings: reply.warnings });
    const colMap = new Map<number, number>();
    const keptColumns: number[] = [];
    for (let c = 0; c < reply.columns.length; c++) {
      const countryIndex = reply.columns[c]!;
      const target = reply.countries[countryIndex]!.tag;
      if (!active.has(target) && !keptColIndices.has(c)) continue;
      colMap.set(c, keptColumns.length);
      keptColumns.push(countryIndex);
    }
    if (!keptColumns.length && !keptRows.some(row => row.unknownTarget.length > 0)) return blank(hour, { empty: NONE_PLACED, warnings: reply.warnings });
    mostRaw = 0;
    activeCells = filteredCells.map(cell => {
      if (!cell.home) mostRaw = Math.max(mostRaw, cell.operatives.length);
      return { ...cell, row: rowMap.get(cell.row)!, column: colMap.get(cell.column)! };
    });
    activeRows = keptRows;
    activeColumns = keptColumns;
  }

  const axes = new Map<number, MatrixAxisEntity>();
  const entity = (index: number): MatrixAxisEntity => {
    const held = axes.get(index);
    if (held) return held;
    const country = reply.countries[index]!;
    const shown = resolveEntityFromReadView(readView, 'country', country.tag, hour, country.tag);
    const made: MatrixAxisEntity = {
      id: country.tag,
      label: shown.label,
      shortLabel: country.tag,
      tag: country.tag,
      ...(shown.nameMissing ? { nameMissing: shown.nameMissing } : {}),
      ...(shown.color ? { color: shown.color } : {}),
      ...(shown.flag ? { flag: shown.flag } : {}),
    };
    axes.set(index, made);
    return made;
  };
  const hasUnknownTarget = activeRows.some(row => row.unknownTarget.length > 0);
  const rows = activeRows.map(row => entity(row.country));
  const columns = activeColumns.map(entity);
  const unknownColIndex = hasUnknownTarget ? columns.length : -1;
  if (hasUnknownTarget) {
    columns.push({
      id: UNKNOWN_TARGET_COLUMN_ID,
      label: 'Target not recorded',
      shortLabel: '?',
      nameMissing: TARGET_NOT_RECORDED,
    });
  }
  const width = columns.length;
  const byPair = new Map<number, OperativeMatrixReply['cells'][number]>();
  for (const cell of activeCells) byPair.set(cell.row * width + cell.column, cell);
  const most = Math.max(1, mostRaw);
  const hasMissing = hasUnknownTarget || activeRows.some(row => row.missing !== null || row.capacity.kind === 'missing');
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
  if (hasUnknownTarget) {
    for (let r = 0; r < activeRows.length; r++) {
      const unknown = activeRows[r]!.unknownTarget;
      if (!unknown.length) continue;
      const reason = activeRows[r]!.missing ?? TARGET_NOT_RECORDED;
      present.set(r * width + unknownColIndex, {
        kind: 'missing',
        reason,
        text: String(unknown.length),
        selectable: true,
        audit: `${unknown.length} (Not recorded: ${reason})`,
      });
    }
  }
  const noOperatives: MatrixCell = { kind: 'empty', label: 'No operatives', audit: '0' };
  const noUnknownTarget: MatrixCell = { kind: 'empty', label: 'No operatives with target not recorded', audit: '0' };
  const home: MatrixCell = { kind: 'diagonal', outlined: true };
  const outsideSelection: MatrixCell = { kind: 'empty', label: 'Outside selected group', audit: 'Outside selection' };
  const activeSet = group.kind === 'filtered' ? group.active : null;
  const absent = (r: number, c?: number): MatrixCell => {
    if (c === unknownColIndex) return noUnknownTarget;
    if (activeSet && c !== undefined && !activeSet.has(rows[r]!.id) && !activeSet.has(columns[c]!.id)) return outsideSelection;
    const missing = activeRows[r]!.missing;
    return missing ? { kind: 'missing', reason: missing } : noOperatives;
  };
  const self = (r: number): MatrixCell => {
    if (activeSet && !activeSet.has(rows[r]!.id)) return outsideSelection;
    const missing = activeRows[r]!.missing;
    return missing ? { kind: 'missing', reason: missing } : home;
  };
  const cells = { present, absent, self };

  const margin = (r: number): MatrixMarginCell => {
    const capacity = activeRows[r]!.capacity;
    if (capacity.kind === 'missing') return { kind: 'missing', reason: capacity.reason };
    if (capacity.kind === 'noAgency') return { kind: 'none', label: 'No agency' };
    return {
      kind: 'value',
      fraction: largestMaximum > 0 ? capacity.active / largestMaximum : 0,
      text: `${capacity.active} / ${capacity.maximum}`,
    };
  };

  return {
    hour,
    ...(reply.readHour !== null ? { dataHour: reply.readHour } : {}),
    rows,
    columns,
    cells,
    shape: 'wide',
    ramp: 'accent',
    corner: 'Spy ↓ · target →',
    margin: { header: 'Active / maximum', cell: margin },
    legend: {
      gradient: { lowLabel: '0', highLabel: operativesLabel(mostRaw), stops: [] },
      entries: [
        { id: 'home', label: 'Own country: counter-intelligence', color: '', outlined: true },
        ...(hasMissing ? [{ id: 'missing', label: 'Not recorded', color: '', hatched: true }] : []),
      ],
    },
    tooltip: (row, column, item, r, c) => {
      if (c === unknownColIndex) {
        const unknown = activeRows[r]!.unknownTarget;
        if (!unknown.length) {
          return { title: `${row.label} → ${column.label}`, hour: readHour, items: [{ label: 'No operatives with target not recorded', tone: 'muted' }] };
        }
        const reason = item.kind === 'missing' ? item.reason : (activeRows[r]!.missing ?? TARGET_NOT_RECORDED);
        return {
          title: `${row.label}: ${operativesLabel(unknown.length)} (target not recorded)`,
          hour: readHour,
          missingReason: reason,
          items: operativeItems(unknown),
        };
      }
      const found = byPair.get(r * width + c);
      if (item.kind === 'diagonal') {
        if (!found) return { title: row.label, hour: readHour, items: [{ label: 'No operatives on counter-intelligence', tone: 'muted' }] };
        return { title: `${row.label}: ${operativesLabel(found.operatives.length)} on counter-intelligence at home`, hour: readHour, items: operativeItems(found.operatives) };
      }
      if (item.kind === 'missing') return { title: `${row.label} → ${column.label}`, hour: readHour, missingReason: item.reason };
      if (!found) return { title: `${row.label} → ${column.label}`, hour: readHour, items: [{ label: item.kind === 'empty' && item.label ? item.label : 'No operatives', tone: 'muted' }] };
      return { title: `${row.label} has ${operativesLabel(found.operatives.length)} in ${column.label}`, hour: readHour, items: operativeItems(found.operatives) };
    },
    warnings: reply.warnings,
  };
}

export const operativeMatrixProjection: HeatMatrixProjection<OperativeMatrixOptions> = {
  kind: 'who-spies-on-whom',
  family: 'heat-matrix',
  project: (readView, options) => projectOperativeMatrix(readView, options),
  nextChange: (readView, options) => nextDatedMembershipChange(options, readView.hour),
};

export const operativeMatrixDeclaration: HeatMatrixWidgetDeclaration<OperativeMatrixOptions> = {
  family: 'heat-matrix',
  kind: 'who-spies-on-whom',
  label: 'Who spies on whom',
  subject: 'operatives',
  emptyCampaign: 'Load a campaign to see its operatives.',
  icon: 'target',
  countries: { mode: 'multi', content: 'hour', datedAtHour: true },
  sources: sources('matrix'),
  projection: operativeMatrixProjection,
  projectionOptions: optionsOf,
  jumpToDate: true,
};

export function projectOperativePairs(readView: BoundedReadView, options?: OperativeMatrixOptions): BarsModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return { form: 'ordered', hour, rows: [], empty: refused, emptyKind: 'unavailable' };
  const reply = readView.panel<OperativeMatrixReply>('main');
  if (!reply) return { form: 'ordered', hour, rows: [], empty: NOT_RECORDED, emptyKind: 'unavailable' };
  const group = activeGroupMembers(options, hour);
  if (group.kind === 'unavailable') return { form: 'ordered', hour, rows: [], empty: group.reason, emptyKind: 'unavailable', warnings: reply.warnings };
  if (group.kind === 'empty') return { form: 'ordered', hour, rows: [], empty: group.reason, warnings: reply.warnings };
  if (reply.missing) return { form: 'ordered', hour, rows: [], empty: NOT_RECORDED, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
  const pairs = group.kind === 'filtered'
    ? (group.active.size > 0
      ? reply.pairs.filter(pair =>
          pair.target === null
          || group.active.has(reply.countries[pair.spy]!.tag)
          || group.active.has(reply.countries[pair.target]!.tag))
      : [])
    : reply.pairs;
  if (!pairs.length) return { form: 'ordered', hour, rows: [], empty: NONE_PLACED, warnings: reply.warnings };
  const readHour = reply.readHour ?? hour;
  const presentations = new Map<string, ReturnType<typeof countryRowPresentation>>();
  const presentation = (tag: string) => {
    const held = presentations.get(tag);
    if (held) return held;
    const made = countryRowPresentation(readView, tag);
    presentations.set(tag, made);
    return made;
  };
  let scaleMax = 0;
  const rows: BarsDataRow[] = pairs.map(pair => {
    const spy = reply.countries[pair.spy]!.tag;
    const shownSpy = presentation(spy);
    const count = pair.operatives.length;
    scaleMax = Math.max(scaleMax, count);
    if (pair.target === null) {
      const missingReason = pair.targetMissing ?? TARGET_NOT_RECORDED;
      if (shownSpy.label.kind === 'pending') {
        return {
          kind: 'data',
          key: `${spy}:unknown-target`,
          subject: spy,
          label: pendingValue(shownSpy.label.reason),
          muted: true,
          media: shownSpy.media,
          ...(shownSpy.color ? { color: shownSpy.color } : {}),
          value: { value: knownValue(count), format: 'integer' },
          numeric: count,
          tooltip: {
            title: barsLabelText(shownSpy.label),
            hour: readHour,
            missingReason,
            items: [{ label: 'Spy', pendingReason: shownSpy.label.reason }, ...operativeItems(pair.operatives)],
          },
        };
      }
      const ownerLabel = shownSpy.label.kind === 'known' ? shownSpy.label.value : spy;
      return {
        kind: 'data',
        key: `${spy}:unknown-target`,
        subject: spy,
        label: knownValue(`${ownerLabel} → Target not recorded`),
        muted: true,
        media: shownSpy.media,
        ...(shownSpy.color ? { color: shownSpy.color } : {}),
        value: { value: knownValue(count), format: 'integer' },
        numeric: count,
        tooltip: {
          title: `${ownerLabel} has ${operativesLabel(count)} (target not recorded)`,
          hour: readHour,
          missingReason,
          items: operativeItems(pair.operatives),
        },
      };
    }
    const target = reply.countries[pair.target]!.tag;
    const shownTarget = presentation(target);
    const spyLabel = shownSpy.label;
    const targetLabel = shownTarget.label;
    let label: Value<string>;
    let tooltipTitle: string;
    const pendingItems: TooltipListItem[] = [];
    if (spyLabel.kind === 'known' && targetLabel.kind === 'known') {
      label = knownValue(`${spyLabel.value} → ${targetLabel.value}`);
      tooltipTitle = `${spyLabel.value} has ${operativesLabel(count)} in ${targetLabel.value}`;
    } else if (spyLabel.kind === 'known') {
      const reason = 'reason' in targetLabel ? targetLabel.reason : 'Country not recorded';
      label = partialValue(`From ${spyLabel.value}`, reason);
      tooltipTitle = barsLabelText(label);
      if (targetLabel.kind === 'pending') pendingItems.push({ label: 'Target', pendingReason: targetLabel.reason });
    } else if (targetLabel.kind === 'known') {
      const reason = 'reason' in spyLabel ? spyLabel.reason : 'Country not recorded';
      label = partialValue(`In ${targetLabel.value}`, reason);
      tooltipTitle = barsLabelText(label);
      if (spyLabel.kind === 'pending') pendingItems.push({ label: 'Spy', pendingReason: spyLabel.reason });
    } else {
      const reasons = [...new Set([
        'reason' in spyLabel ? spyLabel.reason : 'Country not recorded',
        'reason' in targetLabel ? targetLabel.reason : 'Country not recorded',
      ])].join('; ');
      label = spyLabel.kind === 'pending' || targetLabel.kind === 'pending' ? pendingValue(reasons) : notRecorded(reasons);
      tooltipTitle = barsLabelText(label);
    }
    return {
      kind: 'data',
      key: `${spy}>${target}`,
      subject: spy,
      label,
      media: shownSpy.media,
      ...(shownSpy.color ? { color: shownSpy.color } : {}),
      value: { value: knownValue(count), format: 'integer' },
      numeric: count,
      tooltip: { title: tooltipTitle, hour: readHour, items: [...pendingItems, ...operativeItems(pair.operatives)] },
    };
  });
  return {
    form: 'ordered',
    hour,
    rows,
    scaleMax,
    animateOrder: true,
    ...(reply.readHour !== null ? { dataHours: [reply.readHour] } : {}),
    warnings: reply.warnings,
  };
}

export const operativePairsProjection: BarsProjection<OperativeMatrixOptions> = {
  kind: 'who-spies-on-whom',
  family: 'bars',
  project: (readView, options) => projectOperativePairs(readView, options),
  nextChange: (readView, options) => nextDatedMembershipChange(options, readView.hour),
};

export const operativePairsDeclaration: BarsWidgetDeclaration<OperativeMatrixOptions> = {
  family: 'bars',
  kind: 'who-spies-on-whom',
  label: 'Who spies on whom',
  subject: 'operatives',
  emptyCampaign: 'Load a campaign to see its operatives.',
  icon: 'target',
  countries: { mode: 'multi', content: 'hour', datedAtHour: true },
  sources: sources('pairs'),
  projection: operativePairsProjection,
  projectionOptions: optionsOf,
  jumpToDate: true,
};
