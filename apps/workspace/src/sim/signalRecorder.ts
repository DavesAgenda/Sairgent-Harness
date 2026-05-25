import type { Bus, RuntimeSignal } from '../types';

export interface RecordedSignal {
  /** Milliseconds since the start of recording. */
  offsetMs: number;
  signal: RuntimeSignal;
}

/**
 * Wraps a Bus to record all emitted signals with relative timestamps.
 * Supports export to JSON for replay testing and save/load via file download.
 */
export class SignalRecorder {
  readonly signals: RecordedSignal[] = [];
  private _recording = false;
  private _startTime = 0;

  get recording(): boolean {
    return this._recording;
  }

  wrap(bus: Bus): Bus {
    const recorder = this;
    recorder._recording = true;
    recorder._startTime = Date.now();
    return {
      subscribe: bus.subscribe.bind(bus),
      emit(signal: RuntimeSignal) {
        if (recorder._recording) {
          recorder.signals.push({
            offsetMs: Date.now() - recorder._startTime,
            signal,
          });
        }
        bus.emit(signal);
      },
    };
  }

  stop(): void {
    this._recording = false;
  }

  toJSON(): string {
    return JSON.stringify(this.signals, null, 2);
  }

  /** Export raw RuntimeSignal[] (preserving original timestamps) for replay compatibility. */
  toRawSignals(): RuntimeSignal[] {
    return this.signals.map((r) => r.signal);
  }

  clear(): void {
    this.signals.length = 0;
    this._recording = false;
  }

  /** Download the recorded signals as a JSON file. */
  download(filename = 'workspace-recording.json'): void {
    const blob = new Blob([this.toJSON()], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
  }

  /** Load RecordedSignal[] from a File (e.g., from a file input). */
  static async fromFile(file: File): Promise<RecordedSignal[]> {
    const text = await file.text();
    return JSON.parse(text) as RecordedSignal[];
  }

  /** Load RecordedSignal[] from a JSON string. */
  static fromJSON(json: string): RecordedSignal[] {
    return JSON.parse(json) as RecordedSignal[];
  }
}
