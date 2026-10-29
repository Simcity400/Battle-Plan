// The game-window renderer's windows, registered with the widgets' seam
// (dashboard/gameWindows.ts): a widget names a window by its semantic selection,
// and the renderer draws it from the recording at that hour.
import { registerGameWindow } from '../dashboard/gameWindows';
import type { GameWindowProps, MioPolicySelection } from '../dashboard/gameWindows';
import { registerDesignerSnapshot } from '../widgets/division-template/designerSnapshot';
import type { DesignerRequest } from '../widgets/division-template/designerSnapshot';
import WidgetState from './WidgetState';
import { loadable } from './loadable';
import type { GameWindowProps as RendererProps } from './GameWindow';

// The renderer loads only once a window is first opened.
const GameWindow = loadable(() => import('./GameWindow').then(module => module.default),
  () => <div className="game-window"><WidgetState kind="loading" label="Loading window…" /></div>);

/** The renderer's view of a policy chooser selection: the organisation's policy
 * screen at the selection's hour. */
export function mioPolicyWindow({ selection, active, client, onDiagnostics }: GameWindowProps<MioPolicySelection>): RendererProps {
  return { client, source: { sourceId: selection.sourceId, revision: selection.revision }, branchId: selection.branchId,
    hour: selection.hour, target: { kind: 'mioPolicies', organisation: selection.organisation }, active, onDiagnostics };
}

/** An industrial organisation's policy chooser, as the game draws it. */
export function MioPolicyChooser(props: GameWindowProps<MioPolicySelection>) {
  return <GameWindow {...mioPolicyWindow(props)} />;
}

registerGameWindow<MioPolicySelection>('mioPolicyChooser', MioPolicyChooser);

/** The renderer's view of a Division template row: the template's division designer at the
 * row's hour. The window reads the template's owner and kind from the recording itself. */
export function designerWindow({ client, source, branchId, hour, template, onDiagnostics }: DesignerRequest, active: boolean): RendererProps {
  return { client, source: { sourceId: source.id, revision: source.revision }, branchId, hour,
    target: { kind: 'divisionDesigner', template }, active, onDiagnostics };
}

/** A template's division designer, as the game draws it. */
export function DivisionDesigner({ request, active }: { request: DesignerRequest; active: boolean }) {
  return <GameWindow {...designerWindow(request, active)} />;
}

registerDesignerSnapshot((request, active) => <DivisionDesigner request={request} active={active} />);
