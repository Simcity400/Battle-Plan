import assert from 'node:assert/strict';
import test from 'node:test';
import { saveCampaignFlag, settingsFlags, shareCampaignFlags } from './campaignFlags.ts';
import { loadCampaignSettings, parseCampaignSettings, setCampaignFlag, setFactionStyle } from './campaignSettings.ts';

test('browser flags stay with this device; desktop flags move once into the shared settings', async () => {
  const key = 'hoi4-replay.campaign-settings.v1';
  const values = new Map<string, string>();
  const storage = { getItem: (item: string) => values.get(item) ?? null, setItem: (item: string, value: string) => { values.set(item, value); } };
  let local = setCampaignFlag(parseCampaignSettings(undefined), 'ab', 'GER');
  local = setFactionStyle(local, 'ab', '1', { colour: '#ff0000' });
  values.set(key, JSON.stringify(local));
  // Without shared settings, a choice is saved with this device's campaign settings.
  assert.equal(saveCampaignFlag('cd', 'mod-country-long-tag', undefined, storage), null);
  assert.deepEqual(settingsFlags(loadCampaignSettings(storage)), { ab: 'GER', cd: 'mod-country-long-tag' });

  const saves: [string, string | null][] = [];
  await shareCampaignFlags({ cd: 'FRA' }, async (campaign, tag) => { saves.push([campaign, tag]); }, storage);
  // The shared choice wins; this device's other choice fills in once.
  assert.deepEqual(saves, [['ab', 'GER']]);
  const after = loadCampaignSettings(storage);
  assert.deepEqual(settingsFlags(after), {}, 'flags leave this device');
  assert.equal(after.campaigns.ab.factions['1'].colour, '#ff0000', 'faction colours stay');
  // Later choices go to the shared settings.
  const reports: (string | null)[] = [];
  assert.equal(saveCampaignFlag('ab', null, error => reports.push(error), storage), null);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(saves.at(-1), ['ab', null]);
  assert.deepEqual(reports, [null]);
  assert.deepEqual(settingsFlags(loadCampaignSettings(storage)), {});
});
