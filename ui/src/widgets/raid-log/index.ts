import type { WidgetRenderer } from '../../dashboard/definitions';
import { setToken } from '../../dashboard/options';
import { raidTypeChoices, useRaidLog } from '../../dashboard/raids';
import RaidLogWidget from './RaidLogWidget';

export default { render: RaidLogWidget, useOptionChoices: (widget, runtime) => {
  const { data } = useRaidLog(widget, runtime, true, true);
  return { types: raidTypeChoices(data?.types ?? []).map(type => ({ value: setToken(type.token), label: type.label })) };
} } satisfies WidgetRenderer;
