import { approach } from '../charts/easing.ts';

export class Tween {
  value: number;
  target: number;

  constructor(value: number) {
    this.value = value;
    this.target = value;
  }

  step(dt: number, tau: number): boolean {
    this.value = approach(this.value, this.target, dt, tau);
    return this.value !== this.target;
  }

  jump(value: number): void {
    this.value = value;
    this.target = value;
  }
}
