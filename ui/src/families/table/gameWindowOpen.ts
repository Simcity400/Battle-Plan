import type { DrillAction } from '../../charts/drill/drillState.ts';
import type { GameWindowSelection } from '../../dashboard/gameWindows.ts';
import type { OpenedRow } from './rowSelection.ts';

export function openGameWindow<S extends GameWindowSelection>(
  scope: string,
  key: string,
  selection: S,
  breakdown: string,
): { readonly opened: OpenedRow<S>; readonly drill: DrillAction } {
  return {
    opened: { scope, key, selection },
    drill: { type: 'open', breakdown, hour: selection.hour, follow: false },
  };
}

export function gameWindowSelectionAt<S extends GameWindowSelection>(opened: OpenedRow<S>, hour: number): S {
  return opened.selection.hour === hour ? opened.selection : { ...opened.selection, hour };
}
