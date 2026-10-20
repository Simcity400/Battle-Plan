// The game-window renderer's windows, registered with the widgets' seam
// (dashboard/gameWindows.ts): a widget names a window by its semantic selection,
// and the renderer draws it from the recording at that hour.
import { Suspense, lazy } from 'react';
import { registerGameWindow } from '../dashboard/gameWindows';
import type { GameWindowProps, MioPolicySelection } from '../dashboard/gameWindows';
import WidgetState from './WidgetState';
import type { GameWindowProps as RendererProps } from './GameWindow';

// The renderer loads only once a window is first opened.
const GameWindow = lazy(() => import('./GameWindow'));

/** The renderer's view of a policy chooser selection: the organisation's policy
 * screen at the selection's hour. */
export function mioPolicyWindow({ selection, active, client, onDiagnostics }: GameWindowProps<MioPolicySelection>): RendererProps {
  return { client, source: { sourceId: selection.sourceId, revision: selection.revision }, branchId: selection.branchId,
    hour: selection.hour, target: { kind: 'mioPolicies', organisation: selection.organisation }, active, onDiagnostics };
}

/** An industrial organisation's policy chooser, as the game draws it. */
export function MioPolicyChooser(props: GameWindowProps<MioPolicySelection>) {
  return <Suspense fallback={<div className="game-window"><WidgetState kind="loading" label="Loading window…" /></div>}>
    <GameWindow {...mioPolicyWindow(props)} />
  </Suspense>;
}

registerGameWindow<MioPolicySelection>('mioPolicyChooser', MioPolicyChooser);
