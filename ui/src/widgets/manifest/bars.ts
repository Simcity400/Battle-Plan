import type { OptionField } from '../../dashboard/options.ts';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { familyBarsLaneView } from '../../families/bars/laneView.ts';
import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry } from '../../families/manifest.ts';
import { OPTIONS as AIR_FORCE_OPTIONS } from '../air-force-size/definition.ts';
import { OPTIONS as ARMY_SIZE_OPTIONS } from '../army-size/definition.ts';
import { OPTIONS as EQUIPMENT_PRODUCED_OPTIONS } from '../equipment-produced/definition.ts';
import { OPTIONS as FACTION_STRENGTH_OPTIONS } from '../faction-strength/definition.ts';
import { OPTIONS as FACTORIES_OPTIONS } from '../factories/definition.ts';
import { OPTIONS as GAME_SCORE_OPTIONS } from '../game-score/definition.ts';
import { OPTIONS as NAVAL_LOSSES_OPTIONS } from '../naval-losses/definition.ts';
import { OPTIONS as NAVY_SIZE_OPTIONS } from '../navy-size/definition.ts';
import { OPTIONS as NUCLEAR_ARSENAL_OPTIONS } from '../nuclear-arsenal/definition.ts';
import { OPTIONS as OCCUPATION_TREND_OPTIONS } from '../occupation-trend/definition.ts';
import { OPTIONS as TERRITORY_OPTIONS } from '../territory/definition.ts';
import { OPTIONS as WORLD_TENSION_OPTIONS } from '../world-tension/definition.ts';

function raceScopedOptions(fields: readonly OptionField[], keys: ReadonlySet<string>): readonly OptionField[] {
  return fields
    .filter(field => keys.has(field.key))
    .map(field => (field.key === 'view' || field.key === 'split' ? { ...field, visibleWhen: undefined } : field));
}

const AIR_FORCE_RACE_OPTION_KEYS = new Set(['view', 'measure', 'ownership', 'split']);
const AIR_FORCE_RACE_OPTIONS = raceScopedOptions(AIR_FORCE_OPTIONS, AIR_FORCE_RACE_OPTION_KEYS);
const AIR_FORCE_MIX_OPTIONS = raceScopedOptions(AIR_FORCE_OPTIONS, new Set(['view', 'measure', 'ownership']));
const ARMY_SIZE_RACE_OPTIONS = raceScopedOptions(ARMY_SIZE_OPTIONS, new Set(['view', 'measure', 'countFor', 'byType']))
  .map((field): OptionField => (field.key === 'byType' ? { ...field, visibleWhen: options => options.measure === 'battalions' } : field));
const ARMY_SIZE_MIX_OPTIONS = raceScopedOptions(ARMY_SIZE_OPTIONS, new Set(['view', 'countFor']));
const EQUIPMENT_PRODUCED_RACE_OPTIONS = raceScopedOptions(EQUIPMENT_PRODUCED_OPTIONS, new Set(['view', 'count', 'measure', 'included', 'refits']));
const FACTION_STRENGTH_RACE_OPTIONS = raceScopedOptions(FACTION_STRENGTH_OPTIONS, new Set(['view', 'measure', 'count', 'territoryMeasure', 'rule']));
const FACTORIES_MIX_OPTIONS = raceScopedOptions(FACTORIES_OPTIONS, new Set(['view', 'count', 'types']));
const GAME_SCORE_RACE_OPTION_KEYS = new Set(['view', 'topMeasure', 'split']);
const GAME_SCORE_RACE_OPTIONS = GAME_SCORE_OPTIONS.filter(field => GAME_SCORE_RACE_OPTION_KEYS.has(field.key));
const NAVAL_LOSSES_RACE_OPTION_KEYS = new Set(['view', 'kind', 'attribution', 'period', 'split']);
const NAVAL_LOSSES_RACE_OPTIONS = raceScopedOptions(NAVAL_LOSSES_OPTIONS, NAVAL_LOSSES_RACE_OPTION_KEYS)
  .map((field): OptionField => (field.key === 'split' ? { ...field, visibleWhen: options => options.kind === 'both' } : field));
const NAVAL_LOSSES_MIX_OPTIONS = raceScopedOptions(NAVAL_LOSSES_OPTIONS, new Set(['view', 'kind', 'attribution', 'period']));
const NAVY_SIZE_RACE_OPTION_KEYS = new Set(['view', 'measure', 'ownership', 'grouping', 'split']);
const NAVY_SIZE_RACE_OPTIONS = raceScopedOptions(NAVY_SIZE_OPTIONS, NAVY_SIZE_RACE_OPTION_KEYS);
const NAVY_SIZE_MIX_OPTIONS = raceScopedOptions(NAVY_SIZE_OPTIONS, new Set(['view', 'measure', 'ownership', 'grouping']));
const NUCLEAR_ARSENAL_RACE_OPTION_KEYS = new Set(['view', 'split']);
const NUCLEAR_ARSENAL_RACE_OPTIONS = raceScopedOptions(NUCLEAR_ARSENAL_OPTIONS, NUCLEAR_ARSENAL_RACE_OPTION_KEYS);
const NUCLEAR_ARSENAL_MIX_OPTIONS = raceScopedOptions(NUCLEAR_ARSENAL_OPTIONS, new Set(['view']));
const OCCUPATION_TREND_RACE_OPTION_KEYS = new Set(['view', 'measure', 'rankingMeasure', 'digits']);
const OCCUPATION_TREND_RACE_OPTIONS = OCCUPATION_TREND_OPTIONS.filter(field => OCCUPATION_TREND_RACE_OPTION_KEYS.has(field.key));
const RAID_TALLY_RACE_OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'race',
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines' }] },
  { key: 'metric', section: 'options', label: 'Count', kind: 'choice', default: 'both',
    choices: [{ value: 'both', label: 'Both' }, { value: 'launched', label: 'Launched' }, { value: 'succeeded', label: 'Succeeded' }] },
];
const TERRITORY_RACE_OPTIONS = raceScopedOptions(TERRITORY_OPTIONS, new Set(['view', 'measure', 'rule']));
const WORLD_TENSION_RACE_OPTIONS = WORLD_TENSION_OPTIONS.filter(field => field.key === 'view');

export const BARS_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'war-contribution',
    view: 'measure',
    label: 'By measure',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../war-contribution/measureProjection.ts').then(m => ({
      family: 'bars', kind: 'war-contribution', view: 'measure',
      projection: m.warContributionMeasureProjection, declaration: m.warContributionMeasureDeclaration,
    })),
  },
  {
    kind: 'air-force-size',
    view: 'mix',
    label: 'Mix',
    family: 'bars',
    viewOption: 'view',
    options: AIR_FORCE_MIX_OPTIONS,
    loadSpec: () => import('../air-force-size/airForceSizeMixProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-force-size', view: 'mix',
      projection: mod.airForceSizeMixProjection, declaration: mod.airForceSizeMixDeclaration,
    })),
  },
  {
    kind: 'air-force-size',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: AIR_FORCE_RACE_OPTIONS,
    loadSpec: () => import('../air-force-size/airForceSizeRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-force-size', view: 'race',
      projection: mod.airForceSizeRaceProjection, declaration: mod.airForceSizeRaceDeclaration,
    })),
  },
  {
    kind: 'air-kills-losses',
    view: 'butterfly',
    label: 'Butterfly',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../air-kills-losses/airKillsLossesProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-kills-losses', view: 'butterfly',
      projection: mod.airKillsLossesProjection, declaration: mod.airKillsLossesDeclaration,
    })),
  },
  {
    kind: 'air-kills-losses',
    view: 'breakdown',
    label: 'Loss causes',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../air-kills-losses/airKillsLossesProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-kills-losses', view: 'breakdown',
      projection: mod.airLossCausesProjection, declaration: mod.airLossCausesDeclaration,
    })),
  },
  {
    kind: 'air-kills-losses',
    view: 'paired',
    label: 'Paired bars',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../air-kills-losses/airKillsLossesProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-kills-losses', view: 'paired',
      projection: mod.airKillsLossesPairedProjection, declaration: mod.airKillsLossesPairedDeclaration,
    })),
  },
  {
    kind: 'air-superiority-region',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../air-superiority-region/airSuperiorityProjection.ts').then(mod => ({
      family: 'bars', kind: 'air-superiority-region', view: DEFAULT_VIEW,
      projection: mod.airSuperiorityRegionProjection, declaration: mod.airSuperiorityRegionDeclaration,
    })),
  },
  {
    kind: 'army-size',
    view: 'mix',
    label: 'Battalion mix',
    family: 'bars',
    viewOption: 'view',
    options: ARMY_SIZE_MIX_OPTIONS,
    loadSpec: () => import('../army-size/armySizeMixProjection.ts').then(mod => ({
      family: 'bars', kind: 'army-size', view: 'mix',
      projection: mod.armySizeMixProjection, declaration: mod.armySizeMixDeclaration,
    })),
  },
  {
    kind: 'army-size',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: ARMY_SIZE_RACE_OPTIONS,
    loadSpec: () => import('../army-size/armySizeRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'army-size', view: 'race',
      projection: mod.armySizeRaceProjection, declaration: mod.armySizeRaceDeclaration,
    })),
  },
  {
    kind: 'bombing-damage',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../bombing-damage/bombingDamageProjection.ts').then(mod => ({
      family: 'bars', kind: 'bombing-damage', view: DEFAULT_VIEW,
      projection: mod.bombingDamageProjection, declaration: mod.bombingDamageDeclaration,
    })),
  },
  {
    kind: 'casualties',
    view: 'butterfly',
    label: 'Butterfly',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../casualties/casualtiesProjection.ts').then(mod => ({
      family: 'bars', kind: 'casualties', view: 'butterfly',
      projection: mod.casualtiesButterflyProjection, declaration: mod.casualtiesButterflyDeclaration,
    })),
  },
  {
    kind: 'casualties',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../casualties/casualtiesProjection.ts').then(mod => ({
      family: 'bars', kind: 'casualties', view: 'race',
      projection: mod.casualtiesRaceProjection, declaration: mod.casualtiesRaceDeclaration,
    })),
  },
  {
    kind: 'casualties-between',
    view: 'opponents',
    label: 'By opponent',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../casualties-between/casualtiesBetweenProjection.ts').then(mod => ({
      family: 'bars', kind: 'casualties-between', view: 'opponents',
      projection: mod.casualtyOpponentsProjection, declaration: mod.casualtyOpponentsDeclaration,
    })),
  },
  {
    kind: 'equipment-produced',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: EQUIPMENT_PRODUCED_RACE_OPTIONS,
    loadSpec: () => import('../equipment-produced/equipmentProducedRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'equipment-produced', view: 'race',
      projection: mod.equipmentProducedRaceProjection, declaration: mod.equipmentProducedRaceDeclaration,
    })),
  },
  {
    kind: 'faction-strength',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: FACTION_STRENGTH_RACE_OPTIONS,
    loadSpec: () => import('../faction-strength/factionStrengthRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'faction-strength', view: 'race',
      projection: mod.factionStrengthRaceProjection, declaration: mod.factionStrengthRaceDeclaration,
    })),
  },
  {
    kind: 'factories',
    view: 'mix',
    label: 'Mix',
    family: 'bars',
    viewOption: 'view',
    options: FACTORIES_MIX_OPTIONS,
    loadSpec: () => import('../factories/factoriesMixProjection.ts').then(mod => ({
      family: 'bars', kind: 'factories', view: 'mix',
      projection: mod.factoriesMixProjection, declaration: mod.factoriesMixDeclaration,
    })),
  },
  {
    kind: 'factories',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../factories/factoriesProjection.ts').then(mod => ({
      family: 'bars', kind: 'factories', view: 'race',
      projection: mod.factoriesProjection, declaration: mod.factoriesDeclaration,
    })),
  },
  {
    kind: 'game-score',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: GAME_SCORE_RACE_OPTIONS,
    loadSpec: () => import('../game-score/gameScoreRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'game-score', view: 'race',
      projection: mod.gameScoreRaceProjection, declaration: mod.gameScoreRaceDeclaration,
    })),
  },
  {
    kind: 'lend-lease-balance',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../lend-lease-balance/lendLeaseBalanceProjection.ts').then(mod => ({
      family: 'bars', kind: 'lend-lease-balance', view: DEFAULT_VIEW,
      projection: mod.lendLeaseBalanceProjection, declaration: mod.lendLeaseBalanceDeclaration,
    })),
  },
  {
    kind: 'minefields',
    view: DEFAULT_VIEW,
    family: 'bars',
    loadSpec: () => import('../minefields/minefieldsProjection.ts').then(mod => ({
      family: 'bars', kind: 'minefields', view: DEFAULT_VIEW,
      projection: mod.minefieldsProjection, declaration: mod.minefieldsDeclaration,
    })),
  },
  {
    kind: 'naval-losses',
    view: 'mix',
    label: 'Mix',
    family: 'bars',
    viewOption: 'view',
    options: NAVAL_LOSSES_MIX_OPTIONS,
    loadSpec: () => import('../naval-losses/navalLossesMixProjection.ts').then(mod => ({
      family: 'bars', kind: 'naval-losses', view: 'mix',
      projection: mod.navalLossesMixProjection, declaration: mod.navalLossesMixDeclaration,
    })),
  },
  {
    kind: 'naval-losses',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: NAVAL_LOSSES_RACE_OPTIONS,
    loadSpec: () => import('../naval-losses/navalLossesRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'naval-losses', view: 'race',
      projection: mod.navalLossesRaceProjection, declaration: mod.navalLossesRaceDeclaration,
    })),
  },
  {
    kind: 'naval-supremacy',
    view: 'shares',
    label: 'Shares',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../naval-supremacy/navalSupremacyProjection.ts').then(m => ({
      family: 'bars', kind: 'naval-supremacy', view: 'shares',
      projection: m.supremacySharesProjection, declaration: m.supremacySharesDeclaration,
    })),
  },
  {
    kind: 'navy-size',
    view: 'mix',
    label: 'Mix',
    family: 'bars',
    viewOption: 'view',
    options: NAVY_SIZE_MIX_OPTIONS,
    loadSpec: () => import('../navy-size/navySizeMixProjection.ts').then(mod => ({
      family: 'bars', kind: 'navy-size', view: 'mix',
      projection: mod.navySizeMixProjection, declaration: mod.navySizeMixDeclaration,
    })),
  },
  {
    kind: 'navy-size',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: NAVY_SIZE_RACE_OPTIONS,
    loadSpec: () => import('../navy-size/navySizeRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'navy-size', view: 'race',
      projection: mod.navySizeRaceProjection, declaration: mod.navySizeRaceDeclaration,
    })),
  },
  {
    kind: 'nuclear-arsenal',
    view: 'mix',
    label: 'Mix',
    family: 'bars',
    viewOption: 'view',
    options: NUCLEAR_ARSENAL_MIX_OPTIONS,
    loadSpec: () => import('../nuclear-arsenal/nuclearArsenalMixProjection.ts').then(mod => ({
      family: 'bars', kind: 'nuclear-arsenal', view: 'mix',
      projection: mod.nuclearArsenalMixProjection, declaration: mod.nuclearArsenalMixDeclaration,
    })),
  },
  {
    kind: 'nuclear-arsenal',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: NUCLEAR_ARSENAL_RACE_OPTIONS,
    loadSpec: () => import('../nuclear-arsenal/nuclearArsenalRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'nuclear-arsenal', view: 'race',
      projection: mod.nuclearArsenalRaceProjection, declaration: mod.nuclearArsenalRaceDeclaration,
    })),
  },
  {
    kind: 'occupation-reserve-manpower',
    view: 'main',
    label: 'Holders',
    family: 'bars',
    viewOption: 'show',
    loadSpec: () => import('../occupation-reserve-manpower/occupationReserveRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'occupation-reserve-manpower', view: 'main',
      projection: mod.occupationReserveRaceProjection, declaration: mod.occupationReserveRaceDeclaration,
    })),
  },
  {
    kind: 'occupation-trend',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: OCCUPATION_TREND_RACE_OPTIONS,
    loadSpec: () => import('../occupation-trend/occupationTrendRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'occupation-trend', view: 'race',
      projection: mod.occupationTrendRaceProjection, declaration: mod.occupationTrendRaceDeclaration,
    })),
  },
  {
    kind: 'raid-tally',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: RAID_TALLY_RACE_OPTIONS,
    loadSpec: () => import('../raid-tally/raidTallyRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'raid-tally', view: 'race',
      projection: mod.raidTallyRaceProjection, declaration: mod.raidTallyRaceDeclaration,
    })),
  },
  {
    kind: 'territory',
    view: 'race',
    label: 'Race',
    family: 'bars',
    viewOption: 'view',
    options: TERRITORY_RACE_OPTIONS,
    loadSpec: () => import('../territory/territoryRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'territory', view: 'race',
      projection: mod.territoryRaceProjection, declaration: mod.territoryRaceDeclaration,
    })),
  },
  {
    kind: 'who-spies-on-whom',
    view: 'pairs',
    label: 'Top pairs',
    family: 'bars',
    viewOption: 'view',
    loadSpec: () => import('../who-spies-on-whom/whoSpiesProjection.ts').then(mod => ({
      family: 'bars', kind: 'who-spies-on-whom', view: 'pairs',
      projection: mod.operativePairsProjection, declaration: mod.operativePairsDeclaration,
    })),
  },
  {
    kind: 'world-tension',
    view: 'race',
    label: 'Contributions race',
    family: 'bars',
    viewOption: 'view',
    options: WORLD_TENSION_RACE_OPTIONS,
    loadSpec: () => import('../world-tension/worldTensionRaceProjection.ts').then(mod => ({
      family: 'bars', kind: 'world-tension', view: 'race',
      projection: mod.worldTensionRaceProjection, declaration: mod.worldTensionRaceDeclaration,
    })),
  },
];

export const barsFamilyManifest: FamilyManifestModule = {
  family: 'bars',
  laneView: familyBarsLaneView,
  views: BARS_VIEWS,
  hostAdapter: barsHostAdapter,
};
