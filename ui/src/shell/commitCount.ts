/** React commits since the last `takeCommits`. */
let commits = 0;

/** A hook as React keeps it: its value, the next hook, and a queue on the hooks that set state (state, reducer, store). */
interface Hook { memoizedState: unknown; next: Hook | null; queue: { getSnapshot?: unknown } | null }
/** A context a component reads, with the value it read. */
interface ContextRead { context: { displayName?: string }; memoizedValue: unknown; next: ContextRead | null }
/** What React keeps of a rendered component, as far as attribution reads it. */
interface Fiber { child: Fiber | null; sibling: Fiber | null; return: Fiber | null; alternate: Fiber | null; flags: number; subtreeFlags: number; type: unknown;
  elementType: unknown; memoizedProps: unknown; memoizedState: unknown; dependencies: { firstContext: ContextRead | null } | null }
/** React's flag on a component that rendered in the commit; a skipped subtree does not carry it up. */
const PERFORMED_WORK = 1;

/** Components that name the part of the page they draw, and the context whose value names a widget. */
const zones = new Map<unknown, string>();
let widgetContext: unknown = null;
/** Each part of the page that rendered in a commit, and why: the component whose own render began the work there and
 * what changed in it (a state or store hook by its index, a context, props), with what components named while
 * rendering (`noteCause`). */
export type CommitCauses = ReadonlyMap<string, ReadonlyMap<string, number>>;
let attribute: ((zones: ReadonlySet<string>, causes: CommitCauses) => void) | null = null;
/** Causes named while rendering since the last commit, by zone. */
let noted = new Map<string, Map<string, number>>();

/** What the page's structure slot commits (`slotCommits`): its commits, and the components each widget rendered in them.
 * They are the slot's, never counted as commits of a frame. */
export interface SlotCommits { commits: number; widgets: Map<string, number>; causes: Map<string, Map<string, number>> }
let slot: SlotCommits | null = null;

// A production React build reports each commit only to a DevTools hook, read once as react-dom loads.
const host = globalThis as { __REACT_DEVTOOLS_GLOBAL_HOOK__?: unknown };
if (host.__REACT_DEVTOOLS_GLOBAL_HOOK__ === undefined) {
  host.__REACT_DEVTOOLS_GLOBAL_HOOK__ = { supportsFiber: true, inject: () => 1, onCommitFiberRoot: (_renderer: number, root: { current: Fiber }) => {
    if (slot) {
      slot.commits++;
      if (attribute) {
        const causes = noted;
        noted = new Map();
        renderedZones(root.current, causes);
        for (const [zone, named] of causes) {
          let held = slot.causes.get(zone);
          if (!held) slot.causes.set(zone, held = new Map());
          for (const [cause, count] of named) held.set(cause, (held.get(cause) ?? 0) + count);
        }
        widgetWork(root.current, slot.widgets);
      }
      return;
    }
    commits++;
    if (!attribute) return;
    const causes = noted;
    noted = new Map();
    attribute(renderedZones(root.current, causes), causes);
  } };
}

/** While measuring, `cause` of a render of the widget zone `zone`, named by the component as it renders. */
export function noteCause(zone: string, cause: string): void {
  if (!attribute) return;
  let held = noted.get(zone);
  if (!held) noted.set(zone, held = new Map());
  held.set(cause, (held.get(cause) ?? 0) + 1);
}

const MEMO = Symbol.for('react.memo'), FORWARD_REF = Symbol.for('react.forward_ref');
/** Whether `fiber` is a component that renders (a function, a class, memo or forwardRef), not a host element, a
 * fragment or a context provider, which pass their parent's render on. */
const renders = (fiber: Fiber): boolean => typeof fiber.type === 'function'
  || (typeof fiber.type === 'object' && fiber.type !== null && [MEMO, FORWARD_REF].includes((fiber.type as { $$typeof?: symbol }).$$typeof!));

const nameOf = (fiber: Fiber): string => {
  const type = (fiber.elementType ?? fiber.type) as { displayName?: string; name?: string; type?: { displayName?: string; name?: string }; render?: { name?: string } } | null;
  return type?.displayName ?? type?.name ?? type?.type?.displayName ?? type?.type?.name ?? type?.render?.name ?? 'component';
};

/** Why `fiber` rendered on its own (its parent did not): mounted, a state or store hook that holds another value (by
 * its index among the component's hooks), a context read with another value, or props it was given anew. */
function causeOf(fiber: Fiber): string {
  const before = fiber.alternate;
  const name = nameOf(fiber);
  if (!before) return `${name} mounted`;
  const changed: string[] = [];
  let now = fiber.memoizedState as Hook | null, was = before.memoizedState as Hook | null;
  if (now && typeof now === 'object' && 'next' in now) {
    for (let index = 0; now && was; index++, now = now.next, was = was.next) {
      if (now.queue && now.memoizedState !== was.memoizedState) changed.push(`${now.queue.getSnapshot ? 'store' : 'state'}#${index}`);
    }
  } else if (fiber.memoizedState !== before.memoizedState) changed.push('state');
  for (let read = fiber.dependencies?.firstContext ?? null, old = before.dependencies?.firstContext ?? null; read; read = read.next, old = old?.next ?? null) {
    if (!old || read.memoizedValue !== old.memoizedValue) changed.push(`context ${read.context.displayName ?? 'unnamed'}`);
  }
  if (!changed.length && fiber.memoizedProps !== before.memoizedProps) {
    const next = fiber.memoizedProps as Record<string, unknown> | null, prior = before.memoizedProps as Record<string, unknown> | null;
    const keys = next && prior ? Object.keys(next).filter(key => next[key] !== prior[key]) : [];
    changed.push(`props ${keys.join(',') || 'new'}`);
  }
  return `${name} ${changed.join(' ') || 'rendered'}`;
}

/** Runs `run`, the page's structure slot, and returns what React committed inside it. */
export function slotCommits(run: () => void): SlotCommits {
  const outer = slot;
  const taken: SlotCommits = { commits: 0, widgets: new Map(), causes: new Map() };
  slot = taken;
  try { run(); } finally { slot = outer; }
  return taken;
}

/** Into `widgets`, how many components of each widget rendered in the commit under `root`: production React keeps no
 * time per component, so the count stands for each widget's share of the commit. */
function widgetWork(root: Fiber, widgets: Map<string, number>): void {
  const stack: [Fiber, string][] = [[root, 'unnamed']];
  for (let next = stack.pop(); next; next = stack.pop()) {
    const [fiber, zone] = next;
    for (let child = fiber.child; child; child = child.sibling) {
      const named = zones.get(child.elementType) ?? zones.get(child.type)
        ?? (widgetContext !== null && child.type === widgetContext ? `widget:${String((child.memoizedProps as { value?: unknown }).value)}` : zone);
      if ((child.flags & PERFORMED_WORK) !== 0 && renders(child)) widgets.set(named, (widgets.get(named) ?? 0) + 1);
      if (child.subtreeFlags & PERFORMED_WORK) stack.push([child, named]);
    }
  }
}

/** The React commits since the last call. */
export function takeCommits(): number {
  const taken = commits;
  commits = 0;
  return taken;
}

/** Commits are told, while measuring, which parts of the page rendered in them, and why (`CommitCauses`). */
export function attributeCommits(sink: ((zones: ReadonlySet<string>, causes: CommitCauses) => void) | null): void { attribute = sink; noted = new Map(); }

/** Names the part of the page `component` draws, for commits that render inside it. */
export function commitZone(component: unknown, name: string): void { zones.set(component, name); }

/** The context whose value is the widget its subtree draws. */
export function widgetZones(context: unknown): void { widgetContext = context; }

/** Each named part of the page with a component that rendered in the commit, by its nearest named ancestor; into
 * `causes`, each widget's components whose work began there (rendered while the component above them did not), with why. */
function renderedZones(root: Fiber, causes: Map<string, Map<string, number>>): Set<string> {
  const found = new Set<string>();
  // Each fiber with whether the nearest component above it rendered in this commit.
  const stack: [Fiber, string, boolean][] = [[root, 'unnamed', false]];
  for (let next = stack.pop(); next; next = stack.pop()) {
    const [fiber, zone, above] = next;
    for (let child = fiber.child; child; child = child.sibling) {
      const named = zones.get(child.elementType) ?? zones.get(child.type)
        ?? (widgetContext !== null && child.type === widgetContext ? `widget:${String((child.memoizedProps as { value?: unknown }).value)}` : zone);
      const rendered = (child.flags & PERFORMED_WORK) !== 0;
      if (rendered) {
        found.add(named);
        if (!above && named.startsWith('widget:')) {
          let held = causes.get(named);
          if (!held) causes.set(named, held = new Map());
          const cause = causeOf(child);
          held.set(cause, (held.get(cause) ?? 0) + 1);
        }
      }
      if (child.subtreeFlags & PERFORMED_WORK) stack.push([child, named, renders(child) ? rendered : above]);
    }
  }
  return found;
}
