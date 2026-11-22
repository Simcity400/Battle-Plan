export class StructureOutbox {
  private waiting = false;
  private readonly send: () => boolean;

  constructor(send: () => boolean) { this.send = send; }

  flush(): void {
    if (!this.waiting) this.waiting = this.send();
  }

  taken(): void {
    this.waiting = false;
    this.flush();
  }
}
