import { CHART_MEASURE_COLOURS, authoredHex, readableColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import { NO_CELLS, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity, type MatrixCell } from '../../families/heat/matrixModel.ts';
import { labelsAtReadings, resolveEntityFromReadView, signedRampColour } from '../../families/heat/scale.ts';
import type { DiplomaticRelationsReply, OpinionOrder } from '../../generated/dashboard';
import { surfaceMix } from '../../shell/appearance/color.ts';

export const RELATIONS_TOP_DEFAULT = 12;

export interface DiplomaticRelationsProjectionOptions {
  readonly order: OpinionOrder;
  readonly surface: string;
}

export function formatOpinion(value: number): string {
  return value > 0 ? `+${value}` : value < 0 ? `−${Math.abs(value)}` : '0';
}

function emptyModel(hour: number, extra: Partial<HeatMatrixModel>): HeatMatrixModel {
  return { hour, rows: [], columns: [], cells: NO_CELLS, ...extra };
}

export function projectDiplomaticRelations(readView: BoundedReadView, options: DiplomaticRelationsProjectionOptions): HeatMatrixModel {
  const hour: number = readView.hour;
  const refused = readView.refused('relations');
  if (refused) return emptyModel(hour, { refused });
  const reply = readView.panel<DiplomaticRelationsReply>('relations');
  if (!reply || !options.surface) return emptyModel(hour, {});
  if (reply.chosenMissing) return emptyModel(hour, { empty: 'The countries to show are not known at this date.', emptyDetail: reply.chosenMissing, emptyKind: 'unavailable', warnings: reply.warnings });
  const owners = reply.owners;
  if (owners.length < 2) {
    return emptyModel(hour, { empty: 'Fewer than two countries are shown at this date.', warnings: reply.warnings });
  }
  const grey = surfaceMix(options.surface, 0.08);
  const negative = readableColour(CHART_MEASURE_COLOURS.opinionNegative, options.surface);
  const positive = readableColour(CHART_MEASURE_COLOURS.opinionPositive, options.surface);
  const resolvedOwners = owners.map(owner => resolveEntityFromReadView(readView, 'country', owner.tag, hour, owner.name ?? owner.tag, owner.nameMissing ?? null, options.surface));
  const labelAt = labelsAtReadings(
    readView,
    owners.map((owner, index) => ({ tag: owner.tag, label: resolvedOwners[index]!.label, validFrom: resolvedOwners[index]!.validFrom })),
    owners.flatMap(owner => (owner.readHour === null || owner.readHour === undefined ? [] : [owner.readHour])),
  );
  const entities: MatrixAxisEntity[] = owners.map((owner, index) => {
    const resolved = resolvedOwners[index]!;
    const group = options.order === 'faction' && owner.factionId
      ? { id: owner.factionId, label: owner.factionName ?? 'Faction name not recorded', ...(owner.color ? { color: readableColour(authoredHex(owner.color), options.surface) } : {}) }
      : null;
    return {
      id: owner.tag,
      label: resolved.label,
      shortLabel: owner.tag,
      tag: owner.tag,
      ...(resolved.nameMissing ? { nameMissing: resolved.nameMissing } : {}),
      ...(resolved.flag ? { flag: resolved.flag } : {}),
      group,
      meta: { countryId: owner.countryId, readHour: owner.readHour ?? null },
    };
  });
  const [low, high] = reply.domain;
  const n = owners.length;
  const opinionCell = (opinion: number): MatrixCell => ({ kind: 'value', color: signedRampColour(opinion, low, high, grey, negative, positive), text: formatOpinion(opinion), value: opinion });
  const present = new Map<number, MatrixCell>();
  for (const pair of reply.pairs) {
    present.set(pair.row * n + pair.column, pair.cell.kind === 'missing' ? { kind: 'missing', reason: pair.cell.reason } : opinionCell(pair.cell.opinion));
  }
  const zero = opinionCell(0);
  const rowDefault: MatrixCell[] = owners.map(owner => (owner.known ? zero : { kind: 'missing', reason: owner.reason ?? 'Opinion not recorded' }));
  const hasMissing = owners.some(owner => !owner.known) || reply.pairs.some(pair => pair.cell.kind === 'missing');
  return {
    hour,
    rows: entities,
    columns: entities,
    cells: { present, absent: rowIndex => rowDefault[rowIndex]! },
    legend: {
      gradient: { lowLabel: formatOpinion(low), highLabel: formatOpinion(high), stops: [negative, grey, positive] },
      ...(hasMissing ? { entries: [{ id: 'missing', label: 'Not recorded', color: grey, hatched: true }] } : {}),
    },
    tooltip: (_row, _column, item, rowIndex, columnIndex) => {
      if (item.kind === 'diagonal') return null;
      const readHour = owners[rowIndex]?.readHour ?? hour;
      const title = `${labelAt(rowIndex, readHour)}’s opinion of ${labelAt(columnIndex, readHour)}`;
      if (item.kind === 'missing') return { title, hour: readHour, missingReason: item.reason };
      return { title, hour: readHour, exactValue: item.kind === 'value' ? item.text ?? '0' : '0' };
    },
    warnings: reply.warnings,
  };
}

export const diplomaticRelationsProjection: HeatMatrixProjection<DiplomaticRelationsProjectionOptions> = {
  kind: 'diplomatic-relations',
  family: 'heat-matrix',
  project: projectDiplomaticRelations,
};

const orderOf = (value: unknown): OpinionOrder => (value === 'name' ? 'name' : 'faction');

export const diplomaticRelationsDeclaration: HeatMatrixWidgetDeclaration<DiplomaticRelationsProjectionOptions> = {
  family: 'heat-matrix',
  kind: 'diplomatic-relations',
  label: 'Diplomatic relations',
  subject: 'diplomatic relations',
  emptyCampaign: 'Load a campaign to see diplomatic relations.',
  icon: 'handshake',
  countries: { mode: 'multi', labels: false, colours: false },
  sources: (ctx): SourcePlan => {
    const order = orderOf(ctx.options.order);
    const top = ctx.countries.top;
    const group = top === null && !ctx.countries.all ? groupSelection(ctx.widget.countries) : null;
    const tags = top === null && !ctx.countries.all && !group ? ctx.countries.tags : [];
    const limit = top ?? RELATIONS_TOP_DEFAULT;
    const chosen = group ? { group } : tags.length ? { countries: tags.map(countryFromKey) } : { top: limit };
    const params = { branchId: ctx.branch.id, ...chosen, order };
    return {
      owner: `diplomatic-relations:${JSON.stringify(params)}`,
      entries: [{
        name: 'relations',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: group ? { mode: 'population', form: 'all' } : tags.length ? { mode: 'keyed', keys: tags } : { mode: 'population', form: 'top', limit },
          product: 'diplomatic-relations',
          followQuery: null,
          params,
          dynamicKeys: ['hour'],
        },
      }],
    };
  },
  projection: diplomaticRelationsProjection,
  projectionOptions: (ctx, extra) => ({ order: orderOf(ctx.options.order), surface: extra.surface }),
  jumpToDate: true,
};
