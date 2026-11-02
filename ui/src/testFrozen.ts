export function deepFreeze<T>(value: T, seen = new WeakSet<object>()): T {
  if (value === null || typeof value !== 'object' && typeof value !== 'function' || seen.has(value)) return value;
  seen.add(value);
  const refuse = () => { throw new TypeError('Frozen test input'); };
  if (value instanceof Map) {
    for (const [key, item] of value) { deepFreeze(key, seen); deepFreeze(item, seen); }
    if (!Object.isFrozen(value)) for (const name of ['set', 'delete', 'clear']) Object.defineProperty(value, name, { value: refuse });
  } else if (value instanceof Set) {
    for (const item of value) deepFreeze(item, seen);
    if (!Object.isFrozen(value)) for (const name of ['add', 'delete', 'clear']) Object.defineProperty(value, name, { value: refuse });
  }
  for (const key of Reflect.ownKeys(value)) {
    const property = Object.getOwnPropertyDescriptor(value, key)!;
    if ('value' in property) deepFreeze(property.value, seen);
  }
  Object.freeze(value);
  return value;
}
