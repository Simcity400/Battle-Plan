let commits = 0;

interface Hook { memoizedState: unknown; next: Hook | null; queue: { getSnapshot?: unknown } | null }
interface ContextRead { context: { displayName?: string }; memoizedValue: unknown; next: ContextRead | null }
interface Fiber { child: Fiber | null; sibling: Fiber | null; return: Fiber | null; alternate: Fiber | null; flags: number; subtreeFlags: number; type: unknown;
  elementType: unknown; memoizedProps: unknown; memoizedState: unknown; dependencies: { firstContext: ContextRead | null } | null }
const PERFORMED_WORK = 1;

const zones = new Map<unknown, string>();
let widgetContext: unknown = null;
export type CommitCauses = ReadonlyMap<string, ReadonlyMap<string, number>>;
let attribute: ((zones: ReadonlySet<string>, causes: CommitCauses) => void) | null = null;

const host = globalThis as { __REACT_DEVTOOLS_GLOBAL_HOOK__?: unknown };
if (host.__REACT_DEVTOOLS_GLOBAL_HOOK__ === undefined) {
  host.__REACT_DEVTOOLS_GLOBAL_HOOK__ = { supportsFiber: true, inject: () => 1, onCommitFiberRoot: (_renderer: number, root: { current: Fiber }) => {
    commits++;
    if (!attribute) return;
    const causes = new Map<string, Map<string, number>>();
    attribute(renderedZones(root.current, causes), causes);
  } };
}

const MEMO = Symbol.for('react.memo'), FORWARD_REF = Symbol.for('react.forward_ref');
const renders = (fiber: Fiber): boolean => typeof fiber.type === 'function'
  || (typeof fiber.type === 'object' && fiber.type !== null && [MEMO, FORWARD_REF].includes((fiber.type as { $$typeof?: symbol }).$$typeof!));

const nameOf = (fiber: Fiber): string => {
  const type = (fiber.elementType ?? fiber.type) as { displayName?: string; name?: string; type?: { displayName?: string; name?: string }; render?: { name?: string } } | null;
  return type?.displayName ?? type?.name ?? type?.type?.displayName ?? type?.type?.name ?? type?.render?.name ?? 'component';
};

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

export function takeCommits(): number {
  const taken = commits;
  commits = 0;
  return taken;
}

export function attributeCommits(sink: ((zones: ReadonlySet<string>, causes: CommitCauses) => void) | null): void { attribute = sink; }

export function commitZone(component: unknown, name: string): void { zones.set(component, name); }

export function widgetZones(context: unknown): void { widgetContext = context; }

function renderedZones(root: Fiber, causes: Map<string, Map<string, number>>): Set<string> {
  const found = new Set<string>();
  const stack: [Fiber, string, boolean][] = [[root, 'unnamed', false]];
  for (let next = stack.pop(); next; next = stack.pop()) {
    const [fiber, zone, above] = next;
    for (let child = fiber.child; child; child = child.sibling) {
      const named = zones.get(child.elementType) ?? zones.get(child.type)
        ?? (widgetContext !== null && child.type === widgetContext ? `widget:${String((child.memoizedProps as { value?: unknown }).value)}` : zone);
      const rendered = (child.flags & PERFORMED_WORK) !== 0;
      if (rendered) {
        found.add(named);
        if (!above) {
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
