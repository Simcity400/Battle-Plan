import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { canvasPage } from '../../../tools/canvas-page.ts';

test('unavailable widget flags preserve eager slots and borders when styles load later', async () => {
  const styles = await Promise.all(['../dashboard/dashboard.css', '../charts/log/log.css', '../widgets/espionage-log/espionageLog.css',
    '../widgets/aces/aces.css', '../widgets/battles/battles.css', '../widgets/peace-deals/peaceDeals.css']
    .map(path => readFile(new URL(path, import.meta.url), 'utf8')));
  const { result } = await canvasPage(`
try {
  const styles = ${JSON.stringify(styles)};
  const root = document.createElement('div');
  root.style.cssText = '--line-soft: #456789; --space-1: 4px; --space-4: 12px; --space-5: 16px';
  root.innerHTML = '<div class="espionage-log"><span class="log-text"><span class="country-flag country-flag-unavailable log-flag"></span><img class="country-flag-art log-flag" alt=""></span></div>'
    + '<div class="aces-widget"><div class="aces-who"><small><span class="country-flag country-flag-unavailable aces-flag"></span><img class="country-flag-art aces-flag" alt=""></small></div></div>'
    + '<div class="battle-country"><span class="country-flag country-flag-unavailable log-flag"></span><img class="country-flag-art log-flag" alt=""></div>'
    + '<div class="peace-deals"><span class="peace-country"><span class="country-flag country-flag-unavailable peace-flag"></span><img class="country-flag-art peace-flag" alt=""></span>'
    + '<div class="peace-sides"><span class="peace-country"><span class="country-flag country-flag-unavailable peace-flag"></span><img class="country-flag-art peace-flag" alt=""></span></div></div>';
  document.body.append(root);
  const slots = [];
  for (const order of [[1, 2, 3, 4, 5, 0], [0, 1, 2, 3, 4, 5]]) {
    const sheets = order.map(index => { const style = document.createElement('style'); style.textContent = styles[index]; document.head.append(style); return style; });
    slots.push([...root.querySelectorAll('.log-flag,.aces-flag,.peace-flag')].map(node => { const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
      return { width: rect.width, height: rect.height, radius: style.borderRadius, shadow: style.boxShadow }; }));
    sheets.forEach(style => style.remove());
  }
  finish({ slots });
} catch (error) { finish({ error: String(error.stack || error) }); }
`);
  const slots = result.slots as { width: number; height: number; radius: string; shadow: string }[][];
  assert.deepEqual(slots[1], slots[0], 'later widget CSS preserves eager placeholder geometry and borders');
  assert.deepEqual(slots[0].map(slot => [slot.width, slot.height]),
    [[25, 16], [18, 12], [25, 16], [15, 10], [20, 12], [20, 12], [20, 14], [20, 14], [20, 14], [20, 14]]);
});
