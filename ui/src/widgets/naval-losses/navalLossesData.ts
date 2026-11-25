import type { OptionValue } from '../../dashboard/options';

type Options = Readonly<Record<string, OptionValue>>;

export const navalKind = (options: Options) => options.kind === 'ships' || options.kind === 'convoys' ? options.kind : 'both';
export const navalAttribution = (options: Options) => options.attribution === 'sunkBy' ? 'sunkBy' : 'lost';
export const navalPeriod = (options: Options) => options.period === 'month' ? 'month' : 'running';

export function navalMeasure(options: Options): string {
  const kind = navalKind(options);
  const what = kind === 'ships' ? 'Ships' : kind === 'convoys' ? 'Convoys' : 'Ships and convoys';
  return `${what} ${navalAttribution(options) === 'lost' ? 'lost' : 'sunk by'}`;
}
