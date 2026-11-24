import type { TooltipRow } from '../../charts/tooltip';
import type { TooltipListItem } from '../host/tooltips.ts';
import type { ArtResult } from '../plan.ts';
import type { BreakdownPart, BreakdownTotal } from './model.ts';

export interface BreakdownLeafKey {
  id: string;
  label: string;
  icon?: string | null;
  countryFlag?: TooltipRow['countryFlag'];
  colour?: string;
  last?: boolean;
  keepZero?: boolean;
  stretch?: number | string | null;
}

export interface BreakdownLeafItem {
  path: readonly BreakdownLeafKey[];
  value: number | null;
  exact?: bigint | null;
  exactDenominator?: bigint;
  unknownValue?: boolean;
  unknownLevels?: readonly boolean[];
  localUnknownLevels?: readonly boolean[];
  present?: boolean;
  final?: number | null;
  weight?: number;
  finalUnknown?: boolean;
  finalUnknownLevels?: readonly boolean[];
  localFinalUnknownLevels?: readonly boolean[];
  unknown?: Readonly<Record<string, number>>;
  note?: string;
  counted?: readonly string[];
  count?: number;
}

export type BreakdownLeafTotal = Pick<
  BreakdownLeafItem,
  'value' | 'exact' | 'exactDenominator' | 'unknownValue' | 'final' | 'finalUnknown' | 'note'
>;

export interface RawPathNode {
  readonly key: string;
  readonly name: string;
  readonly color?: string;
  readonly neutral?: boolean;
  readonly flag?: string;
  readonly flagArt?: ArtResult;
  readonly icon?: string | null;
  readonly subtitle?: string;
  readonly note?: string;
  readonly detailRows?: readonly TooltipListItem[];
  readonly meta?: unknown;
}

export interface RawBreakdownLeaf {
  readonly path: readonly RawPathNode[];
  readonly value: number | null;
  readonly exact?: bigint | null;
  readonly exactDenominator?: bigint;
  readonly notRecorded?: boolean;
  readonly missingReason?: string;
  readonly partial?: boolean;
  readonly partialReason?: string;
  readonly note?: string;
  readonly count?: number;
  readonly countLabel?: string;
  readonly present?: boolean;
}

export interface BuildTreeOptions {
  readonly leaves: readonly RawBreakdownLeaf[];
  readonly levels?: readonly boolean[];
  readonly formatValue: (value: number, exact: bigint | null, denominator: bigint) => string;
  readonly unit: string;
  readonly verb: string;
  readonly authoritativeTotal?: {
    readonly value: number | null;
    readonly exact?: bigint | null;
    readonly denominator?: bigint;
    readonly atLeast?: boolean;
    readonly partial?: boolean;
    readonly reason?: string;
    readonly shortReason?: string;
  };
  readonly partialReason?: string;
  readonly shortReason?: string;
  readonly sortByValue?: boolean;
}

interface TreeAcc {
  readonly key: string;
  name: string;
  ordinal: number;
  color?: string;
  neutral?: boolean;
  flag?: string;
  flagArt?: ArtResult;
  icon?: string | null;
  subtitle?: string;
  note?: string;
  detailRows?: readonly TooltipListItem[];
  meta?: unknown;
  valueSum: number;
  exactSum: bigint;
  exactDenom: bigint;
  hasExact: boolean;
  hasKnown: boolean;
  hasNotRecorded: boolean;
  missingReason?: string;
  partial: boolean;
  partialReason?: string;
  count: number;
  countLabel?: string;
  readonly children: Map<string, TreeAcc>;
}

function addExact(acc: TreeAcc, exact: bigint, denom: bigint): void {
  if (!acc.hasExact) {
    acc.exactSum = exact;
    acc.exactDenom = denom;
    acc.hasExact = true;
    return;
  }
  if (acc.exactDenom === denom) {
    acc.exactSum += exact;
    return;
  }
  acc.exactSum = acc.exactSum * denom + exact * acc.exactDenom;
  acc.exactDenom = acc.exactDenom * denom;
}

export function buildBreakdownTree(options: BuildTreeOptions): {
  readonly parts: readonly BreakdownPart[];
  readonly total: BreakdownTotal;
} {
  const { leaves, levels, formatValue, unit, verb, authoritativeTotal, partialReason, shortReason, sortByValue = true } = options;
  const roots = new Map<string, TreeAcc>();
  let nextRootOrdinal = 0;

  for (const leaf of leaves) {
    if (leaf.present === false) continue;
    const filteredPath = levels
      ? leaf.path.filter((_, idx) => levels[idx] !== false)
      : leaf.path;
    if (!filteredPath.length) continue;

    let map = roots;
    for (let depth = 0; depth < filteredPath.length; depth++) {
      const step = filteredPath[depth]!;
      const nodeKey = depth === 0 ? step.key : `${filteredPath.slice(0, depth + 1).map(s => s.key).join('\u0000')}`;
      let acc = map.get(nodeKey);
      if (!acc) {
        const ordinal = depth === 0 ? nextRootOrdinal++ : map.size;
        acc = {
          key: nodeKey,
          name: step.name,
          ordinal,
          color: step.color,
          neutral: step.neutral,
          flag: step.flag,
          flagArt: step.flagArt,
          icon: step.icon,
          subtitle: step.subtitle,
          note: step.note,
          detailRows: step.detailRows,
          meta: step.meta,
          valueSum: 0,
          exactSum: 0n,
          exactDenom: 1n,
          hasExact: false,
          hasKnown: false,
          hasNotRecorded: false,
          partial: false,
          count: 0,
          children: new Map(),
        };
        map.set(nodeKey, acc);
      } else {
        if (step.color && !acc.color) acc.color = step.color;
        if (step.flag && !acc.flag) acc.flag = step.flag;
        if (step.flagArt && !acc.flagArt) acc.flagArt = step.flagArt;
        if (step.subtitle && !acc.subtitle) acc.subtitle = step.subtitle;
        if (step.note && !acc.note) acc.note = step.note;
        if (step.detailRows && !acc.detailRows) acc.detailRows = step.detailRows;
        if (step.meta !== undefined && acc.meta === undefined) acc.meta = step.meta;
      }

      if (leaf.notRecorded || leaf.value === null) {
        acc.hasNotRecorded = true;
        if (leaf.missingReason && !acc.missingReason) acc.missingReason = leaf.missingReason;
      } else {
        acc.valueSum += leaf.value;
        acc.hasKnown = true;
        if (leaf.exact != null) {
          addExact(acc, leaf.exact, leaf.exactDenominator ?? 1n);
        }
      }
      if (leaf.note && depth === filteredPath.length - 1) {
        if (!acc.note) acc.note = leaf.note;
        else if (!acc.note.includes(leaf.note)) acc.note = `${acc.note} ${leaf.note}`;
      }
      if (leaf.partial) {
        acc.partial = true;
        if (leaf.partialReason && !acc.partialReason) acc.partialReason = leaf.partialReason;
      }
      if (leaf.count) {
        acc.count += leaf.count;
        if (leaf.countLabel) acc.countLabel = leaf.countLabel;
      }
      map = acc.children;
    }
  }

  const rootList = [...roots.values()].filter(acc => acc.hasNotRecorded || acc.valueSum !== 0);
  let knownSum = 0;
  let exactKnownSum = 0n;
  let exactKnownDenom = 1n;
  let hasAnyKnown = false;
  let hasExactKnown = false;
  let anyRootMissing = false;
  let anyRootPartial = false;
  let firstMissingReason: string | undefined;

  for (const acc of rootList) {
    if (acc.hasNotRecorded && !acc.hasKnown) {
      anyRootMissing = true;
      if (!firstMissingReason) firstMissingReason = acc.missingReason ?? `${acc.name} not recorded`;
    } else {
      knownSum += acc.valueSum;
      hasAnyKnown = true;
      if (acc.hasExact) {
        if (!hasExactKnown) {
          exactKnownSum = acc.exactSum;
          exactKnownDenom = acc.exactDenom;
          hasExactKnown = true;
        } else if (exactKnownDenom === acc.exactDenom) {
          exactKnownSum += acc.exactSum;
        } else {
          exactKnownSum = exactKnownSum * acc.exactDenom + acc.exactSum * exactKnownDenom;
          exactKnownDenom *= acc.exactDenom;
        }
      }
      if (acc.hasNotRecorded) {
        anyRootMissing = true;
        if (!firstMissingReason) firstMissingReason = acc.missingReason ?? `${acc.name} partly not recorded`;
      }
    }
    if (acc.partial) anyRootPartial = true;
  }

  const atLeast = authoritativeTotal?.atLeast ?? anyRootMissing;
  const totalValue = authoritativeTotal?.value !== undefined
    ? (authoritativeTotal.value ?? (hasAnyKnown ? knownSum : null))
    : (hasAnyKnown || !anyRootMissing ? knownSum : null);
  const totalExact = authoritativeTotal?.exact !== undefined
    ? authoritativeTotal.exact
    : (hasExactKnown ? exactKnownSum : null);
  const totalDenom = authoritativeTotal?.denominator ?? exactKnownDenom;
  const totalText = totalValue !== null ? formatValue(totalValue, totalExact, totalDenom) : 'Not recorded';
  const totalPartial = authoritativeTotal?.partial ?? (anyRootPartial || Boolean(partialReason));

  const shareDenom = !atLeast && totalValue !== null && totalValue > 0 ? totalValue : null;

  const finalize = (acc: TreeAcc, parentShareDenom: number | null): BreakdownPart => {
    const isMissingOnly = acc.hasNotRecorded && !acc.hasKnown;
    const isPartialNode = acc.partial || (acc.hasNotRecorded && acc.hasKnown);
    const val = isMissingOnly ? null : acc.valueSum;
    const text = val === null ? 'Not recorded' : formatValue(val, acc.hasExact ? acc.exactSum : null, acc.exactDenom);
    const share = val !== null && val > 0 && parentShareDenom !== null && parentShareDenom > 0
      ? val / parentShareDenom
      : null;

    const childAccs = [...acc.children.values()].filter(ch => ch.hasNotRecorded || ch.valueSum !== 0);
    if (sortByValue) {
      childAccs.sort((a, b) => {
        const aMiss = a.hasNotRecorded && !a.hasKnown ? 1 : 0;
        const bMiss = b.hasNotRecorded && !b.hasKnown ? 1 : 0;
        if (aMiss !== bMiss) return aMiss - bMiss;
        if (a.neutral !== b.neutral) return a.neutral ? 1 : -1;
        if (b.valueSum !== a.valueSum) return b.valueSum - a.valueSum;
        return a.ordinal - b.ordinal;
      });
    }
    const childDenom = !atLeast && !acc.hasNotRecorded && val !== null && val > 0 ? val : null;
    const children = childAccs.length ? childAccs.map(ch => finalize(ch, childDenom)) : undefined;

    return {
      key: acc.key,
      name: acc.name,
      ordinal: acc.ordinal,
      value: val,
      text,
      share,
      ...(acc.color ? { color: acc.color } : {}),
      ...(acc.neutral ? { neutral: true } : {}),
      ...(acc.flag ? { flag: acc.flag } : {}),
      ...(acc.flagArt ? { flagArt: acc.flagArt } : {}),
      ...(acc.icon !== undefined ? { icon: acc.icon } : {}),
      ...(isMissingOnly ? { notRecorded: true, missingReason: acc.missingReason ?? 'Not recorded' } : {}),
      ...(isPartialNode ? { partial: true, partialReason: acc.partialReason ?? acc.missingReason ?? 'Partly not recorded' } : {}),
      ...(acc.count > 0 ? { count: acc.count, countLabel: acc.countLabel } : {}),
      ...(acc.subtitle ? { subtitle: acc.subtitle } : {}),
      ...(acc.note ? { note: acc.note } : {}),
      ...(acc.detailRows ? { detailRows: acc.detailRows } : {}),
      ...(acc.meta !== undefined ? { meta: acc.meta } : {}),
      ...(children ? { children } : {}),
    };
  };

  if (sortByValue) {
    rootList.sort((a, b) => {
      const aMiss = a.hasNotRecorded && !a.hasKnown ? 1 : 0;
      const bMiss = b.hasNotRecorded && !b.hasKnown ? 1 : 0;
      if (aMiss !== bMiss) return aMiss - bMiss;
      if (a.neutral !== b.neutral) return a.neutral ? 1 : -1;
      if (b.valueSum !== a.valueSum) return b.valueSum - a.valueSum;
      return a.ordinal - b.ordinal;
    });
  }

  const parts = rootList.map(acc => finalize(acc, shareDenom));

  return {
    parts,
    total: {
      known: !atLeast && totalValue !== null,
      atLeast,
      partial: totalPartial,
      value: totalValue,
      text: totalText,
      unit,
      verb,
      reason: authoritativeTotal?.reason ?? partialReason ?? firstMissingReason,
      shortReason: authoritativeTotal?.shortReason ?? shortReason ?? (anyRootMissing ? firstMissingReason : undefined),
    },
  };
}
