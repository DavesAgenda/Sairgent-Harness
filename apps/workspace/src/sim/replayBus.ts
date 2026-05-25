import type { Bus, RuntimeSignal } from '../types';
import type { RecordedSignal } from './signalRecorder';

/**
 * ReplayBus -- Replays recorded signal sequences with speed control,
 * pause/resume, step-through, and instant mode capabilities.
 */
export class ReplayBus implements Bus {
  private listeners = new Set<(signal: RuntimeSignal) => void>();
  private timers: ReturnType<typeof setTimeout>[] = [];
  private _speed = 1;
  private _paused = false;
  private _playing = false;
  private pendingSignals: { signal: RuntimeSignal; delay: number }[] = [];
  private pausedAt = 0;
  private _emittedCount = 0;

  subscribe(callback: (signal: RuntimeSignal) => void): () => void {
    this.listeners.add(callback);
    return () => {
      this.listeners.delete(callback);
    };
  }

  emit(signal: RuntimeSignal): void {
    this._emittedCount++;
    for (const listener of this.listeners) {
      listener(signal);
    }
  }

  get speed(): number {
    return this._speed;
  }

  set speed(value: number) {
    this._speed = Math.max(0.1, Math.min(10, value));
  }

  get paused(): boolean {
    return this._paused;
  }

  get playing(): boolean {
    return this._playing;
  }

  get emittedCount(): number {
    return this._emittedCount;
  }

  /**
   * Replay a recorded signal array with original timing, adjusted by speed.
   * Accepts either RuntimeSignal[] (using timestamp-based offsets) or
   * RecordedSignal[] (using explicit offsetMs).
   */
  replay(signals: RuntimeSignal[] | RecordedSignal[]): void {
    this.stop();
    if (signals.length === 0) return;

    this._playing = true;
    this._paused = false;

    // Detect format: RecordedSignal has { offsetMs, signal }, RuntimeSignal has { type, timestamp }
    const isRecorded = signals.length > 0 && 'offsetMs' in signals[0]!;

    if (isRecorded) {
      const recorded = signals as RecordedSignal[];
      this.pendingSignals = recorded.map((r) => ({
        signal: r.signal,
        delay: r.offsetMs / this._speed,
      }));
    } else {
      const raw = signals as RuntimeSignal[];
      const baseTime = raw[0]!.timestamp;
      this.pendingSignals = raw.map((signal) => ({
        signal,
        delay: (signal.timestamp - baseTime) / this._speed,
      }));
    }

    this.scheduleRemaining(0);
  }

  /**
   * Replay all signals instantly without any delays. Perfect for tests.
   * Returns the total number of signals emitted.
   */
  replayInstant(signals: RuntimeSignal[] | RecordedSignal[]): number {
    this.stop();
    if (signals.length === 0) return 0;

    const isRecorded = signals.length > 0 && 'offsetMs' in signals[0]!;

    let count = 0;
    if (isRecorded) {
      for (const r of signals as RecordedSignal[]) {
        this.emit(r.signal);
        count++;
      }
    } else {
      for (const signal of signals as RuntimeSignal[]) {
        this.emit(signal);
        count++;
      }
    }

    return count;
  }

  private scheduleRemaining(startOffset: number): void {
    for (const { signal, delay } of this.pendingSignals) {
      const adjustedDelay = Math.max(0, delay - startOffset);
      const timer = setTimeout(() => {
        if (this._paused) return;
        this.emit(signal);
        // Remove from pending
        this.pendingSignals = this.pendingSignals.filter((p) => p.signal !== signal);
        if (this.pendingSignals.length === 0) {
          this._playing = false;
        }
      }, adjustedDelay);
      this.timers.push(timer);
    }
  }

  pause(): void {
    if (!this._playing || this._paused) return;
    this._paused = true;
    this.pausedAt = Date.now();
    // Clear all pending timers
    for (const t of this.timers) clearTimeout(t);
    this.timers = [];
  }

  resume(): void {
    if (!this._paused) return;
    this._paused = false;
    // Recalculate remaining delays
    const elapsed = Date.now() - this.pausedAt;
    this.pendingSignals = this.pendingSignals.map((p) => ({
      ...p,
      delay: Math.max(0, p.delay - elapsed),
    }));
    this.scheduleRemaining(0);
  }

  /** Emit the next pending signal immediately (step-through mode). */
  step(): void {
    if (this.pendingSignals.length === 0) return;
    const next = this.pendingSignals.shift()!;
    this.emit(next.signal);
    if (this.pendingSignals.length === 0) {
      this._playing = false;
    }
  }

  stop(): void {
    for (const t of this.timers) clearTimeout(t);
    this.timers = [];
    this.pendingSignals = [];
    this._playing = false;
    this._paused = false;
  }

  /** Load signals from a JSON string (RecordedSignal[] format). */
  static fromJSON(json: string): RecordedSignal[] {
    return JSON.parse(json) as RecordedSignal[];
  }

  /** Load raw RuntimeSignal[] from a JSON string. */
  static rawFromJSON(json: string): RuntimeSignal[] {
    return JSON.parse(json) as RuntimeSignal[];
  }
}
