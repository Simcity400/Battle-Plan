import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { ForceParams } from '../../charts/race/forceRace';
import { usNumber } from '../../charts/numberFormat.ts';
import type { OptionValue } from '../../dashboard/options';

type Options = Readonly<Record<string, OptionValue>>;

export const navalKind = (options: Options) => options.kind === 'ships' || options.kind === 'convoys' ? options.kind : 'both';
export const navalAttribution = (options: Options) => options.attribution === 'sunkBy' ? 'sunkBy' : 'lost';
export const navalPeriod = (options: Options) => options.period === 'month' ? 'month' : 'running';

export function navalRequest(branchId: string, resolved: { tags: readonly string[]; top?: number }, options: Options): ForceParams {
  return { branchId, countries: resolved.top == null ? resolved.tags.map(countryFromKey) : [], top: resolved.top ?? null,
    attribution: navalAttribution(options), kind: navalKind(options), period: navalPeriod(options) };
}

export function navalMeasure(options: Options): string {
  const kind = navalKind(options);
  const what = kind === 'ships' ? 'Ships' : kind === 'convoys' ? 'Convoys' : 'Ships and convoys';
  return `${what} ${navalAttribution(options) === 'lost' ? 'lost' : 'sunk by'}`;
}

export const navalOwnership = (options: Options) => navalPeriod(options) === 'month' ? 'Per month' : 'Running total';

export const formatNavalCount = (value: number) => usNumber.format(value);
