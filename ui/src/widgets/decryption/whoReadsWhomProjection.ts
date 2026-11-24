import type { HeatMatrixWidgetDeclaration } from '../../families/heat/matrixDeclaration.ts';
import type { HeatMatrixModel, HeatMatrixProjection, MatrixAxisEntity, MatrixCell, MatrixCells } from '../../families/heat/matrixModel.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { formatProgress } from '../../families/valueFormat.ts';
import type { CipherCounts, CipherPair } from '../../generated/dashboard';
import { pairKey, percentOf } from './decryptionData.ts';
import {
  decryptionFilterOptions,
  decryptionNextChange,
  decryptionSources,
  brokenText,
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
    flag: country.flag,
    ...(trailing ? { trailing } : {}),
  };
}

function ownerTotal(counts: CipherCounts): string {
  return [
    `Broken ${counts.broken}`,
    `Decrypting ${counts.decrypting}`,
    ...(counts.paused ? [`Paused ${counts.paused}`] : []),
    ...(counts.breakUnknown ? [`Not known ${counts.breakUnknown}`] : []),
  ].join(' · ');
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

export const whoReadsWhomProjection: HeatMatrixProjection<DecryptionFilterOptions> = registerFamilyProjection({
  family: 'heat-matrix',
  kind: 'decryption',
  nextChange(readView, options): number {
    return decryptionNextChange(options.filter, readView.hour);
  },
  project(readView, options): HeatMatrixModel {
    const none: MatrixCells = { present: new Map(), absent: NO_ATTEMPT, self: SELF };
    const frame = readDecryption(readView, options);
    if (frame.kind === 'refused') return { hour: readView.hour, rows: [], columns: [], cells: none, refused: frame.reason };
    if (frame.kind === 'pending') return { hour: readView.hour, rows: [], columns: [], cells: none };
    if (frame.kind === 'empty') {
      return {
        hour: readView.hour,
        rows: [],
        columns: [],
        cells: none,
        empty: frame.empty,
        ...(frame.emptyDetail ? { emptyDetail: frame.emptyDetail } : {}),
        emptyKind: frame.emptyKind,
        ...(frame.summaryText ? { summaryText: frame.summaryText } : {}),
        warnings: frame.warnings,
      };
    }

    const { pairs, country, reply, live } = frame;
    const byOwner = new Map<number, { counts: CipherCounts; country: ResolvedCountry }>();
    const byCell = new Map<string, CipherPair>();
    for (const pair of pairs) {
      const held = byOwner.get(pair.owner.id) ?? { counts: { broken: 0, decrypting: 0, paused: 0, breakUnknown: 0 }, country: country(pair.owner) };
      held.counts[pair.status]++;
      byOwner.set(pair.owner.id, held);
      byCell.set(`${pair.owner.id}:${pair.target.id}`, pair);
    }
    const targetsById = new Map(pairs.map(pair => [pair.target.id, pair.target]));
    const ownerIds = firstSeen(pairs, 'owner');
    const targetIds = firstSeen(pairs, 'target');
    const rows = ownerIds.map(id => {
      const held = byOwner.get(id)!;
      return entity(held.country, ownerTotal(held.counts));
    });
    const columns = targetIds.map(id => entity(country(targetsById.get(id)!)));

    const width = targetIds.length;
    const rowOf = new Map(ownerIds.map((id, index) => [id, index]));
    const columnOf = new Map(targetIds.map((id, index) => [id, index]));
    const present = new Map<number, MatrixCell>();
    for (const pair of pairs) {
      if (pair.owner.id === pair.target.id) continue;
      const owner = byOwner.get(pair.owner.id)!.country;
      const at = rowOf.get(pair.owner.id)! * width + columnOf.get(pair.target.id)!;
      const target = country(pair.target);
      const numbers = readCipherNumbers(live, pair);
      const percent = percentOf(pair, numbers);
      const tooltip = {
        title: `${owner.name} → ${target.name}`,
        hour: readView.hour,
        items: pairTooltipItems(pair, reply, numbers, owner, target, percent),
      };
      const color = owner.colour ?? '';
      if (pair.status === 'broken') {
        const date = brokenText(pair.brokenAt) ?? 'Date not recorded';
        present.set(at, { kind: 'value', mark: 'square', color, tooltip, audit: `${pairKey(pair)}|broken|${date}`, selectable: true });
      } else if (percent === null) {
        const reason = pair.thresholdUnknown ?? (numbers.missing || 'Progress not recorded');
        present.set(at, { kind: 'missing', reason, tooltip, audit: `${pairKey(pair)}|not-recorded`, selectable: true });
      } else {
        present.set(at, {
          kind: 'value',
          mark: 'ring',
          fraction: percent / 100,
          color,
          tooltip,
          audit: `${pairKey(pair)}|${formatProgress(percent / 100)}`,
          selectable: true,
        });
      }
    }
    return {
      hour: readView.hour,
      corner: 'Reads',
      rows,
      columns,
      cells: { present, absent: NO_ATTEMPT, self: SELF },
      summaryText: frame.summaryText,
      notes: frame.notes,
      warnings: reply.warnings,
    };
  },
}, 'matrix');

export const whoReadsWhomDeclaration: HeatMatrixWidgetDeclaration<DecryptionFilterOptions> = {
  family: 'heat-matrix',
  kind: 'decryption',
  label: 'Who reads whom',
  subject: 'decryption',
  emptyCampaign: 'Load a campaign to see who reads whom.',
  icon: 'lock',
  countries: { mode: 'filter' },
  sources: decryptionSources,
  projection: whoReadsWhomProjection,
  projectionOptions: decryptionFilterOptions,
  jumpToDate: true,
  keyByBranch: true,
};
