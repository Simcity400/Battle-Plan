import type { GrantInventory } from '../../generated/dashboard';
import { contentTitle } from '../../dashboard/contentTitles';
import { researchNumber } from '../research/researchBreakdown';

export default function ResearchGrants({ inventory }: { inventory?: GrantInventory }) {
  if (!inventory) return <p role="status">Research bonuses are still being prepared.</p>;
  if (inventory.unavailable) return <p role="status">Research bonuses unavailable: {inventory.unavailable}</p>;
  return <details className="research-grants" open>
    <summary>Research bonuses <span className="pill">{inventory.grants.length} held</span></summary>
    {inventory.grants.length === 0 && <p>No research bonuses held.</p>}
    <ul>{inventory.grants.map((grant, index) => <li key={`${grant.kind}:${grant.ordinal}:${index}`}>
      <b title={grant.token}>{contentTitle(grant.name, grant.token, 'Bonus name unavailable')}</b>
      <span>{grant.uses} uses remaining{grant.activeClaims !== null && ` · ${grant.activeClaims} active claims`}</span>
      <span>{grant.speedBonusRaw !== null && `Research speed ${researchNumber(BigInt(grant.speedBonusRaw) * 100n)}%`}
        {grant.aheadReductionRaw !== null && BigInt(grant.aheadReductionRaw) !== 0n && ` · Ahead of time reduction ${researchNumber(BigInt(grant.aheadReductionRaw))} years`}
        {grant.costReductionRaw !== null && `Cost reduction ${researchNumber(BigInt(grant.costReductionRaw) * 100n)}%`}</span>
      {grant.technologies.length > 0 && <span>Technologies: {grant.technologies.map((restriction, index) => <span key={index} title={restriction.token}>
        {index > 0 && ', '}{contentTitle(restriction.name, restriction.token, 'Technology name unavailable')}</span>)}</span>}
      {grant.categories.length > 0 && <span>Categories: {grant.categories.map((restriction, index) => <span key={index} title={restriction.token}>
        {index > 0 && ', '}{contentTitle(restriction.name, restriction.token, 'Category name unavailable')}</span>)}</span>}
      <small>Grant ID {grant.id}</small>
    </li>)}</ul>
    <small>Next grant ID: {inventory.nextId}</small>
  </details>;
}
