import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { Canvas2D } from '../../charts/textWidth.ts';
import { createBoundedReadView, FutureDataError } from '../boundedView.ts';
import type { ArtProvider, FamilyTheme, IdentityProvider } from '../contract.ts';
import { missingArt, PENDING_ART, readyArt } from '../art/types.ts';
import { activeResearchDeclaration, activeResearchProjection } from '../../widgets/active-research/activeResearchProjection.ts';
import { doctrineMasteryProjection } from '../../widgets/doctrine-mastery/doctrineMasteryProjection.ts';
import { decryptionProjection } from '../../widgets/decryption/decryptionProjection.ts';
import { whoReadsWhomProjection } from '../../widgets/decryption/whoReadsWhomProjection.ts';
import { deriveMeterAudit, deriveMeterMirror, deriveMeterNav, findMeterTooltip } from './model.ts';
import { hitMeterModel, layoutMeterModel } from './painter.ts';
import { meterHostAdapter } from './hostAdapter.ts';
import type { DecryptionReply, DoctrineMasteryReply, ResearchSlotsReply } from '../../generated/dashboard';
import type { ResearchData } from '../../widgets/research/types';
import { matrixCellAt } from '../heat/matrixModel.ts';
import { autonomyLevelsProjection } from '../../widgets/autonomy/autonomyLevelsProjection.ts';
import type { AutonomyLevelsReply } from '../../generated/dashboard';

const THEME: FamilyTheme = {
  surface: '#14171d',
  text: '#eceff4',
  muted: '#94a0b2',
  accent: '#4da3ff',
  blue: '#4da3ff',
  line: 'rgba(255, 255, 255, 0.1)',
  lineSoft: 'rgba(255, 255, 255, 0.06)',
  green: '#4cc38a',
  red: '#ff6b6f',
  amber: '#e5a93c',
  grey: '#8b93a1',
  hatch: 'rgba(255, 255, 255, 0.2)',
  lineWidth: 1,
  hatchSpacing: 4,
  font: 'Inter, sans-serif',
  fontSize: { small: 11, body: 12 },
  dark: true,
};

function makeFakeCtx(): Canvas2D {
  return {
    font: '',
    measureText: (s: string) => ({ width: s.length * 7 }),
  } as unknown as Canvas2D;
}

function makeLive(values: Readonly<Record<string, number | bigint | undefined>>, missing: Readonly<Record<string, string>> = {}): LiveValues<number | bigint | string> {
  return {
    get: key => values[key],
    raw: key => {
      const v = values[key];
      return typeof v === 'bigint' ? v : typeof v === 'number' && Number.isSafeInteger(v) ? BigInt(v) : undefined;
    },
    missing: key => missing[key] ?? 'Not recorded',
  };
}

function makeIdentity(countries: Readonly<Record<string, { name: string; colour?: string; flag?: string; absent?: boolean }>>): IdentityProvider {
  return {
    resolve(kind, key, atHour) {
      const entry = countries[key];
      if (kind !== 'country' || !entry) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'ready', value: key },
          tag: { status: 'ready', value: key },
          name: { status: 'absent', reason: 'Name not recorded' },
          colour: { status: 'absent', reason: 'Colour not recorded' },
          flagChoice: { status: 'absent', reason: 'Flag not recorded' },
          validInterval: [atHour, atHour],
          nextChangeAt: Infinity,
        };
      }
      if (entry.absent) {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'absent', reason: 'Country absent' },
          name: { status: 'absent', reason: 'Country absent' },
          colour: { status: 'absent', reason: 'Country absent' },
          flagChoice: { status: 'absent', reason: 'Country absent' },
          validInterval: [atHour, atHour],
          nextChangeAt: Infinity,
        };
      }
      return {
        kind,
        key,
        atHour,
        holder: { status: 'ready', value: key },
        tag: { status: 'ready', value: key },
        name: { status: 'ready', value: entry.name },
        colour: entry.colour ? { status: 'ready', value: entry.colour } : { status: 'absent', reason: 'No colour' },
        flagChoice: entry.flag ? { status: 'ready', value: entry.flag } : { status: 'absent', reason: 'No flag' },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    },
  };
}

test('activeResearch asks for the research history of the country the gate resolved', () => {
  const track = { branch: { id: 'b1' }, selectedTag: null } as unknown as Parameters<NonNullable<typeof activeResearchDeclaration.aux>['params']>[0];
  const params = activeResearchDeclaration.aux!.params;
  assert.deepEqual(params(track, {}, 'USA'), { branchId: 'b1', country: 'USA' });
  assert.equal(params(track, {}, null), null);
});

test('activeResearchProjection renders researching, idle, missing, stale caption, and daily nextChange', () => {
  const hour = 61320100;
  const history: ResearchData = {
    history: {
      coverage: [{ firstHour: 61320000, lastHour: 61320200 }],
      slots: [
        {
          index: 0,
          availability: [{ firstHour: 61320000, lastHour: 61320200 }],
          entries: [{
            token: 'basic_machine_tools',
            databaseIndex: 1,
            technologyHandle: 1,
            continuityStartHour: 61320000,
            level: 0,
            startedAfterHour: null,
            firstObservedHour: 61320000,
            lastObservedHour: 61320100,
            endedByHour: null,
            outcomeObservedHour: null,
            outcome: 'active',
          }],
          states: [],
          progress: [],
        },
        {
          index: 1,
          availability: [{ firstHour: 61320000, lastHour: 61320200 }],
          entries: [],
          states: [],
          progress: [],
        },
        {
          index: 2,
          availability: [{ firstHour: 61320000, lastHour: 61320200 }],
          entries: [],
          states: [],
          progress: [],
        },
      ],
    },
    definitions: [{ token: 'basic_machine_tools', databaseIndex: 1, name: 'Basic Machine Tools', description: null }],
    artwork: [],
    folders: [{ key: 'industry_folder', name: 'Industry' }],
    technologyFolders: [{ token: 'basic_machine_tools', databaseIndex: 1, folder: 'industry_folder' }],
    warnings: [],
  };
  const reply: ResearchSlotsReply = {
    tag: 'GER',
    readHour: hour,
    missing: null,
    warnings: [],
    slots: [
      {
        index: 0,
        state: {
          firstHour: hour - 48,
          lastHour: hour,
          token: 'basic_machine_tools',
          databaseIndex: 1,
          entryFirstObservedHour: 61320000,
          speedMultiplierRaw: '100000',
          bankedDaysRaw: '0',
          appliedDaysRaw: '0',
          aheadReductionYearsRaw: null,
          details: { remainingDays: 92, estimateRaw: '9200000', bonuses: [], modifiers: null },
        },
        progress: { firstHour: hour, lastHour: hour, entryFirstObservedHour: 61320000, progressRaw: '4500000', baseDaysRaw: '10000000' },
        definition: null,
      },
      {
        index: 1,
        state: {
          firstHour: hour,
          lastHour: hour,
          token: null,
          databaseIndex: null,
          entryFirstObservedHour: null,
          speedMultiplierRaw: '100000',
          bankedDaysRaw: '1400000',
          appliedDaysRaw: '0',
          aheadReductionYearsRaw: null,
          details: null,
        },
        progress: null,
        definition: null,
      },
    ],
    grants: { nextId: null, unavailable: null, grants: [] },
  };

  const art: ArtProvider = {
    resolve: handle => handle.kind === 'technology' && handle.token === 'basic_machine_tools'
      ? readyArt('asset://tech/basic_machine_tools.png')
      : missingArt('Art missing from the current installation'),
  };

  const view = createBoundedReadView({
    hour,
    panels: new Map([['main', reply]]),
    live: new Map([['main', makeLive({ [liveKey('0', 'progress')]: 0.68 })]]),
    art,
  });

  const model = activeResearchProjection.project(view, { history, countryName: 'Germany' });
  assert.equal(model.layout, 'rows');
  assert.equal(model.summaryText, null);
  assert.equal(model.sections?.length, 1);
  const sec = model.sections![0]!;
  assert.equal(sec.title, 'Researching');
  assert.equal(sec.count, '1 of 3 slots · 1 not recorded', 'an unread slot is named, not counted as idle');
  assert.ok(sec.rightCaption?.startsWith('As of '), 'shows single As of caption when reading >= 24h old');
  assert.equal(sec.rows?.length, 3);
  assert.equal(sec.rows![0]!.name, 'Basic Machine Tools');
  assert.equal(sec.rows![0]!.subtitle, 'Industry');
  assert.equal(sec.rows![0]!.figure, '68%');
  assert.equal(sec.rows![0]!.secondary, '90 days left');
  assert.equal(sec.rows![0]!.art?.result.status, 'ready');
  assert.equal(sec.rows![1]!.name, 'Idle');
  assert.equal(sec.rows![1]!.secondary, '14 days saved');
  assert.equal(sec.rows![2]!.name, 'Not recorded');
  assert.equal(sec.rows![2]!.figureMissing, true);
  assert.equal(activeResearchProjection.nextChange?.(view, { history, countryName: 'Germany' }), hour + 24);

  const unreadable = createBoundedReadView({
    hour,
    panels: new Map([['main', { ...reply, slots: [], missing: 'Research table block 100 unreadable: checksum' }]]),
  });
  const missingModel = activeResearchProjection.project(unreadable, { history, countryName: 'Germany' });
  assert.equal(missingModel.emptyKind, 'unavailable');
  assert.equal(missingModel.empty, 'Research table block 100 unreadable: checksum');
});

test('doctrineMasteryProjection builds threshold meters, missing mastery, empty track, and folder filter', () => {
  const hour = 61320100;
  const reply: DoctrineMasteryReply = {
    warnings: [],
    folders: [
      {
        ordinal: 0,
        id: 'land_doctrine',
        name: 'Land',
        icon: null,
        grand: { id: 'mobile_warfare', name: 'Mobile Warfare', icon: null },
        completed: [],
        tracks: [
          {
            ordinal: 0,
            name: 'Armour',
            sub: { id: 'armored_spearhead', name: 'Armored Spearhead', icon: null },
            unlocked: 2,
            rewardsUnknown: null,
            rewards: [
              { id: 'r1', name: 'Breakthrough', threshold: '5000000' },
              { id: 'r2', name: 'Blitz', threshold: '5000000' },
              { id: 'r3', name: 'Modern Blitzkrieg', threshold: '10000000' },
            ],
          },
          {
            ordinal: 1,
            name: 'Infantry',
            sub: null,
            unlocked: 0,
            rewardsUnknown: null,
            rewards: [],
          },
        ],
      },
      {
        ordinal: 1,
        id: 'mod_orbital_doctrine',
        name: 'Orbital Warfare',
        icon: null,
        grand: null,
        completed: [],
        tracks: [
          {
            ordinal: 0,
            name: 'Drop Pods',
            sub: { id: 'drop_assault', name: 'Drop Assault', icon: null },
            unlocked: 0,
            rewardsUnknown: null,
            rewards: [{ id: 'o1', name: 'Shock Drop', threshold: '10000000' }],
          },
        ],
      },
      {
        ordinal: 2,
        id: 'mod_cyber_doctrine',
        name: 'Cyber Warfare',
        icon: null,
        grand: null,
        completed: [],
        tracks: [],
      },
      {
        ordinal: 3,
        id: 'mod_missing_rewards',
        name: 'Unread Rewards',
        icon: null,
        grand: { id: 'mod_grand', name: 'Mod Grand', icon: null },
        completed: [],
        tracks: [{
          ordinal: 0,
          name: 'Mod Track',
          sub: { id: 'mod_sub', name: 'Mod Sub', icon: null },
          unlocked: 2,
          rewardsUnknown: "The subdoctrine's rewards are not in the content",
          rewards: [],
        }],
      },
    ],
  };

  const view = createBoundedReadView({
    hour,
    panels: new Map([['main', reply]]),
    live: new Map([['main', makeLive({
      [liveKey('0|0', 'mastery')]: 12400000n,
      [liveKey('0|0', 'effective')]: 150000n,
      [liveKey('0|0', 'daily')]: 100000n,
      [liveKey('0|0', 'bank')]: 0n,
      [liveKey('0|1', 'bank')]: 2000000n,
    }, {
      [liveKey('1|0', 'mastery')]: 'Doctrine mastery not recorded',
    })]]),
  });

  const model = doctrineMasteryProjection.project(view, { folders: [], countryName: 'Germany' });
  assert.equal(model.layout, 'columns');
  assert.equal(model.summaryText, '3 tracks active');
  assert.equal(model.sections?.length, 4);
  const unread = model.sections![3]!.rows![0]!;
  assert.equal(unread.subtitle, '2 steps unlocked', 'the recorded unlock count, never 2 of 0');
  assert.equal(unread.clickable, true);
  assert.equal(unread.tooltip?.items?.find(item => item.label === 'Steps in all')?.missingReason, "The subdoctrine's rewards are not in the content");
  assert.equal(model.sections![0]!.title, 'Land \u00b7 Mobile Warfare');
  assert.deepEqual(model.sections![2]!.rows!.map(row => [row.name, row.bar.kind]), [['No doctrine chosen', 'none']]);
  const armour = model.sections![0]!.rows![0]!;
  assert.equal(armour.name, 'Armored Spearhead');
  assert.equal(armour.subtitle, 'Step 3 of 3 \u00b7 Modern Blitzkrieg');
  assert.equal(armour.bar.kind, 'threshold');
  if (armour.bar.kind === 'threshold') {
    assert.equal(armour.bar.ticks.length, 3);
    assert.equal(armour.bar.ticks[0]!.reached, true);
    assert.equal(armour.bar.ticks[2]!.reached, false);
  }
  assert.equal(armour.figure, '124 / 200');
  assert.equal(armour.secondary, '+1.5 a day');
  assert.equal(armour.compactFigure, '2/3');

  const emptyTrack = model.sections![0]!.rows![1]!;
  assert.equal(emptyTrack.name, 'No subdoctrine chosen');
  assert.equal(emptyTrack.bar.kind, 'none');
  assert.equal(emptyTrack.secondary, '20 banked');

  const missingTrack = model.sections![1]!.rows![0]!;
  assert.equal(missingTrack.figureMissing, true);
  assert.equal(missingTrack.figureMissingLabel, 'Not recorded');

  const filtered = doctrineMasteryProjection.project(view, { folders: ['mod_orbital_doctrine'], countryName: 'Germany' });
  assert.equal(filtered.sections?.length, 1);
  assert.equal(filtered.sections![0]!.title, 'Orbital Warfare');
});

test('decryptionProjection renders Ciphers and Who reads whom views via readView.who and readView.art', () => {
  const hour = 61320100;
  const reply: DecryptionReply = {
    missing: null,
    attemptsUnknown: null,
    warnings: [],
    counts: { broken: 1, decrypting: 1, paused: 0, breakUnknown: 1 },
    pairs: [
      {
        owner: { id: 1, tag: 'ENG' },
        target: { id: 2, tag: 'GER' },
        thresholdRaw: '10000000',
        thresholdUnknown: null,
        bonusActive: 0,
        decrypting: 1,
        broken: true,
        status: 'broken',
        started: { hour: 61310000, upperBound: false },
        brokenAt: { hour: 61319000, upperBound: true },
        weekChangeTenths: 0,
        weekChangeUnknown: null,
      },
      {
        owner: { id: 2, tag: 'GER' },
        target: { id: 3, tag: 'SOV' },
        thresholdRaw: '10000000',
        thresholdUnknown: null,
        bonusActive: 0,
        decrypting: 1,
        broken: false,
        status: 'decrypting',
        started: { hour: 61318000, upperBound: false },
        brokenAt: null,
        weekChangeTenths: 24,
        weekChangeUnknown: null,
      },
      {
        owner: { id: 1, tag: 'ENG' },
        target: { id: 3, tag: 'SOV' },
        thresholdRaw: null,
        thresholdUnknown: 'Target strength not recorded',
        bonusActive: 0,
        decrypting: 1,
        broken: null,
        status: 'breakUnknown',
        started: null,
        brokenAt: null,
        weekChangeTenths: null,
        weekChangeUnknown: 'Progress is not known: Target strength not recorded',
      },
    ],
  };

  const identity = makeIdentity({
    ENG: { name: 'United Kingdom', colour: '#d64545', flag: 'ENG:democratic' },
    GER: { name: 'German Reich', colour: '#666b73', flag: 'GER:fascism' },
    SOV: { name: 'Soviet Union', colour: '#b83232', flag: 'SOV:communism' },
  });
  const art: ArtProvider = {
    resolve: handle => handle.kind === 'flag' && handle.choice === 'ENG:democratic'
      ? readyArt('asset://flags/ENG.png')
      : handle.kind === 'flag' && handle.choice === 'GER:fascism'
        ? PENDING_ART
        : missingArt('Flag missing'),
  };

  const view = createBoundedReadView({
    hour,
    panels: new Map([['main', reply]]),
    live: new Map([['main', makeLive({
      [liveKey('1:2', 'amount')]: 10000000,
      [liveKey('2:3', 'amount')]: 6200000,
      [liveKey('1:3', 'amount')]: undefined,
    })]]),
    identity,
    art,
  });

  const allFilter = { passing: null, unrecorded: [], bounds: [] };
  const ciphers = decryptionProjection.project(view, { filter: allFilter, filterAll: true, side: 'decryptor' });
  assert.equal(ciphers.layout, 'ciphers');
  assert.equal(ciphers.summaryText, '1 broken \u00b7 1 decrypting \u00b7 1 not known', 'a break not known is counted apart, not as decrypting or broken');
  assert.deepEqual(ciphers.sections?.map(section => [section.title, section.count]), [['Decrypting', '1'], ['Not known if broken', '1'], ['Cipher broken', '1']]);
  assert.equal(ciphers.sections![0]!.rows![0]!.figure, '62%');
  assert.equal(ciphers.sections![1]!.rows![0]!.figureMissing, true);
  const weekItem = (section: number) => ciphers.sections![section]!.rows![0]!.tooltip?.items?.find(item => item.label === 'Last 7 days');
  assert.equal(weekItem(0)?.value, '+2.4 points');
  assert.equal(weekItem(1)?.missingReason, 'Progress is not known: Target strength not recorded');
  assert.equal(ciphers.sections![2]!.chipLines?.length, 1);
  assert.equal(ciphers.sections![2]!.chipLines![0]!.ownerName, 'United Kingdom');
  assert.equal(ciphers.sections![2]!.chipLines![0]!.chips[0]!.targetName, 'German Reich');
  assert.ok(!ciphers.sections![2]!.chipLines![0]!.chips[0]!.dateText?.includes(':'), 'broken date omits hour');
  const chip = ciphers.sections![2]!.chipLines![0]!.chips[0]!;
  assert.ok(chip.dateText?.startsWith('By '), 'a break first witnessed after a gap keeps its upper bound on the chip');
  assert.match(chip.spoken, / By /);
  assert.match(chip.audit, /\|broken\|By /);

  const partialReply: DecryptionReply = { ...reply, attemptsUnknown: 'Attempt starts and breaks are not known: history unreadable' };
  const partialView = createBoundedReadView({ hour, panels: new Map([['main', partialReply]]), live: new Map([['main', makeLive({ [liveKey('2:3', 'amount')]: 6200000 })]]), identity, art });
  const onlyGerman = { passing: { GER: [[0, hour + 1000]] as [number, number][] }, unrecorded: [], bounds: [] };
  const filtered = decryptionProjection.project(partialView, { filter: onlyGerman, filterAll: false, side: 'decryptor' });
  assert.equal(filtered.summaryText, '0 broken \u00b7 1 decrypting \u00b7 Partial', 'a filtered count over a partially known population is marked, never exact');

  const matrix = whoReadsWhomProjection.project(view, { filter: allFilter, filterAll: true, side: 'decryptor' });
  assert.equal(matrix.corner, 'Reads');
  assert.equal(matrix.summaryText, '1 broken \u00b7 1 decrypting \u00b7 1 not known');
  assert.deepEqual(matrix.rows.map(r => [r.label, r.trailing]), [
    ['United Kingdom', 'Broken 1 \u00b7 Decrypting 0 \u00b7 Not known 1'],
    ['German Reich', 'Broken 0 · Decrypting 1'],
  ]);
  assert.deepEqual(matrix.columns.map(c => c.label), ['German Reich', 'Soviet Union']);
  const engGer = matrixCellAt(matrix, 0, 0);
  assert.equal(engGer.kind === 'value' && engGer.mark, 'square');
  assert.ok(engGer.kind === 'value' && engGer.audit?.includes('|broken|By '), 'the matrix audit keeps the upper bound');
  assert.equal(matrixCellAt(matrix, 0, 1).kind, 'missing');
  assert.equal(matrixCellAt(matrix, 1, 0).kind, 'diagonal');
  const gerSov = matrixCellAt(matrix, 1, 1);
  assert.ok(gerSov.kind === 'value' && gerSov.mark === 'ring' && gerSov.fraction === 0.62);

  const ctx = makeFakeCtx();
  const layout = layoutMeterModel(ctx, ciphers, 832, THEME);
  assert.ok(layout.hits.length >= 3);
  const firstHit = layout.hits[0]!;
  assert.equal(hitMeterModel(layout, firstHit.x + 2, firstHit.y + 2, 0, 0), `row:${firstHit.key}`);
  assert.ok(findMeterTooltip(ciphers, firstHit.key) !== null);
  assert.equal(deriveMeterNav(ciphers).length, 3);
  const measured = deriveMeterNav(ciphers, layout.itemBands);
  assert.deepEqual(measured.map(entry => [entry[6], entry[7]]), measured.map(entry => [...layout.itemBands.get(entry[0])!]), 'navigation scrolls to each item’s measured band');
  assert.equal(measured[0]![7], 23, 'a cipher row is 23 px, not an assumed 32');
  assert.equal(deriveMeterMirror(ciphers).total, 3);
  const audit = deriveMeterAudit(ciphers);
  assert.equal(audit.length, 7, 'the summary, each section header and each item');
  assert.equal(audit.find(row => row.key === 'section:decrypting')?.value, 'Decrypting|1|');
});

test('autonomyLevelsProjection keeps the product order, lists an unread subject with its reason, and filters by overlord or subject', () => {
  const hour = 61320100;
  const subject = (overlord: string, tag: string, id: number, level: string | null, progress: number | null, reason: string | null = null) => ({
    overlordId: 1, overlord, subjectId: id, subject: tag, level,
    levelBelow: level === 'dominion' ? 'puppet' : null, levelAbove: level === 'dominion' ? 'free' : null, levelsUnknown: level === null ? 'The subject\'s level is not recorded at this hour' : null,
    progressMilli: progress, progressUnknown: reason,
  });
  const reply: AutonomyLevelsReply = {
    levels: [{ token: 'puppet', name: 'Puppet', ordered: true }, { token: 'dominion', name: 'Dominion', ordered: true }, { token: 'free', name: 'Free', ordered: true }],
    subjects: [
      subject('ENG', 'RAJ', 10, 'dominion', 97_000),
      subject('ENG', 'CAN', 11, 'dominion', 50_000),
      subject('GER', 'SLO', 12, null, null, 'The subject\'s autonomy is not recorded at this hour'),
    ],
    missing: null,
    warnings: [],
  };
  const view = createBoundedReadView({ hour, panels: new Map([['main', reply]]), identity: makeIdentity({ RAJ: { name: 'British Raj' }, CAN: { name: 'Dominion of Canada' }, SLO: { name: 'Slovakia' } }) });
  const all = { passing: null, unrecorded: [], bounds: [] };
  const model = autonomyLevelsProjection.project(view, { filter: all, filterAll: true, bySubject: false, surface: '#111111' });
  const rows = model.sections![0]!.rows!;
  assert.deepEqual(rows.map(row => row.name), ['British Raj', 'Dominion of Canada', 'Slovakia'], 'the product order, never re-sorted');
  assert.equal(model.summaryText, '3 subjects');
  assert.equal(rows[0]!.figure, '97%');
  assert.equal(rows[0]!.subtitle, 'Dominion');
  assert.ok(rows[0]!.bar.kind === 'threshold' && rows[0]!.bar.ends === true && rows[0]!.bar.fraction === 0.97);
  assert.equal(rows[0]!.tooltip?.items?.find(item => item.label === 'Level below')?.value, 'Puppet');
  assert.equal(rows[2]!.figureMissing, true, 'an unread subject is listed and marked, not drawn as zero');
  assert.ok(rows[2]!.bar.kind === 'threshold' && rows[2]!.bar.fraction === null);
  assert.equal(rows[2]!.tooltip?.items?.find(item => item.label === 'Within level')?.missingReason, "The subject's autonomy is not recorded at this hour");
  const onlyGerman = { passing: { GER: [[0, hour + 10]] as [number, number][] }, unrecorded: [], bounds: [] };
  assert.deepEqual(autonomyLevelsProjection.project(view, { filter: onlyGerman, filterAll: false, bySubject: false, surface: '#111111' }).sections![0]!.rows!.map(row => row.name), ['Slovakia']);
  const onlyCanada = { passing: { CAN: [[0, hour + 10]] as [number, number][] }, unrecorded: [], bounds: [] };
  assert.deepEqual(autonomyLevelsProjection.project(view, { filter: onlyCanada, filterAll: false, bySubject: true, surface: '#111111' }).sections![0]!.rows!.map(row => row.name), ['Dominion of Canada']);
});

test('the meter binding names its view, so a second meters view of a kind finds its projection', () => {
  const ctx = { kind: 'autonomy', view: 'meters', options: {}, sources: {}, viewport: { width: 400, height: 200, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null },
    theme: THEME, identitySnapshot: null, selectedKey: null, focusedKey: null, audit: false, auditId: undefined, pauseFlush: 0, reducedMotion: false };
  assert.equal(meterHostAdapter.buildBinding(ctx as never)?.view, 'meters');
});

test('bounded readView throws FutureDataError on future who() lookups', () => {
  const view = createBoundedReadView({ hour: 100 });
  assert.throws(() => view.who('country', 'GER', 101), FutureDataError);
});
