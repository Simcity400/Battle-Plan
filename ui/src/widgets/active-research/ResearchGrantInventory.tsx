import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { researchNumber } from '../research/researchBreakdown';
import { grantCounts } from '../research/researchSlots';
import type { GrantInventory } from '../../generated/dashboard';

type Grant = GrantInventory['grants'][number];

function effects(grant: Grant): string {
  const ahead = grant.aheadReductionRaw === null || BigInt(grant.aheadReductionRaw) === 0n ? null : researchNumber(BigInt(grant.aheadReductionRaw));
  return [grant.speedBonusRaw !== null ? `Research speed ${researchNumber(BigInt(grant.speedBonusRaw) * 100n)}%` : null,
    ahead !== null ? `Ahead of time reduction ${ahead} ${ahead === '1' ? 'year' : 'years'}` : null,
    grant.costReductionRaw !== null ? `Cost reduction ${researchNumber(BigInt(grant.costReductionRaw) * 100n)}%` : null].filter(Boolean).join(' · ');
}

export default function ResearchGrants({ inventory }: { inventory: GrantInventory }) {
  if (inventory.unavailable) return <p role="status">Research bonuses unavailable: {inventory.unavailable}</p>;
  return <details className="research-grants" open>
    <summary>Research bonuses <span className="pill">{inventory.grants.length} held</span></summary>
    {inventory.grants.length === 0 && <p>No research bonuses held.</p>}
    <ul>{inventory.grants.map((grant, index) => <li key={`${grant.kind}:${grant.ordinal}:${index}`}>
      <b data-tooltip={contentTitleHint(grant.token)}>{contentTitle(grant.name, grant.token, 'Bonus name unavailable')}</b>
      <span data-prose="" data-paint={`grant:${index}:counts`}>{grantCounts(grant)}</span>
      {effects(grant) !== '' && <span>{effects(grant)}</span>}
      {grant.technologies.length > 0 && <span>Technologies: {grant.technologies.map((restriction, index) => <span key={index} data-tooltip={contentTitleHint(restriction.token)}>
        {index > 0 && ', '}{contentTitle(restriction.name, restriction.token, 'Technology name unavailable')}</span>)}</span>}
      {grant.categories.length > 0 && <span>Categories: {grant.categories.map((restriction, index) => <span key={index} data-tooltip={contentTitleHint(restriction.token)}>
        {index > 0 && ', '}{contentTitle(restriction.name, restriction.token, 'Category name unavailable')}</span>)}</span>}
    </li>)}</ul>
  </details>;
}
