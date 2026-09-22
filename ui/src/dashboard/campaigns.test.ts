import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultTemplate } from './model.ts';
import { campaignSlots, defaultAfterOpen, defaultAfterUnload, includeCampaigns, resolveWidgetCampaign, sameCampaignPath } from './campaigns.ts';
import { parseTemplate } from './storage.ts';
import '../widgets/definitions.ts';

const campaigns = [{ id: 'primary', label: 'Campaign A', path: 'C:/a.battleplan' }, { id: 'second', label: 'Campaign B', path: 'C:/b.battleplan' }];

test('failed loads reserve only transient capacity and never enter saved layouts', () => {
  const template = defaultTemplate();
  for (let i = 0; i < 20; i++) {
    const pending = [{ id: 'primary', label: `Invalid ${i}`, path: `C:/${i}`, loading: true }];
    assert.equal(campaignSlots(template, pending).empty.length, 0);
    assert.equal(includeCampaigns(template, pending), template);
    assert.deepEqual(campaignSlots(template, []).empty.map(role => role.id), ['primary']);
  }
  const pending = Array.from({ length: 12 }, (_, i) => ({ id: i ? `slot${i}` : 'primary', path: `${i}`, label: `${i}`, loading: true }));
  assert.equal(campaignSlots(template, pending).canAdd, false);
  assert.equal(includeCampaigns(template, pending), template);
});

test('saved slots can be rebound at the role and track limits without changing pins', () => {
  const loaded = Array.from({ length: 12 }, (_, i) => ({ id: i ? `slot${i}` : 'primary', path: `${i}`, label: `Campaign ${i}` }));
  const template = includeCampaigns(defaultTemplate(), loaded);
  while (template.tracks.length < 32) template.tracks.push({ ...template.tracks[1], id: `comparison${template.tracks.length}` });
  template.widgets[0] = { ...template.widgets[0], campaign: 'tracks', trackIds: ['track_slot1'], time: { mode: 'pin', windows: { track_slot1: { firstOffset: 5, lastOffset: 10 } } } };
  const restored = parseTemplate(template)!;
  const slots = campaignSlots(restored, loaded.filter(campaign => campaign.id !== 'slot1'));
  assert.deepEqual(slots.empty.map(role => role.id), ['slot1']);
  assert.equal(slots.canAdd, false);
  const rebound = includeCampaigns(restored, [{ id: 'slot1', path: 'new-file', label: 'Reopened' }]);
  assert.deepEqual(rebound.tracks, restored.tracks);
  assert.deepEqual(rebound.widgets, restored.widgets);
  assert.equal(campaignSlots(restored, loaded).empty.length, 0);
});

test('unloading the default chooses the next loaded campaign and clears the last one', () => {
  assert.equal(defaultAfterUnload('primary', 'primary', ['primary', 'second']), 'second');
  assert.equal(defaultAfterUnload('primary', 'second', ['primary', 'second']), 'primary');
  assert.equal(defaultAfterUnload('primary', 'primary', ['primary']), null);
  assert.equal(defaultAfterUnload(null, 'primary', []), null);
});

test('first successful campaign stays default when another campaign opens', () => {
  assert.equal(defaultAfterOpen(null, 'primary'), 'primary');
  assert.equal(defaultAfterOpen('primary', 'second'), 'primary');
  assert.equal(defaultAfterOpen('second', 'primary'), 'second');
  assert.ok(sameCampaignPath('C:\\Campaigns\\A.battleplan', 'c:/campaigns/a.battleplan'));
  assert.ok(sameCampaignPath('C:\\Campaigns\\A.battleplan', '\\\\?\\C:\\Campaigns\\A.battleplan'));
  assert.ok(sameCampaignPath('\\\\server\\share\\A.battleplan', '\\\\?\\UNC\\server\\share\\A.battleplan'));
  assert.equal(sameCampaignPath('C:/a.battleplan', 'C:/b.battleplan'), false);
});

test('default widgets follow default changes while explicit campaign choices remain fixed', () => {
  const template = includeCampaigns(defaultTemplate(), campaigns);
  const widget = template.widgets[0];
  assert.deepEqual(resolveWidgetCampaign(widget, template, 'primary').trackIds, ['main']);
  assert.deepEqual(resolveWidgetCampaign(widget, template, 'second').trackIds, ['track_second']);
  const explicit = { ...widget, campaign: 'tracks' as const, trackIds: ['main'] };
  assert.deepEqual(resolveWidgetCampaign(explicit, template, 'second').trackIds, ['main']);
  assert.deepEqual(resolveWidgetCampaign(widget, template, null).trackIds, []);
  assert.deepEqual(resolveWidgetCampaign(widget, template, 'missing').trackIds, []);
  assert.deepEqual(resolveWidgetCampaign({ ...explicit, trackIds: ['missing'] }, template, 'primary').trackIds, ['missing']);
});

test('campaign slots survive layout restoration without persisting paths or changing saved selections', () => {
  const saved = defaultTemplate();
  const merged = includeCampaigns(saved, campaigns);
  assert.equal(saved.roles.length, 1);
  assert.equal(merged.roles.length, 2);
  assert.equal(includeCampaigns(merged, campaigns), merged);
  merged.widgets[1] = { ...merged.widgets[1], campaign: 'tracks' as const, trackIds: ['track_second'] };
  const parsed = parseTemplate(merged)!;
  assert.equal(parsed.widgets[0].campaign, 'default');
  assert.equal(parsed.widgets[1].campaign, 'tracks');
  assert.deepEqual(parsed.widgets[1].trackIds, ['track_second']);
  assert.equal(JSON.stringify(parsed).includes('.battleplan'), false);
  const legacy = defaultTemplate();
  delete legacy.widgets[0].campaign;
  delete legacy.widgets[1].campaign;
  legacy.widgets[1].time = { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 0 } } };
  assert.equal(parseTemplate(legacy)?.widgets[0].campaign, 'default');
  assert.equal(parseTemplate(legacy)?.widgets[1].campaign, 'tracks');
  assert.equal(parseTemplate({ ...merged, widgets: [{ ...merged.widgets[0], campaign: 'arbitrary' }] }), null);
});
