import { formatDateTime } from '../../dashboard/calendar.ts';
import type { CountryPartsData } from '../../charts/race/countryPartsDrillData';
import { countRuns, factoriesAt, factoryReadAt, factorySegmentAt, factoryTooltips, groupName, mixBands } from './factoriesData.ts';
import type { CountMode, FactoryModel } from './factoriesData';
import type { CountryFactoriesReply, FactoryGroupKey } from '../../generated/dashboard';
import { shownRuns } from '../../charts/race/lines.ts';

/** Delayed picks follow numeric operands, independent of arriving artwork/names. */
export const factoryDrillNumbers = (reply: CountryFactoriesReply): CountryFactoriesReply => ({ ...reply, warnings: [],
  groups: reply.groups.map(group => ({ key: group.key, flag: group.flag,
    buildings: group.buildings.map(building => ({ token: building.token, name: null })) })) });

/** The recorded groups use the same construction/top-bar formulas as
 * factoriesData; no finer building split is recorded. */
export function factoryDrillData(model: FactoryModel, groups: readonly FactoryGroupKey[], mode: CountMode): CountryPartsData {
  const parts = model.reply.groups.filter(group => groups.includes(group.key)).map(group => ({ id: group.key, label: groupName(group) }));
  return {
    histories: [...model.series].map(([tag, series]) => ({ tag, identities: series.identities })),
    warnings: model.reply.warnings, parts: () => parts,
    at: (tag, hour) => {
      const at = factoriesAt(model, tag, hour, groups, mode);
      return at ? { total: at.total, values: at.groups.map(group => group.count) } : null;
    },
    bands: (tag, range) => [...mixBands(model, tag, groups, mode, null, range),
      shownRuns(countRuns(model, tag, groups, mode).map(run => ({ ...run, value: 0 })), null, range)],
    segmentAt: (tag, hour) => factorySegmentAt(model, tag, hour),
    readHour: (tag, hour) => factoriesAt(model, tag, hour, groups, mode) ? factoryReadAt(model, tag, hour) : null,
    keyHours: [...new Set(model.reply.series.flatMap(series => (series.reads ?? []).map(read => read.firstHour)))].sort((a, b) => a - b),
    tooltip: (tag, hour, name, colours) => {
      const at = factoriesAt(model, tag, hour, groups, mode);
      return at ? [...factoryTooltips(model, name, at, formatDateTime(hour), mode, colours)] : null;
    },
  };
}
