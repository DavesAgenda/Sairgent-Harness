import { ReplayBus } from '../sim/replayBus';
import { SignalRecorder, type RecordedSignal } from '../sim/signalRecorder';
import { MockBus } from '../sim/mockBus';
import { agents } from '../sim/mockRoster';
import { computeLayout } from '../world/layoutEngine';
import { computeTubes } from '../world/tubePathComputer';
import type { AgentPresence, InboxItem, RuntimeSignal, SwoRecord } from '../types';
import happyPathRecording from '../fixtures/happy-path-recording.json';

/**
 * Minimal world-state reducer -- mirrors useWorkspaceState logic without React hooks.
 */
function buildWorldState(signals: RuntimeSignal[]) {
  const swos = new Map<string, SwoRecord>();
  const presence = new Map<string, AgentPresence>();
  const inbox: InboxItem[] = [];

  for (const signal of signals) {
    const { type, payload } = signal;
    switch (type) {
      case 'swo.created': {
        const swo = payload['swo'] as SwoRecord;
        swos.set(swo.id, swo);
        break;
      }
      case 'swo.updated': {
        const partial = payload['swo'] as Partial<SwoRecord> & { id: string };
        const existing = swos.get(partial.id);
        if (existing) swos.set(partial.id, { ...existing, ...partial });
        break;
      }
      case 'swo.completed': {
        const partial = payload['swo'] as Partial<SwoRecord> & { id: string };
        const existing = swos.get(partial.id);
        if (existing) swos.set(partial.id, { ...existing, ...partial, status: 'COMPLETED' });
        break;
      }
      case 'agent.presence.changed': {
        const { agentId, presence: p } = payload as { agentId: string; presence: AgentPresence };
        presence.set(agentId, p);
        break;
      }
      case 'inbox.item.added': {
        const item = payload['item'] as InboxItem;
        inbox.push(item);
        break;
      }
    }
  }

  const swoArray = Array.from(swos.values());
  const { desks, bench } = computeLayout(agents, swoArray, presence);
  const tubes = computeTubes(swoArray);
  return { desks, tubes, bench, inbox, swos, presence };
}

describe('replay recording: happy path fixture', () => {
  it('loads the recording fixture correctly', () => {
    const recording = happyPathRecording as RecordedSignal[];
    expect(recording.length).toBeGreaterThan(0);
    expect(recording[0]!.offsetMs).toBe(0);
    expect(recording[0]!.signal.type).toBe('swo.created');
  });

  it('replays all signals via replayInstant', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    const recording = happyPathRecording as RecordedSignal[];
    const count = replayBus.replayInstant(recording);

    expect(count).toBe(recording.length);
    expect(received).toHaveLength(recording.length);
  });

  it('final state: all 4 SWOs are COMPLETED', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    replayBus.replayInstant(happyPathRecording as RecordedSignal[]);

    const { swos } = buildWorldState(received);
    expect(swos.size).toBe(4);
    for (const swo of swos.values()) {
      expect(swo.status).toBe('COMPLETED');
    }
  });

  it('final state: perry, lois, lex, stacker all READY', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    replayBus.replayInstant(happyPathRecording as RecordedSignal[]);

    const { presence } = buildWorldState(received);
    expect(presence.get('perry')).toBe('READY');
    expect(presence.get('lois')).toBe('READY');
    expect(presence.get('lex')).toBe('READY');
    expect(presence.get('stacker')).toBe('READY');
  });

  it('final state: all tubes show direction up (complete)', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    replayBus.replayInstant(happyPathRecording as RecordedSignal[]);

    const { tubes } = buildWorldState(received);
    expect(tubes.length).toBeGreaterThan(0);
    for (const tube of tubes) {
      expect(tube.status).toBe('complete');
      expect(tube.direction).toBe('up');
    }
  });

  it('final state: inbox has 1 item from Perry', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    replayBus.replayInstant(happyPathRecording as RecordedSignal[]);

    const { inbox } = buildWorldState(received);
    expect(inbox).toHaveLength(1);
    expect(inbox[0]!.agentName).toBe('Perry');
  });

  it('final state: no active desks (all completed, agents on bench)', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    replayBus.replayInstant(happyPathRecording as RecordedSignal[]);

    const { desks } = buildWorldState(received);
    expect(desks).toHaveLength(0);
  });

  it('delegation tubes: 3 tubes exist (perry->lois, perry->lex, lois->stacker)', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    replayBus.replayInstant(happyPathRecording as RecordedSignal[]);

    const { tubes } = buildWorldState(received);
    expect(tubes).toHaveLength(3);

    const tubeKeys = tubes.map((t) => `${t.fromAgentId}->${t.toAgentId}`).sort();
    expect(tubeKeys).toEqual(['lois->stacker', 'perry->lex', 'perry->lois']);
  });
});

describe('SignalRecorder', () => {
  it('wraps a bus and records emitted signals with relative timestamps', () => {
    const bus = new MockBus();
    const recorder = new SignalRecorder();
    const wrapped = recorder.wrap(bus);

    const signal: RuntimeSignal = {
      type: 'swo.created',
      timestamp: Date.now(),
      payload: { swo: { id: 'test' } },
    };

    wrapped.emit(signal);

    expect(recorder.signals).toHaveLength(1);
    expect(recorder.signals[0]!.offsetMs).toBeGreaterThanOrEqual(0);
    expect(recorder.signals[0]!.signal).toEqual(signal);
  });

  it('stops recording when stop() is called', () => {
    const bus = new MockBus();
    const recorder = new SignalRecorder();
    const wrapped = recorder.wrap(bus);

    wrapped.emit({ type: 'swo.created', timestamp: 1, payload: {} });
    recorder.stop();
    wrapped.emit({ type: 'swo.created', timestamp: 2, payload: {} });

    expect(recorder.signals).toHaveLength(1);
  });

  it('toJSON produces valid JSON', () => {
    const bus = new MockBus();
    const recorder = new SignalRecorder();
    const wrapped = recorder.wrap(bus);
    wrapped.emit({ type: 'swo.created', timestamp: 1, payload: { swo: { id: 'x' } } });

    const json = recorder.toJSON();
    const parsed = JSON.parse(json);
    expect(Array.isArray(parsed)).toBe(true);
    expect(parsed).toHaveLength(1);
  });

  it('fromJSON round-trips correctly', () => {
    const bus = new MockBus();
    const recorder = new SignalRecorder();
    const wrapped = recorder.wrap(bus);
    wrapped.emit({ type: 'swo.created', timestamp: 1, payload: { swo: { id: 'x' } } });

    const json = recorder.toJSON();
    const loaded = SignalRecorder.fromJSON(json);
    expect(loaded).toHaveLength(1);
    expect(loaded[0]!.signal.type).toBe('swo.created');
  });

  it('clear resets all state', () => {
    const bus = new MockBus();
    const recorder = new SignalRecorder();
    const wrapped = recorder.wrap(bus);
    wrapped.emit({ type: 'swo.created', timestamp: 1, payload: {} });

    recorder.clear();

    expect(recorder.signals).toHaveLength(0);
    expect(recorder.recording).toBe(false);
  });
});

describe('ReplayBus', () => {
  it('replayInstant emits all signals synchronously', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    const signals: RuntimeSignal[] = [
      { type: 'swo.created', timestamp: 100, payload: { swo: { id: '1' } } },
      { type: 'swo.updated', timestamp: 200, payload: { swo: { id: '1', progress: 0.5 } } },
      { type: 'swo.completed', timestamp: 300, payload: { swo: { id: '1' } } },
    ];

    const count = replayBus.replayInstant(signals);

    expect(count).toBe(3);
    expect(received).toHaveLength(3);
    expect(received[0]!.type).toBe('swo.created');
    expect(received[2]!.type).toBe('swo.completed');
  });

  it('replay with timers emits signals on schedule', () => {
    vi.useFakeTimers();

    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    const signals: RuntimeSignal[] = [
      { type: 'swo.created', timestamp: 1000, payload: {} },
      { type: 'swo.updated', timestamp: 2000, payload: {} },
      { type: 'swo.completed', timestamp: 3000, payload: {} },
    ];

    replayBus.replay(signals);

    // At t=0, first signal fires immediately
    vi.advanceTimersByTime(0);
    expect(received).toHaveLength(1);

    // At t=1000, second signal fires
    vi.advanceTimersByTime(1000);
    expect(received).toHaveLength(2);

    // At t=2000, third signal fires
    vi.advanceTimersByTime(1000);
    expect(received).toHaveLength(3);

    vi.useRealTimers();
  });

  it('step() advances one signal at a time', () => {
    const replayBus = new ReplayBus();
    const received: RuntimeSignal[] = [];
    replayBus.subscribe((s) => received.push(s));

    const recording: RecordedSignal[] = [
      { offsetMs: 0, signal: { type: 'swo.created', timestamp: 100, payload: {} } },
      { offsetMs: 100, signal: { type: 'swo.updated', timestamp: 200, payload: {} } },
      { offsetMs: 200, signal: { type: 'swo.completed', timestamp: 300, payload: {} } },
    ];

    // Start replay but immediately pause
    replayBus.replay(recording);
    replayBus.pause();

    replayBus.step();
    expect(received).toHaveLength(1);

    replayBus.step();
    expect(received).toHaveLength(2);

    replayBus.step();
    expect(received).toHaveLength(3);

    expect(replayBus.playing).toBe(false);
  });

  it('emittedCount tracks total emitted signals', () => {
    const replayBus = new ReplayBus();
    replayBus.subscribe(() => {});

    const signals: RuntimeSignal[] = [
      { type: 'swo.created', timestamp: 100, payload: {} },
      { type: 'swo.updated', timestamp: 200, payload: {} },
    ];

    replayBus.replayInstant(signals);
    expect(replayBus.emittedCount).toBe(2);
  });
});
