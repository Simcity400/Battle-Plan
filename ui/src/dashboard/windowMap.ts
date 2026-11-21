interface Node<V> { key: number; value: V; left: Node<V> | null; right: Node<V> | null; height: number; size: number }
const height = <V>(node: Node<V> | null) => node?.height ?? 0;
const node = <V>(key: number, value: V, left: Node<V> | null, right: Node<V> | null): Node<V> =>
  ({ key, value, left, right, height: 1 + Math.max(height(left), height(right)), size: 1 + (left?.size ?? 0) + (right?.size ?? 0) });
function balance<V>(root: Node<V>): Node<V> {
  if (height(root.left) - height(root.right) > 1) {
    let left = root.left!;
    if (height(left.right) > height(left.left)) { const pivot = left.right!; left = node(pivot.key, pivot.value, node(left.key, left.value, left.left, pivot.left), pivot.right); }
    return node(left.key, left.value, left.left, node(root.key, root.value, left.right, root.right));
  }
  if (height(root.right) - height(root.left) > 1) {
    let right = root.right!;
    if (height(right.left) > height(right.right)) { const pivot = right.left!; right = node(pivot.key, pivot.value, pivot.left, node(right.key, right.value, pivot.right, right.right)); }
    return node(right.key, right.value, node(root.key, root.value, root.left, right.left), right.right);
  }
  return root;
}
function set<V>(root: Node<V> | null, key: number, value: V): Node<V> {
  if (!root) return node(key, value, null, null);
  if (key === root.key) return Object.is(root.value, value) ? root : node(key, value, root.left, root.right);
  const child = set(key < root.key ? root.left : root.right, key, value);
  if (child === (key < root.key ? root.left : root.right)) return root;
  return balance(node(root.key, root.value, key < root.key ? child : root.left, key > root.key ? child : root.right));
}
export class WindowMap<V> implements ReadonlyMap<number, V> {
  private root: Node<V> | null;
  constructor(root: Node<V> | null = null) { this.root = root; }
  get size(): number { return this.root?.size ?? 0; }
  get(key: number): V | undefined { let node = this.root; while (node) { if (key === node.key) return node.value; node = key < node.key ? node.left : node.right; } return undefined; }
  has(key: number): boolean { let node = this.root; while (node) { if (key === node.key) return true; node = key < node.key ? node.left : node.right; } return false; }
  with(key: number, value: V): WindowMap<V> { const root = set(this.root, key, value); return root === this.root ? this : new WindowMap(root); }
  *entries(): MapIterator<[number, V]> {
    const stack: Node<V>[] = []; let current = this.root;
    while (current || stack.length) { while (current) { stack.push(current); current = current.left; } const next = stack.pop()!; yield [next.key, next.value]; current = next.right; }
  }
  *keys(): MapIterator<number> { for (const [key] of this.entries()) yield key; }
  *values(): MapIterator<V> { for (const [, value] of this.entries()) yield value; }
  [Symbol.iterator](): MapIterator<[number, V]> { return this.entries(); }
  forEach(callbackfn: (value: V, key: number, map: ReadonlyMap<number, V>) => void, thisArg?: unknown): void { for (const [key, value] of this) callbackfn.call(thisArg, value, key, this); }
}
export function withWindowEntries<V>(map: ReadonlyMap<number, V>, entries: readonly (readonly [number, V])[]): ReadonlyMap<number, V> {
  if (!entries.length) return map;
  let next = map instanceof WindowMap ? map : new WindowMap<V>();
  if (!(map instanceof WindowMap)) for (const [key, value] of map) next = next.with(key, value);
  for (const [key, value] of entries) next = next.with(key, value);
  return next;
}
