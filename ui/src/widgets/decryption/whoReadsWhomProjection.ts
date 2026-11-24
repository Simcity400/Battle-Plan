import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import type { HeatMatrixModel, HeatMatrixProjection, MatrixAxisEntity, MatrixCell } from '../../families/heat/matrixModel.ts';
import { registerHeatMatrixProjection } from '../../families/heat/matrixRegistry.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { CipherPair } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { pairKey, percentOf } from './decryptionData.ts';
import {
  decryptionFilterOptions,
  decryptionNextChange,
  decryptionSources,
  pairTooltipItems,
  readCipherNumbers,
  readDecryption,
  type DecryptionFilterOptions,
  type ResolvedCountry,
} from './decryptionRead.ts';

const NO_ATTEMPT: MatrixCell = { kind: 'empty', mark: 'dot', label: 'No attempt' };
const SELF: MatrixCell = { kind: 'diagonal' };

function entity(country: ResolvedCountry, trailing?: string): MatrixAxisEntity {
  return {
    id: country.key,
    label: country.name,
    tag: country.tag,
    flag: country.flagHandle,
    ...(trailing ? { trailing } : {}),
  };
}

function firstSeen(pairs: readonly CipherPair[], side: 'owner' | 'target'): number[] {
  const ids: number[] = [];
  const seen = new Set<number>();
  for (const pair of pairs) {
    const id = pair[side].id;
    if (!seen.has(id)) {
      seen.add(id);
      ids.push(id);
    }
  }
  return ids;
}

export const whoReadsWhomProjection: HeatMatrixProjection<DecryptionFilterOptions> = registerHeatMatrixProjection({
  family: 'heat-matrix',
  kind: 'decryption',
  nextChange(readView, options): number {
    return decryptionNextChange(options.filter, readView.hour);
  },
  project(readView, options): HeatMatrixModel {
    const none = () => NO_ATTEMPT;
    const frame = readDecryption(readView, options);
    if (frame.kind === 'refused') return { hour: readView.hour, rows: [], columns: [], cell: none, refused: frame.reason };
    if (frame.kind === 'pending') return { hour: readView.hour, rows: [], columns: [], cell: none };
    if (frame.kind === 'empty') {
      return {
        hour: readView.hour,
        rows: [],
        columns: [],
        cell: none,
        empty: frame.empty,
        ...(frame.emptyDetail ? { emptyDetail: frame.emptyDetail } : {}),
        emptyKind: frame.emptyKind,
        ...(frame.summaryText ? { summaryText: frame.summaryText } : {}),
        warnings: frame.warnings,
      };
    }

    const { pairs, country, reply, live } = frame;
    const byOwner = new Map<number, { broken: number; decrypting: number; country: ResolvedCountry }>();
    const byCell = new Map<string, CipherPair>();
    for (const pair of pairs) {
      const held = byOwner.get(pair.owner.id) ?? { broken: 0, decrypting: 0, country: country(pair.owner) };
      if (pair.broken === true) held.broken++;
      else held.decrypting++;
      byOwner.set(pair.owner.id, held);
      byCell.set(`${pair.owner.id}:${pair.target.id}`, pair);
    }
    const targetsById = new Map(pairs.map(pair => [pair.target.id, pair.target]));
    const ownerIds = firstSeen(pairs, 'owner');
    const targetIds = firstSeen(pairs, 'target');
    const rows = ownerIds.map(id => {
      const held = byOwner.get(id)!;
      return entity(held.country, `Broken ${held.broken} · Decrypting ${held.decrypting}`);
    });
    const columns = targetIds.map(id => entity(country(targetsById.get(id)!)));

    const cells: MatrixCell[] = [];
    for (const ownerId of ownerIds) {
      const owner = byOwner.get(ownerId)!.country;
      for (const targetId of targetIds) {
        if (ownerId === targetId) {
          cells.push(SELF);
          continue;
        }
        const pair = byCell.get(`${ownerId}:${targetId}`);
        if (!pair) {
          cells.push(NO_ATTEMPT);
          continue;
        }
        const target = country(pair.target);
        const numbers = readCipherNumbers(live, pair);
        const percent = percentOf(pair, numbers);
        const tooltip = {
          title: `${owner.name} → ${target.name}`,
          hour: readView.hour,
          items: pairTooltipItems(pair, reply, numbers, owner, target, percent),
        };
        const color = owner.colour ?? '';
        if (pair.broken === true) {
          const date = pair.brokenAt ? formatDateOnly(pair.brokenAt.hour) : 'Date not recorded';
          cells.push({ kind: 'value', mark: 'square', color, tooltip, audit: `${pairKey(pair)}|broken|${date}`, selectable: true });
        } else if (percent === null) {
          const reason = pair.thresholdUnknown ?? (numbers.missing || 'Progress not recorded');
          cells.push({ kind: 'missing', reason, tooltip, audit: `${pairKey(pair)}|not-recorded`, selectable: true });
        } else {
          cells.push({
            kind: 'value',
            mark: 'ring',
            fraction: percent / 100,
            color,
            tooltip,
            audit: `${pairKey(pair)}|${percent}%`,
            selectable: true,
          });
        }
      }
    }
    const width = targetIds.length;
    return {
      hour: readView.hour,
      corner: 'Reads',
      rows,
      columns,
      cell: (rowIndex, columnIndex) => cells[rowIndex * width + columnIndex] ?? NO_ATTEMPT,
      summaryText: frame.summaryText,
      notes: frame.notes,
      warnings: reply.warnings,
    };
  },
});

export const whoReadsWhomDeclaration: HeatMatrixWidgetDeclaration<DecryptionFilterOptions> = {
  family: 'heat-matrix',
  kind: 'decryption',
  label: 'Who reads whom',
  subject: 'decryption',
  emptyCampaign: 'Load a campaign to see its decryption races.',
  icon: 'lock',
  optionFields: OPTIONS,
  countries: { mode: 'filter', labels: true, colours: true },
  sources: decryptionSources,
  projection: whoReadsWhomProjection,
  projectionOptions: decryptionFilterOptions,
  jumpToDate: true,
  keyByBranch: true,
};
