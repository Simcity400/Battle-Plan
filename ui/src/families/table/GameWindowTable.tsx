import { useCallback, useEffect, useId, useRef, useState, useSyncExternalStore, type ComponentType } from 'react';
import WidgetState from '../../components/WidgetState';
import FollowControl from '../../charts/drill/FollowControl';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import GateState from '../../dashboard/GateState';
import type { WidgetProps } from '../../dashboard/definitions';
import {
  claimGameWindowSlot,
  gameWindow,
  releaseGameWindowSlot,
  subscribeGameWindows,
  updateGameWindowSlotPosition,
  windowDayLabel,
  type GameWindowProps,
  type GameWindowSelection,
  type ShownWindow,
  type WindowPosition,
} from '../../dashboard/gameWindows';
import type { BoundTrack } from '../../dashboard/model';
import { TimeRegion, useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../host/DetailWindowHost';
import type { AnyTableWidgetDeclaration } from './declaration.ts';
import type { InstalledAction } from '../contract.ts';
import { FamilyWidget } from '../host/registry';
import { gameWindowSelectionAt, openGameWindow, type OpenedGameWindow } from './gameWindowOpen.ts';
import './gameWindowTable.css';

const NO_LINES: readonly string[] = [];
const WINDOW_BREAKDOWN = 'window';

export interface GameWindowRow<S extends GameWindowSelection> {
  readonly kind: S['kind'];
  readonly select: (track: BoundTrack, key: string, meta: unknown, hour: number) => S | null;
  readonly label: string;
}

export interface GameWindowTableProps<S extends GameWindowSelection> extends WidgetProps {
  readonly declaration: AnyTableWidgetDeclaration;
  readonly window: GameWindowRow<S>;
}

export default function GameWindowTable<S extends GameWindowSelection>({ declaration, window: row, ...props }: GameWindowTableProps<S>) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lookup = useCallback(() => gameWindow(row.kind), [row.kind]);
  const Window = useSyncExternalStore(subscribeGameWindows, lookup, lookup) as unknown as ComponentType<GameWindowProps<S>> | null;
  const slotOwner = useId();
  const anchor = useRef<HTMLDivElement>(null);
  const drill = useDrill(runtime, useRegion(widget));
  const dispatch = drill.dispatch;
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries]);
  const [picked, setPicked] = useState<OpenedGameWindow<S> | null>(null);
  const opened = picked?.scope === scope ? picked : null;
  const [inherited, setInherited] = useState<WindowPosition | null>(null);
  const [shownFor, setShownFor] = useState<{ readonly key: string; readonly shown: ShownWindow } | null>(null);
  const openedKey = useRef<string | null>(null);
  const [lines, setLines] = useState<readonly string[]>(NO_LINES);
  const onWindowDiagnostics = useCallback((next: string[]) =>
    setLines(prev => (prev.length === next.length && prev.every((line, i) => line === next[i]) ? prev : next)), []);

  const handOver = useCallback(() => {
    dispatch({ type: 'close' });
    setPicked(null);
    setShownFor(null);
    setInherited(null);
    setLines(NO_LINES);
  }, [dispatch]);
  const close = useCallback(() => {
    releaseGameWindowSlot(row.kind, slotOwner);
    handOver();
  }, [row.kind, slotOwner, handOver]);
  useEffect(() => close, [scope, close]);

  const activate = useCallback(({ subject: key, meta, hour }: InstalledAction) => {
    const clicked = Window && track ? row.select(track, key, meta, hour) : null;
    if (!clicked) return;
    const { opened: next, drill: opening } = openGameWindow(scope, key, clicked, WINDOW_BREAKDOWN);
    setInherited(claimGameWindowSlot(row.kind, slotOwner, handOver));
    setPicked(next);
    dispatch(opening);
  }, [Window, track, row, slotOwner, handOver, scope, dispatch]);

  const open = drill.state.breakdown !== null && opened !== null && Window !== null;
  const at = useRegionHour(runtime, drill.region, active && open);
  const shut = drill.gate.stage.stage !== 'open' && drill.gate.stage.stage !== 'unbound';
  const selection = open && at !== null ? gameWindowSelectionAt(opened, at) : null;
  openedKey.current = opened?.key ?? null;
  const onShown = useCallback((next: ShownWindow | null) => {
    const key = openedKey.current;
    setShownFor(next && key !== null ? { key, shown: next } : null);
  }, []);
  const shown = opened && shownFor?.key === opened.key ? shownFor.shown : null;
  const title = shown?.title ?? row.label;

  return <div ref={anchor} className="game-window-table">
    <FamilyWidget
      {...props}
      family="table"
      declaration={declaration}
      selectedKey={opened?.key ?? null}
      onActivate={Window ? activate : undefined}
      extraWarnings={open ? lines : NO_LINES}
    />
    <DetailWindowHost
      open={open}
      subject={opened?.key ?? null}
      hour={at}
      title={title}
      day={shut ? '' : shown ? windowDayLabel(shown) : at === null ? '' : dayLabel(at)}
      placement="center"
      anchor={anchor}
      initialPosition={inherited}
      settled={shown !== null}
      onPositionChange={(position: WindowPosition) => updateGameWindowSlotPosition(row.kind, slotOwner, position)}
      headerActions={<FollowControl following={drill.following} onFollow={drill.follow} onPause={drill.pause} />}
      className="detail-window-game"
      closeLabel={`Close ${title}`}
      onClose={close}
    >
      <TimeRegion runtime={runtime} region={drill.region}>
        {shut ? <GateState gate={drill.gate} loading="Loading…" />
          : open && at === null ? <WidgetState kind="outside" />
          : selection && Window ? <Window
            selection={selection}
            active={active}
            client={runtime.client}
            onDiagnostics={onWindowDiagnostics}
            onShown={onShown}
            onClose={close}
          /> : null}
      </TimeRegion>
    </DetailWindowHost>
  </div>;
}
