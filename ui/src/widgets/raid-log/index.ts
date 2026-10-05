import type { WidgetRenderer } from '../../dashboard/definitions';
import { setToken } from '../../dashboard/options';
import { raidTypeChoices } from '../../dashboard/raids';
import { useRaids } from '../../dashboard/useRaids';
import RaidLogWidget from './RaidLogWidget';

export default { render: RaidLogWidget, useOptionChoices: (widget, runtime) => {
  const { data } = useRaids(widget, runtime, true, { view: 'catalogue', tags: [], top: null });
  return { types: raidTypeChoices(data?.types ?? []).map(type => ({ value: setToken(type.token), label: type.label })) };
} } satisfies WidgetRenderer;
