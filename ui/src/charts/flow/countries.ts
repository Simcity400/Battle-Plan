import { mix } from '../../shell/appearance/color.ts';
import { countryLineColours } from '../../dashboard/countryLineColours.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryStyle } from '../../generated/dashboard';
import type { CountryFlagLook } from '../../components/CountryFlag';
import type { ChartTokens } from '../tokens';
import type { FlowNode } from './layout';
import { OTHER } from './sum.ts';
import type { SummedNode } from './sum';

export const flowOtherColour = (tokens: Pick<ChartTokens, 'surface' | 'text'>) => mix(tokens.surface, tokens.text, 0.4);

/** One stable complete country set, independent of node sides and fitted capacity. */
export function countryFlowColours(tags: readonly string[], color: (tag: string) => CountryStyle['color'], surface: string, fixed: boolean): Map<string, string> {
  const styles = new Map([...new Set(tags)].sort(compareTags).map(tag => [tag, { tag, name: null, color: color(tag) }]));
  if (![...styles.values()].some(style => style.color != null)) return new Map();
  const colours = countryLineColours([...styles.keys()], styles, surface, fixed);
  return new Map([...colours].filter(([tag]) => styles.get(tag)?.color != null));
}

/** Country map colours, flags and missing looks are
 * identical for every country flow diagram. Widgets supply only their wording. */
export function countryFlowNodes(nodes: readonly SummedNode[], look: (tag: string) => CountryFlagLook,
  tokens: Pick<ChartTokens, 'surface' | 'text'>, nodeName: (node: SummedNode) => string,
  order: (node: SummedNode) => string, countryColours: ReadonlyMap<string, string>): { nodes: (FlowNode & { countryFlag?: CountryFlagLook })[]; colours: ReadonlyMap<string, string> } {
  const otherColour = flowOtherColour(tokens);
  const colours = new Map(nodes.filter(node => node.tag != null).map(node => [node.id, countryColours.get(node.tag!) ?? otherColour]));
  return { colours, nodes: nodes.map(node => ({ id: node.id, side: node.side, label: nodeName(node), color: colours.get(node.id) ?? otherColour,
    flag: node.tag != null ? look(node.tag).flag ?? null : undefined, countryFlag: node.tag != null ? look(node.tag) : undefined, order: node.tag != null ? order(node) : OTHER, last: node.tag == null })) };
}
