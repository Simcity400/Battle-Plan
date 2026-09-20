/** Dispose on an actual unmount, not React's immediate cleanup/setup replay. */
export class EffectLifetime {
  private generation = 0;
  private owner: unknown;
  private dispose?: () => void;
  attach(owner: unknown, dispose: () => void): () => void {
    if (this.owner !== owner) this.dispose?.();
    this.owner = owner;
    this.dispose = dispose;
    const generation = ++this.generation;
    return () => queueMicrotask(() => {
      if (this.generation === generation) { this.dispose = undefined; dispose(); }
    });
  }
}
