import { contentKey, contentSet, setLimit, storableToken, toggleContentSet, UNSTORABLE } from './options.ts';

/** Choosing content with in-widget chips (a content-set option shown as chips above
 * a chart, such as resources or equipment types): which tokens are shown, the
 * automatic choice until the user picks, and what clicking one chip does. The
 * settings panel stores the same list, so both always agree. */

/** What the recording offers for choosing: how many lines a token draws here (none
 * when this campaign lacks it; a token can stand for several things, such as a
 * resource declared twice), what one of them is called, and what a chosen thing is
 * called in the "one always stays chosen" reason. */
export interface ChoiceSource {
  lines: (token: string) => number;
  name: (token: string) => string;
  noun: string;
}

/** The stored token of a choice whose identity holds only inside one campaign, such
 * as an equipment variant's handle: the campaign's key (`ReplayMetadata.campaignKey`,
 * shared by every recording of the campaign) before the token. A widget's options
 * travel with the layout across campaigns, so a choice made while one campaign is
 * loaded then never selects another campaign's unrelated item reusing the identity;
 * it is kept, as other campaigns' tokens are. Storable wherever the token is and the
 * key is (hex). */
export const campaignToken = (campaignKey: string, token: string) => `${campaignKey}:${token}`;

/** A choice source over content keys, as one over the tokens a setting stores
 * (`setToken`): each token is read back with `contentKey`. Chips offer
 * `setToken(key)` and `chosenShown`/`toggleChosen` work on the stored tokens through
 * it, so a key with characters a setting cannot store (such as `mod/rifles`) can be
 * chosen like any other. */
export const storedChoices = (source: ChoiceSource): ChoiceSource => ({
  lines: token => source.lines(contentKey(token)),
  name: token => source.name(contentKey(token)),
  noun: source.noun,
});

/** What clicking one in-widget chip does: the tokens to store, or why it cannot. */
export type ChipAction = { value: string } | { reason: string };

/** The token with the largest `size` among those recorded, else the first token:
 * a chart's choice until the user picks. Ties keep the first. */
export function largestToken(items: readonly { token: string; size: bigint; recorded: boolean }[]): string | null {
  let best: { token: string; size: bigint } | null = null;
  for (const item of items) if (item.recorded && (!best || item.size > best.size)) best = item;
  return best?.token ?? items[0]?.token ?? null;
}

/** The chosen tokens this recording has, in the order chosen; the automatic choice
 * (one token or several) when none are chosen or none are recorded here. */
export function chosenShown(source: Pick<ChoiceSource, 'lines'>, chosen: readonly string[], automatic: string | readonly string[] | null): string[] {
  const known = chosen.filter(token => source.lines(token) > 0);
  if (known.length) return known;
  return automatic == null ? [] : typeof automatic === 'string' ? [automatic] : [...automatic];
}

/** Clicking one in-widget chip: that token toggled in the stored list, as the
 * settings panel's chips do (`toggleContentSet`). With nothing stored, the list
 * (or none of it in this campaign) starts from the automatic choice shown, when that
 * can be stored at all, keeping any stored tokens this campaign lacks: choosing
 * another never requires storing one that cannot be. Tokens this campaign lacks,
 * such as another campaign's modded content, are kept and count toward `max`. A
 * click that cannot act says why: its token cannot be stored, the limit is reached,
 * or it would leave nothing this campaign shows. `max` also bounds the lines drawn,
 * since a token can draw several. */
export function toggleChosen(source: ChoiceSource, stored: readonly string[], shown: readonly string[], token: string, max: number): ChipAction {
  // Toggle from what is shown: when no stored token is in this campaign, the automatic
  // choice is what the user sees, so it joins the stored tokens (kept for other campaigns).
  const storedHere = stored.some(item => source.lines(item) > 0);
  const base = storedHere ? stored : [...stored, ...shown.filter(item => storableToken(item) && !stored.includes(item))];
  if (!base.includes(token) && !storableToken(token)) return { reason: UNSTORABLE };
  const next = toggleContentSet(base.join(' '), token, max);
  if (next == null) return { reason: `At most ${setLimit(max)} can be chosen` };
  const lines = contentSet(next).reduce((sum, item) => sum + source.lines(item), 0);
  if (!lines) return { reason: `One ${source.noun} always stays chosen` };
  if (!base.includes(token) && lines > max) {
    return { reason: `At most ${max} lines can be drawn; ${source.name(token)} adds ${source.lines(token)}` };
  }
  return { value: next };
}
