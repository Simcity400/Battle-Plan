import { useCallback, useId, useMemo } from 'react';
import { fitTooltip, tooltipText } from '../charts/tooltip';
import { useViewportHeight } from '../charts/useChartTokens';
import { useRowTooltips } from '../charts/useRowTooltips';
import type { CommandHeldAt } from './politicsSpending';
import { commandHeldTooltip, spendText } from './politicsSpending';

/** The current allocation figure is separate from accumulated spending. The
 * shared tooltip fits the holder list and keeps its full spoken description. */
export default function PoliticsCommandHeld({ held, active }: { held: CommandHeldAt; active: boolean }) {
  const owner = useId();
  const height = useViewportHeight();
  const full = useMemo(() => commandHeldTooltip(held), [held]);
  const content = useCallback(() => fitTooltip([full], height, { keep: 1 }), [full, height]);
  const onMenu = useCallback(() => {}, []);
  const tip = useRowTooltips({ owner, active, content, revision: String(held.hour), onMenu });
  return <span className="politics-command-held" {...tip.rowProps('held')}>
    CP held <b>{spendText(held.total)}</b>
    <span className="sr-only" id={tip.descriptionId('held')}>{tooltipText(full)}</span>
  </span>;
}
