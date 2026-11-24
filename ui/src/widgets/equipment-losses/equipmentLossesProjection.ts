import type { LossHeadReply, LossesReply } from '../../generated/dashboard';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';
import {
  atHead,
  causeName,
  formatLosses,
  lossItems,
  lossesTotal,
  withShips,
  type LossWindow,
} from './lossesData.ts';

export interface EquipmentLossesProjectionOptions {
  readonly window: LossWindow;
  readonly levels: readonly boolean[];
  readonly useHead: boolean;
}

export const equipmentLossesProjection: BreakdownProjection<EquipmentLossesProjectionOptions> = {
  kind: 'equipment-losses',
  project(readView: BoundedReadView, options: EquipmentLossesProjectionOptions): BreakdownModel {
    const rawReply = readView.panel<LossesReply>('losses');
    const hour = Math.floor(readView.hour);
    const rootLabel = 'All losses';
    if (!rawReply) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: '0', unit: 'units', verb: 'Lost' },
        parts: [],
        empty: 'No equipment losses recorded.',
      };
    }

    const headPanel = options.useHead ? readView.panel<LossHeadReply>('head') : undefined;
    const headLive = options.useHead ? readView.live('head') : undefined;
    const named = new Set(headPanel?.leaves ?? []);
    const activeWindow: LossWindow = {
      ...options.window,
      hour,
    };
    const reply = options.useHead && headPanel && headLive
      ? atHead(rawReply, hour, headLive as never, named)
      : withShips(rawReply, hour);

    const rawItems = lossItems(reply, activeWindow);
    const totalInfo = lossesTotal(rawItems);
    const scale = reply.amountScale || 1;
    const periodLabel = `Lost since ${formatDateOnly(activeWindow.since ?? activeWindow.from ?? activeWindow.first)}`;

    const leaves: RawBreakdownLeaf[] = [];
    for (const item of rawItems) {
      if (item.present === false) continue;
      if (item.path.length === 1 && !item.unknownValue && (item.exact ?? 0n) === 0n) continue;
      leaves.push({
        path: item.path.map(step => ({
          key: step.id,
          name: step.label,
          meta: item.counted ? [...item.counted] : undefined,
        })),
        value: item.value,
        exact: item.exact ?? null,
        notRecorded: item.value === null || item.unknownValue === true,
        missingReason: item.note ?? (item.unknownValue ? 'Not recorded' : undefined),
        partial: item.incomplete === true && item.value !== null,
        partialReason: item.incomplete ? 'Incomplete coverage for this cause.' : undefined,
        note: item.note,
      });
    }

    if (!leaves.length) {
      const unknownOnly = reply.causes.filter(c => c.loss && c.unknown !== null);
      if (unknownOnly.length) {
        for (const c of unknownOnly) {
          leaves.push({
            path: [{ key: `cause:${c.name.key ?? causeName(c)}`, name: causeName(c) }],
            value: null,
            exact: null,
            notRecorded: true,
            missingReason: c.unknown ?? 'Not recorded',
          });
        }
      } else {
        return {
          hour,
          rootLabel,
          periodLabel,
          total: { known: true, value: 0, text: '0', unit: 'units', verb: 'Lost' },
          parts: [],
          empty: 'No equipment losses recorded in this period.',
        };
      }
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: (value, exact) => formatLosses(value, exact, scale),
      unit: 'units',
      verb: 'Lost',
      authoritativeTotal: totalInfo.exact === null
        ? { value: null, atLeast: true, reason: totalInfo.note ?? 'Losses were not fully recorded.' }
        : {
            value: totalInfo.value,
            exact: totalInfo.exact,
            atLeast: totalInfo.incomplete,
            partial: totalInfo.incomplete,
            reason: totalInfo.incomplete ? 'Some causes were not fully recorded.' : undefined,
            shortReason: totalInfo.incomplete ? 'Partial coverage' : undefined,
          },
    });

    return {
      hour,
      rootLabel,
      periodLabel,
      total,
      parts,
      warnings: [...reply.warnings, ...(headPanel?.warnings ?? [])],
    };
  },
};

registerBreakdownProjection(equipmentLossesProjection);
