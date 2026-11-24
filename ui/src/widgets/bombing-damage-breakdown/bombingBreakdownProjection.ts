import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles.ts';
import {
  ledgerComplete,
  ledgerCountryAt,
  ledgerLabel,
  ledgerScale,
  militaryAirIndex,
  partAmount,
  type MilitaryAirLedger,
} from '../../dashboard/militaryAirLedger.ts';
import type { PlaceStateName } from '../../generated/dashboard';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';

export interface BombingPanelData {
  readonly data: MilitaryAirLedger;
  readonly stateNames: Readonly<Record<number, PlaceStateName>>;
  readonly stateNamesPending: boolean;
  readonly stateNamesError: string | null;
}

export interface BombingBreakdownProjectionOptions {
  readonly tag: string;
  readonly identity?: string;
  readonly from: number;
  readonly dealt: boolean;
  readonly levels: readonly boolean[];
}

export const bombingBreakdownProjection: BreakdownProjection<BombingBreakdownProjectionOptions> = {
  kind: 'bombing-damage-breakdown',
  project(readView: BoundedReadView, options: BombingBreakdownProjectionOptions): BreakdownModel {
    const panel = readView.panel<BombingPanelData>('bombing');
    const hour = Math.floor(readView.hour);
    const rootLabel = options.dealt ? 'Damage inflicted' : 'Damage suffered';
    const verb = options.dealt ? 'Inflicted' : 'Suffered';

    if (!panel) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: '0', unit: 'levels', verb },
        parts: [],
        empty: 'No bombing damage recorded.',
      };
    }

    const { data, stateNames, stateNamesPending, stateNamesError } = panel;
    if (!data.available) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: 'Not recorded', unit: 'levels', verb },
        parts: [],
        empty: 'Bombing damage was not recorded.',
        emptyKind: 'unavailable',
      };
    }

    const index = militaryAirIndex(data);
    const shown = ledgerCountryAt(index, options.tag, options.from, hour, options.identity);
    if (!shown) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: 'Not recorded', unit: 'levels', verb },
        parts: [],
        empty: 'This country’s bombing damage was not recorded at this moment.',
        emptyKind: 'unavailable',
      };
    }

    const scale = ledgerScale('bombing');
    const buildings = new Map(data.buildings.map(item => [item.id, item]));
    const complete = ledgerComplete(data.observed, options.from, hour, data.incomplete);
    const periodLabel = `${verb} since ${formatDateOnly(options.from)}`;

    const leaves: RawBreakdownLeaf[] = [];
    for (const leaf of shown.series.parts) {
      if (leaf.dealt !== options.dealt || leaf.buildingId === null) continue;
      const exact = partAmount(leaf, options.from, hour);
      if (exact === 0n) continue;
      const building = leaf.buildingToken == null ? null : buildings.get(leaf.buildingToken);
      const state = leaf.state == null || leaf.state === 0 ? null : stateNames[leaf.state];
      const buildingLabel = contentTitle(
        building?.name,
        leaf.buildingToken,
        leaf.buildingToken == null ? 'Building type not recorded' : 'Building name unavailable',
      );
      const stateLabel = leaf.state === 0
        ? 'State unknown'
        : contentTitle(stateNamesError ?? state?.name, leaf.state, state?.missing ?? 'State name not recorded', stateNamesPending);

      leaves.push({
        path: [
          { key: `building:${leaf.buildingToken ?? leaf.buildingId}`, name: buildingLabel, note: contentTitleHint(leaf.buildingToken) },
          { key: `state:${leaf.state}`, name: stateLabel },
        ],
        value: Number(exact) / scale,
        exact,
        partial: !complete,
        partialReason: !complete ? 'Some bombing damage may be missing.' : undefined,
      });
    }

    const totalExact = options.dealt ? shown.dealt : shown.taken;
    if (!leaves.length && totalExact === 0n) {
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: complete, value: 0, text: '0', unit: 'levels', verb },
        parts: [],
        empty: options.dealt ? 'No bombing damage inflicted in this period.' : 'No bombing damage suffered in this period.',
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: (value, exact) => ledgerLabel(exact ?? BigInt(Math.round(value * scale)), 'bombing'),
      unit: 'levels',
      verb,
      authoritativeTotal: {
        value: Number(totalExact) / scale,
        exact: totalExact,
        atLeast: !complete,
        partial: !complete,
        reason: !complete ? 'Some bombing damage may be missing.' : undefined,
        shortReason: !complete ? 'Partial coverage' : undefined,
      },
    });

    return {
      hour,
      rootLabel,
      periodLabel,
      total,
      parts,
      warnings: data.warnings,
    };
  },
};

registerBreakdownProjection(bombingBreakdownProjection);
