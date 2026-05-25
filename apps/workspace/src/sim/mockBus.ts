import type { Bus, RuntimeSignal } from '../types';

export class MockBus implements Bus {
  private listeners = new Set<(signal: RuntimeSignal) => void>();

  subscribe(callback: (signal: RuntimeSignal) => void): () => void {
    this.listeners.add(callback);
    return () => {
      this.listeners.delete(callback);
    };
  }

  emit(signal: RuntimeSignal): void {
    for (const listener of this.listeners) {
      listener(signal);
    }
  }
}

export const bus = new MockBus();
