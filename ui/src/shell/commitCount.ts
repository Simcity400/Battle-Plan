/** React commits since the last `takeCommits`. */
let commits = 0;

/** What React keeps of a rendered component, as far as attribution reads it. */
interface Fiber { child: Fiber | null; sibling: Fiber | null; flags: number; subtreeFlags: number; type: unknown; elementType: unknown; memoizedProps: unknown }
/** React's flag on a component that rendered in the commit; a skipped subtree does not carry it up. */
const PERFORMED_WORK = 1;

/** Components that name the part of the page they draw, and the context whose value names a widget. */
const zones = new Map<unknown, string>();
let widgetContext: unknown = null;
let attribute: ((zones: ReadonlySet<string>) => void) | null = null;

// A production React build reports each commit only to a DevTools hook, read once as react-dom loads.
const host = globalThis as { __REACT_DEVTOOLS_GLOBAL_HOOK__?: unknown };
if (host.__REACT_DEVTOOLS_GLOBAL_HOOK__ === undefined) {
  host.__REACT_DEVTOOLS_GLOBAL_HOOK__ = { supportsFiber: true, inject: () => 1, onCommitFiberRoot: (_renderer: number, root: { current: Fiber }) => {
    commits++;
    attribute?.(renderedZones(root.current));
  } };
}

/** The React commits since the last call. */
export function takeCommits(): number {
  const taken = commits;
  commits = 0;
  return taken;
}

/** Commits are told, while measuring, which parts of the page rendered in them. */
export function attributeCommits(sink: ((zones: ReadonlySet<string>) => void) | null): void { attribute = sink; }

/** Names the part of the page `component` draws, for commits that render inside it. */
export function commitZone(component: unknown, name: string): void { zones.set(component, name); }

/** The context whose value is the widget its subtree draws. */
export function widgetZones(context: unknown): void { widgetContext = context; }

/** Each named part of the page with a component that rendered in the commit, by its nearest named ancestor. */
function renderedZones(root: Fiber): Set<string> {
  const found = new Set<string>();
  const stack: [Fiber, string][] = [[root, 'unnamed']];
  for (let next = stack.pop(); next; next = stack.pop()) {
    const [fiber, zone] = next;
    for (let child = fiber.child; child; child = child.sibling) {
      const named = zones.get(child.elementType) ?? zones.get(child.type)
        ?? (widgetContext !== null && child.type === widgetContext ? `widget:${String((child.memoizedProps as { value?: unknown }).value)}` : zone);
      if (child.flags & PERFORMED_WORK) found.add(named);
      if (child.subtreeFlags & PERFORMED_WORK) stack.push([child, named]);
    }
  }
  return found;
}
