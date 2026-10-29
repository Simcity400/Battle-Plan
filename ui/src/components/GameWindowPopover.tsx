import { useId, useState } from 'react';
import type { ReactNode, SyntheticEvent } from 'react';
import type { DashboardClient } from '../dashboard/client';
import { Icon } from './Icons';
import { loadable } from './loadable';
import { Popover, PopoverBody, usePopover } from './Popover';
import WidgetState from './WidgetState';
import type { GameWindowTarget } from '../generated/dashboard';
import './GameWindowPopover.css';

// The renderer loads only once a window is first opened.
const GameWindow = loadable(() => import('./GameWindow').then(module => module.default),
  () => <div className="game-window"><WidgetState kind="loading" label="Loading window…" /></div>);

const stop = (event: SyntheticEvent) => event.stopPropagation();
/** The popover sits in the trigger's React tree, so its pointer, focus, menu and key events
 * would reach the trigger's owners (a table row's tooltip and menu); they stay inside.
 * Escape still reaches the popover, which closes. */
const fence = {
  onPointerMove: stop, onFocus: stop, onBlur: stop, onContextMenu: stop,
  onKeyDown: (event: SyntheticEvent & { key: string }) => { if (event.key !== 'Escape') event.stopPropagation(); },
};

/** A button that opens a game window in a popover. Nothing is loaded or asked for until it opens. */
export default function GameWindowPopover({ client, source, branchId, hour, target, label, heading, className, children, onDiagnostics }: {
  client: DashboardClient;
  source: { sourceId: string; revision: string };
  branchId: string;
  hour: number;
  target: GameWindowTarget;
  /** The trigger's accessible name. */
  label: string;
  /** The heading until the window names itself. */
  heading: string;
  className?: string;
  children: ReactNode;
  onDiagnostics?: (lines: string[]) => void;
}) {
  const id = useId();
  const menu = usePopover('start');
  const [title, setTitle] = useState<string | null>(null);
  const shown = title ?? heading;
  return <>
    <button ref={menu.trigger} type="button" className={className} popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}
      aria-label={label}>{children}</button>
    <Popover menu={menu} id={id} className="game-window-popover" aria-label={shown}>
      {/* Focus moves into the window as it opens, so the trigger's owners see it leave. */}
      <div className="game-window-popover-body" tabIndex={-1} data-popover-autofocus {...fence}>
        {/* The shared heading, keeping the in-game name's case in its close label. */}
        <div className="ui-popover-heading"><strong>{shown}</strong><div className="ui-popover-actions">
          <button type="button" className="icon-button ui-popover-action" aria-label={`Close ${shown}`} onClick={menu.close}><Icon name="close" /></button>
        </div></div>
        <PopoverBody open={menu.open}>{() => <GameWindow client={client} source={source} branchId={branchId} hour={hour} target={target} active={menu.open}
          onTitle={setTitle} onDiagnostics={onDiagnostics} />}</PopoverBody>
      </div>
    </Popover>
  </>;
}
