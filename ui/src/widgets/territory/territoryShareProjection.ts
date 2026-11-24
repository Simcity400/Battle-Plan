import { AREA_UNIT, formatTerritory, MEASURES, type TerritoryMeasure, type TerritoryRule } from '../../dashboard/territoryMeasure.ts';
import type { TerritoryShareReply } from '../../generated/dashboard';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';
import { WORLD_TOTAL, entryCell } from './territoryData.ts';

export interface TerritoryShareProjectionOptions {
  readonly measure: TerritoryMeasure;
  readonly rule: TerritoryRule;
  readonly count: number;
}

export const territoryShareProjection: BreakdownProjection<TerritoryShareProjectionOptions> = {
  kind: 'territory',
  project(readView: BoundedReadView, options: TerritoryShareProjectionOptions): BreakdownModel {
    const reply = readView.panel<TerritoryShareReply>('share');
    const live = readView.live('share');
    const hour = Math.floor(readView.hour);
    const measureName = MEASURES.find(m => m.value === options.measure)?.label ?? options.measure;
    const unit = options.measure === 'area' ? AREA_UNIT : options.measure === 'victoryPoints' ? 'victory points' : options.measure;
    const verb = options.rule === 'owned' ? 'Owned' : 'Controlled';

    if (!reply || !live) {
      return {
        hour,
        rootLabel: measureName,
        total: { known: false, value: null, text: '0', unit, verb },
        parts: [],
        empty: 'No share to show.',
      };
    }

    const totalRaw = live.get(WORLD_TOTAL);
    const totalNum = typeof totalRaw === 'number' ? totalRaw : typeof totalRaw === 'bigint' ? Number(totalRaw) : undefined;
    if (reply.missing !== null || totalNum === undefined) {
      return {
        hour,
        rootLabel: measureName,
        total: { known: false, value: null, text: 'Not recorded', unit, verb },
        parts: [],
        empty: reply.missing ?? live.missing(WORLD_TOTAL) ?? 'Territory is not recorded at this hour.',
        emptyKind: 'unavailable',
      };
    }

    const shown = reply.entries.slice(0, Math.max(0, options.count));
    const leaves: RawBreakdownLeaf[] = [];
    let knownShown = 0;
    let anyMissing = false;

    for (const entry of shown) {
      const cellKey = entryCell(entry.slot);
      const raw = live.get(cellKey);
      const val = typeof raw === 'number' ? raw : typeof raw === 'bigint' ? Number(raw) : null;
      const who = readView.who('country', entry.tag, hour);
      const name = who.name.status === 'ready' ? who.name.value : entry.tag;
      const color = who.colour?.status === 'ready' ? who.colour.value : undefined;
      const flagArt = who.flagChoice?.status === 'ready' ? readView.art(flagArtHandle(who.flagChoice.value, 'panel')) : undefined;

      if (val === null) {
        anyMissing = true;
      } else {
        knownShown += val;
      }
      leaves.push({
        path: [{ key: `entity:${entry.tag}`, name, color, flag: entry.tag, flagArt }],
        value: val,
        exact: val === null ? null : BigInt(Math.trunc(val)),
        notRecorded: val === null,
        missingReason: val === null ? live.missing(cellKey) : undefined,
      });
    }

    if (!anyMissing) {
      const remainder = totalNum - knownShown;
      if (remainder > 0) {
        leaves.push({
          path: [{ key: 'rest', name: 'Rest of the world', neutral: true }],
          value: remainder,
          exact: BigInt(Math.trunc(remainder)),
        });
      }
    }

    const periodLabel = `${verb} on ${formatDateOnly(hour)}`;
    if (!leaves.length) {
      return {
        hour,
        rootLabel: measureName,
        periodLabel,
        total: { known: true, value: 0, text: '0', unit, verb },
        parts: [],
        empty: 'No share to show.',
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      formatValue: value => formatTerritory(value),
      unit,
      verb,
      authoritativeTotal: anyMissing
        ? { value: knownShown, atLeast: true, reason: 'Some countries were not recorded at this hour.' }
        : { value: totalNum, exact: BigInt(Math.trunc(totalNum)) },
    });

    const warnings = [
      ...reply.warnings,
      ...(reply.unnamed ? ['Some territory is held by countries no tag names at this hour; it counts in the rest of the world.'] : []),
    ];

    return {
      hour,
      rootLabel: measureName,
      periodLabel,
      total,
      parts,
      warnings,
    };
  },
};

registerBreakdownProjection(territoryShareProjection);
