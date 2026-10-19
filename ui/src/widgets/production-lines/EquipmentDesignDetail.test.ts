import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('equipment design sections preserve references, zeroes, contributors and named refusals', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { validateDesignDetail, DesignSections } = await vite.ssrLoadModule('/ui/src/widgets/production-lines/EquipmentDesignDetail.tsx');
    const reply = { branchId: '1', hour: 22, warnings: [], designs: [{ id: '18446744073709551615', name: 'Synthetic', creator: 5000, obsolete: false }],
      sections: [{ title: 'Stat contributors', fields: [{ label: 'Synthetic company / soft_attack / factor', value: '0.00000 × 1.00000', refusal: null }] },
        { title: 'Costs and XP', fields: [{ label: 'Refit', value: null, refusal: 'missing country template discount' }] }] };
    assert.equal(validateDesignDetail(reply).designs[0].id, '18446744073709551615');
    const html = renderToStaticMarkup(createElement(DesignSections, { sections: reply.sections }));
    assert.match(html, /Stat contributors/);
    assert.match(html, /0\.00000 × 1\.00000/);
    assert.match(html, /Unavailable: missing country template discount/);
    assert.throws(() => validateDesignDetail({ ...reply, sections: [{ title: 'Costs', fields: [{ label: 'Refit', value: '0', refusal: 'missing' }] }] }), /Invalid equipment/);
    assert.throws(() => validateDesignDetail({ ...reply, designs: [{ ...reply.designs[0], id: 18446744073709551615n }] }), /Invalid equipment/);
  } finally { await vite.close(); }
});
