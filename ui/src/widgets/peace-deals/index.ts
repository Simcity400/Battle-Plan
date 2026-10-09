import { useMemo } from 'react';
import type { WidgetRenderer } from '../../dashboard/definitions';
import { usePeaceDeals } from '../../dashboard/usePeaceDeals';
import PeaceDealsWidget from './PeaceDealsWidget';
import { peaceActionChoices, preparePeaceLog } from './peaceData';

export default { render: PeaceDealsWidget, useOptionChoices: (widget, runtime) => {
  const { data } = usePeaceDeals(widget, runtime, true);
  return useMemo(() => ({ actions: peaceActionChoices(data ? preparePeaceLog(data) : null) }), [data]);
} } satisfies WidgetRenderer;
