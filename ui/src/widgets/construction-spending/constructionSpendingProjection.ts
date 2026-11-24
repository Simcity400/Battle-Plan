import { placeName, stateNamesOf, typeName } from '../../dashboard/construction.ts';
import type { ConstructionSpendingReply } from '../../generated/dashboard';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';
import { formatSpent, totalAt } from './spendingData.ts';

export interface ConstructionProjectionOptions {
  readonly from: number | null;
  readonly firstHour: number;
  readonly digits: number;
  readonly levels: readonly boolean[];
}

export const constructionSpendingProjection: BreakdownProjection<ConstructionProjectionOptions> = {
  kind: 'construction-spending',
  project(readView: BoundedReadView, options: ConstructionProjectionOptions): BreakdownModel {
    const reply = readView.panel<ConstructionSpendingReply>('spending');
    const hour = Math.floor(readView.hour);
    if (!reply) {
      return {
        hour,
        rootLabel: 'All construction',
        total: { known: false, value: null, text: '0', unit: 'IC', verb: 'Spent' },
        parts: [],
        empty: 'No construction recorded.',
      };
    }
    if (!reply.recorded) {
      return {
        hour,
        rootLabel: 'All construction',
        total: { known: false, value: null, text: 'Not recorded', unit: 'IC', verb: 'Spent' },
        parts: [],
        empty: 'Construction was not recorded.',
        emptyKind: 'unavailable',
      };
    }

    const before = options.from == null ? null : options.from - 1;
    const types = new Map(reply.types.map(t => [t.id, t]));
    const stateAt = stateNamesOf(reply.states);
    const leaves: RawBreakdownLeaf[] = [];
    let anyPartial = false;

    for (const leaf of reply.leaves) {
      let gapCount = 0;
      for (const gap of leaf.unknown) {
        if (gap.knownAt <= hour && (gap.until == null || gap.until > hour)) gapCount++;
      }
      const projects = [...new Set(leaf.projects
        .filter(span => span.firstRead <= hour && (options.from == null || span.until == null || span.until > options.from))
        .map(span => span.project))];

      const now = totalAt(leaf, hour);
      let exact: bigint | null = now;
      if (before != null && now != null) {
        const start = totalAt(leaf, before);
        exact = start == null ? null : now - start;
      }
      const present = projects.length > 0 || (exact != null && exact !== 0n) || gapCount > 0;
      if (!present) continue;

      const type = leaf.building == null ? undefined : types.get(leaf.building);
      const railway = leaf.kind === 'railway';
      const place = placeName({ state: leaf.state, toState: leaf.toState, railway }, stateAt);
      const partial = gapCount > 0;
      if (partial) anyPartial = true;

      leaves.push({
        path: [
          leaf.repair ? { key: 'repair', name: 'Repairs' } : { key: 'new', name: 'New buildings' },
          { key: `type:${type?.id ?? leaf.building ?? 'none'}`, name: typeName(type) },
          { key: `place:${leaf.state ?? ''}>${railway && leaf.toState !== leaf.state ? leaf.toState ?? '' : ''}`, name: place.label, note: place.missing },
        ],
        value: exact === null ? null : Number(exact),
        exact,
        notRecorded: exact === null,
        missingReason: exact === null ? 'Output was not recorded exactly.' : undefined,
        partial,
        partialReason: partial ? 'Some output was not recorded exactly.' : undefined,
        note: place.missing,
        count: projects.length,
        countLabel: 'Projects',
      });
    }

    const periodLabel = `Spent since ${formatDateOnly(options.from ?? options.firstHour)}`;
    const hasMissingSpans = reply.missing.some(span => span.firstHour <= hour && (options.from == null || span.lastHour >= options.from));
    const unowned = reply.unattributed.filter(gap => gap.knownAt <= hour && (options.from == null || gap.until == null || gap.until > options.from));
    const notes: string[] = [];
    if (reply.beforeFirstRead) notes.push(`Earlier spending on ${reply.beforeFirstRead} ${reply.beforeFirstRead === 1 ? 'project' : 'projects'} is not counted.`);
    if (anyPartial) notes.push('Partial amounts leave out output that was not recorded exactly.');
    if (unowned.length) notes.push('Output of projects not traced to a country is not counted.');
    if (hasMissingSpans) notes.push('Output in unrecorded hours is not counted.');

    if (!leaves.length) {
      return {
        hour,
        rootLabel: 'All construction',
        periodLabel,
        total: { known: true, value: 0, text: '0', unit: 'IC', verb: 'Spent' },
        parts: [],
        empty: 'No construction recorded.',
        emptyDetail: notes.length ? notes.join(' ') : undefined,
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: (value, exact) => exact == null ? 'Not recorded' : formatSpent(value, exact, reply.costScale, options.digits),
      unit: 'IC',
      verb: 'Spent',
      partialReason: notes.length ? notes.join(' ') : undefined,
      shortReason: hasMissingSpans ? 'Hours not recorded' : anyPartial ? 'Partial coverage' : undefined,
    });

    return {
      hour,
      rootLabel: 'All construction',
      periodLabel,
      total,
      parts,
      warnings: reply.warnings,
    };
  },
};

registerBreakdownProjection(constructionSpendingProjection);
