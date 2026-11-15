import { campaignToken } from './chosen.ts';
import { contentKey, setToken } from './options.ts';
import type { OptionField } from './options';

/** Shared semantic settings; the two widgets keep independent saved choices. */
export const AIR_OPTIONS: readonly OptionField[] = [
  { key: 'mode', section: 'options', label: 'Comparison', kind: 'choice', default: 'sides', choices: [
    { value: 'sides', label: 'Per side' }, { value: 'viewpoint', label: 'Country viewpoint' }] },
  { key: 'viewpoint', section: 'options', label: 'Viewpoint', kind: 'content-choice', automatic: 'Dashboard’s current country', default: '',
    hint: 'A country, or a side using its leader’s view', visibleWhen: o => o.mode === 'viewpoint' },
  { key: 'sides', section: 'options', label: 'Sides compared', kind: 'content-set', automatic: 'All recorded sides', default: '',
    visibleWhen: o => o.mode === 'sides' },
  { key: 'theatres', section: 'options', label: 'Air theatres', kind: 'content-set', automatic: 'All recorded theatres', default: '' },
];

/** Runtime faction, coalition and theatre identities belong to their recording. */
export const airChoiceToken = (campaign: string, key: string) => setToken(campaignToken(campaign, key));
export function airChoiceKey(campaign: string, value: string): string | null {
  const key = contentKey(value), prefix = campaignToken(campaign, '');
  return key.startsWith(prefix) ? key.slice(prefix.length) : null;
}
