import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { DiplomaticRelationsReply, OpinionOrder } from '../../generated/dashboard';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import {
  registerHeatMatrixProjection,
  resolveEntityFromReadView,
  resolveRecordedColour,
  signedRampColour,
  type HeatMatrixModel,
  type HeatMatrixProjection,
  type HeatMatrixWidgetDeclaration,
  type MatrixAxisEntity,
  type MatrixCell,
} from '../../families/heat/index.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import { OPTIONS } from './definition.ts';

export interface DiplomaticRelationsProjectionOptions {
  readonly order: OpinionOrder;
  readonly surface: string;
  readonly grey: string;
  readonly negative: string;
  readonly positive: string;
}

export function formatOpinion(value: number): string {
  return value > 0 ? `+${value}` : `${value}`;
}

export function projectDiplomaticRelations(
  readView: BoundedReadView,
  options: DiplomaticRelationsProjectionOptions,
): HeatMatrixModel {
  const refused = readView.refused('main');
  if (refused) {
    return { hour: readView.hour, rows: [], columns: [], cell: () => ({ kind: 'diagonal' }), refused };
  }
  const reply = readView.panel<DiplomaticRelationsReply>('main');
  if (!reply) {
    return { hour: readView.hour, rows: [], columns: [], cell: () => ({ kind: 'diagonal' }), empty: 'No diplomatic relations recorded at this date.' };
  }
  const owners = reply.owners ?? [];
  if (owners.length < 2) {
    return {
      hour: readView.hour,
      rows: [],
      columns: [],
      cell: () => ({ kind: 'diagonal' }),
      empty: 'Fewer than two countries are shown at this date.',
      warnings: reply.warnings,
    };
  }

  const entities: MatrixAxisEntity[] = owners.map((owner, idx) => {
    const resolved = resolveEntityFromReadView(
      readView,
      'country',
      owner.tag,
      readView.hour,
      owner.name ?? owner.tag,
      owner.nameMissing ?? null,
      options.surface,
    );
    const groupColor = owner.color ? resolveRecordedColour(owner.color, options.surface, idx) : undefined;
    const group = options.order === 'faction' && owner.factionId
      ? {
          id: owner.factionId,
          label: owner.factionName ?? `Faction ${owner.factionId}`,
          ...(groupColor ? { color: groupColor } : {}),
        }
      : null;
    return {
      id: owner.tag,
      label: resolved.label,
      shortLabel: owner.tag,
      tag: owner.tag,
      ...(resolved.nameMissing ? { nameMissing: resolved.nameMissing } : {}),
      flag: resolved.flag ?? null,
      flagPending: resolved.pending === true,
      group,
      meta: { countryId: owner.countryId, readHour: owner.readHour ?? null },
    };
  });

  const [low, high] = reply.domain ?? [0, 0];
  const n = owners.length;
  const cells = reply.cells ?? [];
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
    if (!raw) return { kind: 'missing', reason: 'Opinion not recorded' };
    if (raw.kind === 'diagonal') return { kind: 'diagonal' };
    if (raw.kind === 'missing') {
      return { kind: 'missing', reason: raw.reason };
    }
    const text = formatOpinion(raw.opinion);
    const color = signedRampColour(raw.opinion, low, high, options.grey, options.negative, options.positive);
    return { kind: 'value', color, text, value: raw.opinion, selectable: true };
  };

  return {
    hour: readView.hour,
    rows: entities,
    columns: entities,
    cell,
    legend: {
      gradient: {
        lowLabel: formatOpinion(low),
        highLabel: formatOpinion(high),
        stops: [options.negative, options.grey, options.positive],
      },
      ...(hasMissing ? { entries: [{ id: 'missing', label: 'Not recorded', color: options.grey, hatched: true }] } : {}),
    },
    tooltip: (row, col, item, rowIndex) => {
      if (item.kind === 'diagonal') return null;
      const owner = owners[rowIndex];
      const readNote = owner?.readHour !== undefined && owner.readHour !== null
        ? `Opinions read ${formatDateOnly(owner.readHour)}`
        : undefined;
      if (item.kind === 'missing') {
        return {
          title: `${row.label} → ${col.label}`,
          hour: readView.hour,
          missingReason: item.reason,
          ...(readNote ? { note: readNote } : {}),
        };
      }
      return {
        title: `${row.label}’s opinion of ${col.label}`,
        hour: readView.hour,
        exactValue: item.kind === 'value' ? (item.text ?? '0') : '0',
        ...(readNote ? { note: readNote } : {}),
      };
    },
    warnings: reply.warnings,
  };
}

export const DIPLOMATIC_RELATIONS_PROJECTION: HeatMatrixProjection<DiplomaticRelationsProjectionOptions> = {
  kind: 'diplomatic-relations',
  family: 'heat-matrix',
  project: projectDiplomaticRelations,
};

registerHeatMatrixProjection(DIPLOMATIC_RELATIONS_PROJECTION);

export const DIPLOMATIC_RELATIONS_DECLARATION: HeatMatrixWidgetDeclaration<DiplomaticRelationsProjectionOptions> = {
  family: 'heat-matrix',
  kind: 'diplomatic-relations',
  label: 'Diplomatic relations',
  subject: 'diplomatic relations',
  emptyCampaign: 'Load a campaign to see diplomatic relations.',
  icon: 'handshake',
  optionFields: OPTIONS,
  countries: { mode: 'multi', labels: false, colours: false, factions: true },
  sources: (ctx): SourcePlan => {
    const order: OpinionOrder = ctx.options.order === 'name' ? 'name' : 'faction';
    const isTop = ctx.countries.top !== null;
    const explicitTags = isTop ? [] : ctx.countries.tags;
    const top = isTop ? (ctx.countries.top ?? 12) : undefined;
    const baseParams: Record<string, unknown> = {
      branchId: ctx.branch.id,
      countries: explicitTags.map(countryFromKey),
      ...(top !== undefined ? { top } : {}),
      order,
    };
    return {
      owner: `diplomatic-relations:${ctx.branch.id}:${order}:${top ?? explicitTags.join(',')}`,
      entries: [
        {
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: isTop
              ? { mode: 'population', form: 'top', limit: top }
              : { mode: 'keyed', keys: explicitTags },
            product: 'diplomatic-relations',
            followQuery: { product: 'diplomatic-relations', params: baseParams },
            params: baseParams,
          },
        },
      ],
    };
  },
  projection: DIPLOMATIC_RELATIONS_PROJECTION,
  projectionOptions: ctx => ({
    order: ctx.options.order === 'name' ? 'name' : 'faction',
    surface: '#11161f',
    grey: '#2f3847',
    negative: '#d95926',
    positive: '#199e70',
  }),
  jumpToDate: true,
};
