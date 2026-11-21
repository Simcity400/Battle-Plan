import assert from 'node:assert/strict';
import test from 'node:test';
import { saveCampaignFlag, settingsFlags, shareCampaignFlags } from './campaignFlags.ts';
import { loadCampaignSettings, parseCampaignSettings, saveCampaignSettings, setCampaignFlag, setFactionStyle } from './campaignSettings.ts';

test('browser flags stay with this device; desktop flags move once into the shared settings', async () => {
  const key = 'hoi4-replay.campaign-settings.v1';
  const values = new Map<string, string>();
  const storage = { getItem: (item: string) => values.get(item) ?? null, setItem: (item: string, value: string) => { values.set(item, value); } };
  let local = setCampaignFlag(parseCampaignSettings(undefined), 'ab', 'GER');
  local = setFactionStyle(local, 'ab', '1', { colour: '#ff0000' });
  values.set(key, JSON.stringify(local));
  assert.equal(saveCampaignFlag('cd', 'mod-country-long-tag', undefined, storage), null);
  assert.deepEqual(settingsFlags(loadCampaignSettings(storage)), { ab: 'GER', cd: 'mod-country-long-tag' });

  const saves: [string, string | null][] = [];
  await shareCampaignFlags({ cd: 'FRA' }, async (campaign, tag) => { saves.push([campaign, tag]); }, storage);
  assert.deepEqual(saves, [['ab', 'GER']]);
  const after = loadCampaignSettings(storage);
  assert.deepEqual(settingsFlags(after), {}, 'flags leave this device');
  assert.equal(after.campaigns.ab.factions['1'].colour, '#ff0000', 'faction colours stay');
  const reports: (string | null)[] = [];
  assert.equal(saveCampaignFlag('ab', null, error => reports.push(error), storage), null);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(saves.at(-1), ['ab', null]);
  assert.deepEqual(reports, [null]);
  assert.deepEqual(settingsFlags(loadCampaignSettings(storage)), {});
});

function device(settings: ReturnType<typeof parseCampaignSettings>) {
  const values = new Map<string, string>();
  const storage = { getItem: (item: string) => values.get(item) ?? null, setItem: (item: string, value: string) => { values.set(item, value); } };
  assert.equal(saveCampaignSettings(settings, storage), null);
  return storage;
}

function heldSave() {
  const saves: [string, string | null][] = [];
  let entered!: () => void, release!: () => void;
  const asked = new Promise<void>(resolve => { entered = resolve; });
  const held = new Promise<void>(resolve => { release = resolve; });
  const save = (campaign: string, tag: string | null) => { saves.push([campaign, tag]); if (saves.length > 1) return Promise.resolve(); entered(); return held; };
  return { saves, asked, release: () => release(), save };
}

test('a flag chosen while this device’s flags move is kept and saved after them', async () => {
  const storage = device(setCampaignFlag(setCampaignFlag(parseCampaignSettings(undefined), 'ab', 'GER'), 'cd', 'ITA'));
  const { saves, asked, release, save } = heldSave();
  const migration = shareCampaignFlags({}, save, storage);
  await asked;
  assert.equal(saveCampaignFlag('ab', 'USA', undefined, storage), null);
  assert.equal(saveCampaignFlag('cd', null, undefined, storage), null);
  release();
  await migration;
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(saves, [['ab', 'GER'], ['ab', 'USA'], ['cd', null]], 'the newer choices are saved last; cd is not moved over them');
  assert.deepEqual(settingsFlags(loadCampaignSettings(storage)), {});
});

test('a flag chosen or cleared while the shared flags are read moves over them', async () => {
  const storage = device(setCampaignFlag(parseCampaignSettings(undefined), 'ab', 'GER'));
  let answer!: (flags: Record<string, string>) => void;
  const read = new Promise<Record<string, string>>(resolve => { answer = resolve; });
  const saves: [string, string | null][] = [];
  const migration = shareCampaignFlags(read, async (campaign, tag) => { saves.push([campaign, tag]); }, storage);
  assert.equal(saveCampaignFlag('ab', 'USA', undefined, storage), null);
  assert.equal(saveCampaignFlag('cd', null, undefined, storage), null);
  answer({ ab: 'GER', cd: 'FRA', ef: 'ITA' });
  await migration;
  assert.deepEqual(saves, [['ab', 'USA'], ['cd', null]], 'the stored GER and FRA are older than these choices; ef is untouched');
  assert.deepEqual(settingsFlags(loadCampaignSettings(storage)), {});
});
