import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { AgreementRow } from '../widgets/lend-lease-agreements/agreementsData';
import type { ContractRow, OfferRow } from '../widgets/international-market/marketData';

test('market tooltip builders carry dated country failures into the rendered tooltip layer', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { agreementTooltip } = await vite.ssrLoadModule('/ui/src/widgets/lend-lease-agreements/LendLeaseAgreementsWidget.tsx') as typeof import('../widgets/lend-lease-agreements/LendLeaseAgreementsWidget');
    const { contractTooltip, offerTooltip } = await vite.ssrLoadModule('/ui/src/widgets/international-market/InternationalMarketWidget.tsx') as typeof import('../widgets/international-market/InternationalMarketWidget');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts') as typeof import('./queryError');
    const { nameReadFrom, nameReadMetadata } = await vite.ssrLoadModule('/ui/src/dashboard/nameReads.ts') as typeof import('./nameReads');
    const { showTooltip, hideTooltip, countryNameDetails } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts') as typeof import('../charts/tooltip');
    const { default: Layer } = await vite.ssrLoadModule('/ui/src/charts/TooltipLayer.tsx') as typeof import('../charts/TooltipLayer');
    const agreement = { giver: 'A', receiver: 'B', at: 100, items: [], since: null, last: null, ended: false } as unknown as AgreementRow;
    const contract = { seller: 'A', buyer: 'B', equipment: [], prices: [], levels: [], assigned: null, collected: null, deliveries: null } as unknown as ContractRow;
    const offer = { seller: 'A', name: 'Synthetic equipment', amount: 100000n, level: 0 } as unknown as OfferRow;
    for (const error of [new Error('Synthetic dated name decoder failure'), new RefusedError('Synthetic market names were not recorded')]) {
      const label = (tag: string | null) => nameReadMetadata(nameReadFrom(tag === 'A' ? error : 'Recorded partner', true));
      for (const content of [agreementTooltip(agreement, label, 1000), contractTooltip(contract, label, 100, 1000), offerTooltip(offer, label, 100, 1000)]) {
        assert.equal(content.detail, error.message);
        const line = error instanceof RefusedError ? error.text : 'Couldn’t load country name.';
        assert.ok([content.title, content.subtitle].some(text => text?.includes(line)));
        showTooltip('name-failure', content, 0, 0);
        const html = renderToStaticMarkup(createElement(Layer));
        assert.ok(html.includes(`data-detail="${error.message}"`));
        assert.ok(html.includes(line));
        hideTooltip('name-failure');
      }
      const failed = label('A'), other = new Error('Another country decoder failure');
      assert.deepEqual(countryNameDetails(failed, failed, { nameFailure: other }, null), { detail: `${error.message}\n${other.message}` });
    }
    assert.deepEqual(countryNameDetails({ name: 'Recorded country' }, null), {});
  } finally { await vite.close(); }
});
