import type { CasualtyBreakdownReply } from '../../generated/dashboard';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';
import { exactLosses, lossesAt, warKey } from './casualtiesBreakdownData.ts';

export interface CasualtiesBreakdownProjectionOptions {
  readonly levels: readonly boolean[];
}

export const casualtiesBreakdownProjection: BreakdownProjection<CasualtiesBreakdownProjectionOptions> = {
  kind: 'casualties-breakdown',
  project(readView: BoundedReadView, options: CasualtiesBreakdownProjectionOptions): BreakdownModel {
    const reply = readView.panel<CasualtyBreakdownReply>('casualties');
    const hour = Math.floor(readView.hour);
    const rootLabel = 'Casualties';

    if (!reply) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: '0', unit: 'casualties', verb: 'Lost' },
        parts: [],
        empty: 'No casualties recorded.',
      };
    }

    if (!reply.available) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: 'Not recorded', unit: 'casualties', verb: 'Lost' },
        parts: [],
        empty: 'Casualties by war were not recorded.',
        emptyKind: 'unavailable',
      };
    }

    const snapshot = lossesAt(reply, hour);
    const periodLabel = `As of ${formatDateOnly(hour)}`;

    if (!snapshot) {
      const observed = reply.observed.some(span => span.firstHour <= hour && hour <= span.lastHour);
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: observed, value: observed ? 0 : null, text: observed ? '0' : 'Not recorded', unit: 'casualties', verb: 'Lost' },
        parts: [],
        empty: observed
          ? 'No casualty relations were recorded for this country at this moment.'
          : 'Casualty counters were not recorded at this moment.',
        emptyKind: observed ? 'empty' : 'unavailable',
      };
    }

    const leaves: RawBreakdownLeaf[] = [];
    for (const row of snapshot.rows) {
      if (row.value <= 0n) continue;
      const who = readView.who('country', row.enemyTag, row.enemyHour);
      const enemyName = who.name.status === 'ready' ? who.name.value : row.enemyTag;
      const enemyColor = who.colour?.status === 'ready' ? who.colour.value : undefined;
      const flagArt = who.flagChoice?.status === 'ready' ? readView.art(flagArtHandle(who.flagChoice.value, 'panel')) : undefined;

      const warLabel = row.warId === null
        ? 'Unattributed war'
        : row.war === 'Not recorded'
          ? 'Unnamed war'
          : row.war;

      leaves.push({
        path: [
          { key: warKey(row), name: warLabel, neutral: row.warId === null },
          {
            key: row.enemyIdentity,
            name: enemyName,
            color: enemyColor,
            flag: row.enemyTag,
            flagArt,
            note: row.reason ?? undefined,
          },
        ],
        value: Number(row.value),
        exact: row.value,
        partial: row.incomplete,
        partialReason: row.incomplete ? 'Casualties in this war were partly recorded.' : undefined,
      });
    }

    if (!leaves.length) {
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: !snapshot.incomplete, value: 0, text: '0', unit: 'casualties', verb: 'Lost' },
        parts: [],
        empty: 'No casualties recorded at this moment.',
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: (value, exact) => exactLosses(exact ?? BigInt(Math.round(value))),
      unit: 'casualties',
      verb: 'Lost',
      authoritativeTotal: {
        value: Number(snapshot.total),
        exact: snapshot.total,
        atLeast: snapshot.incomplete,
        partial: snapshot.incomplete,
        reason: snapshot.incomplete ? 'Some casualty counters were not recorded.' : undefined,
        shortReason: snapshot.incomplete ? 'Partial coverage' : undefined,
      },
    });

    return {
      hour,
      rootLabel,
      periodLabel,
      total,
      parts,
      warnings: reply.warnings,
    };
  },
};

registerBreakdownProjection(casualtiesBreakdownProjection);
