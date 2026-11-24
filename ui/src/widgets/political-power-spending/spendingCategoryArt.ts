import type { ArtHandle } from '../../families/plan.ts';

const CATEGORY_TEXT_ICONS: ReadonlyMap<string, string> = new Map([
  ['advisors', 'pol_idea'],
  ['laws', 'change_government'],
  ['decisions', 'decision_icon_small'],
  ['diplomacy', 'war_score'],
  ['dailyCosts', 'pol_power'],
  ['organisationUpkeep', 'industry_texticon'],
  ['scriptEffects', 'icons_mods'],
  ['autonomy', 'autonomy_texticon'],
  ['scientists', 'research_icon'],
  ['traits', 'doctrine_mastery'],
  ['promotions', 'hq_texticon'],
  ['tactics', 'in_combat'],
  ['raids', 'generic_war_escalation_text_icon'],
]);

const IDEA_CATEGORY = 'ideas:';
const IDEA_TEXT_ICON = 'pol_idea';

export function spendingCategoryArt(category: string): ArtHandle | null {
  const icon = CATEGORY_TEXT_ICONS.get(category) ?? (category.startsWith(IDEA_CATEGORY) ? IDEA_TEXT_ICON : undefined);
  return icon === undefined ? null : { kind: 'sprite', key: icon, frame: 0, text: true };
}
