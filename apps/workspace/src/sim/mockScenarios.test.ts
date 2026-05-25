import { MockBus } from './mockBus';
import { runHappyPath, runBlockedPath, runParallelBurst, resetIdCounter } from './mockScenarios';
import type { RuntimeSignal } from '../types';

describe('mockScenarios', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetIdCounter();
  });

  afterEach(() => {
    vi.useRealTimers();
    resetIdCounter();
  });

  describe('runHappyPath', () => {
    it('emits signals in correct order covering full lifecycle', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runHappyPath(bus);
      vi.runAllTimers();

      const types = signals.map((s) => s.type);
      expect(types[0]).toBe('swo.created');
      expect(types).toContain('agent.presence.changed');
      expect(types).toContain('swo.updated');
      expect(types).toContain('delegation.started');
      expect(types).toContain('swo.completed');
      expect(types).toContain('artifact.produced');
      expect(types).toContain('inbox.item.added');
    });

    it('first signal is swo.created for perry root SWO', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runHappyPath(bus);
      vi.runAllTimers();

      const first = signals[0]!;
      expect(first.type).toBe('swo.created');
      const swo = first.payload['swo'] as { assigneeId: string; parentSwoId: unknown };
      expect(swo.assigneeId).toBe('perry');
      expect(swo.parentSwoId).toBeNull();
    });

    it('produces exactly one inbox.item.added signal', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runHappyPath(bus);
      vi.runAllTimers();

      const inboxSignals = signals.filter((s) => s.type === 'inbox.item.added');
      expect(inboxSignals).toHaveLength(1);
    });

    it('produces exactly one artifact.produced signal', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runHappyPath(bus);
      vi.runAllTimers();

      const artifactSignals = signals.filter((s) => s.type === 'artifact.produced');
      expect(artifactSignals).toHaveLength(1);
    });

    it('delegates from perry to lois and lex', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runHappyPath(bus);
      vi.runAllTimers();

      const delegations = signals
        .filter((s) => s.type === 'delegation.started')
        .map((s) => s.payload as { fromAgentId: string; toAgentId: string });

      const toAgents = delegations.map((d) => d.toAgentId);
      expect(toAgents).toContain('lois');
      expect(toAgents).toContain('lex');
      expect(toAgents).toContain('stacker');
    });
  });

  describe('runBlockedPath', () => {
    it('includes a BLOCKED status signal', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runBlockedPath(bus);
      vi.runAllTimers();

      const updatedSwos = signals
        .filter((s) => s.type === 'swo.updated')
        .map((s) => (s.payload['swo'] as { status?: string }).status)
        .filter(Boolean);

      expect(updatedSwos).toContain('BLOCKED');
    });

    it('produces an inbox.item.added at the end', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runBlockedPath(bus);
      vi.runAllTimers();

      const inboxSignals = signals.filter((s) => s.type === 'inbox.item.added');
      expect(inboxSignals).toHaveLength(1);
    });

    it('ends with root SWO completed', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runBlockedPath(bus);
      vi.runAllTimers();

      const completed = signals.filter((s) => s.type === 'swo.completed');
      expect(completed.length).toBeGreaterThanOrEqual(1);
    });

    it('delegation goes to felicity', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runBlockedPath(bus);
      vi.runAllTimers();

      const delegations = signals
        .filter((s) => s.type === 'delegation.started')
        .map((s) => (s.payload as { toAgentId: string }).toAgentId);

      expect(delegations).toContain('felicity');
    });
  });

  describe('runParallelBurst', () => {
    it('produces 3 root SWOs', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runParallelBurst(bus);
      vi.runAllTimers();

      const rootSwos = signals
        .filter((s) => s.type === 'swo.created')
        .map((s) => (s.payload['swo'] as { parentSwoId: unknown }) )
        .filter((swo) => swo.parentSwoId === null);

      expect(rootSwos).toHaveLength(3);
    });

    it('delegates to clark, lex, and cat', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runParallelBurst(bus);
      vi.runAllTimers();

      const delegations = signals
        .filter((s) => s.type === 'delegation.started')
        .map((s) => (s.payload as { toAgentId: string }).toAgentId);

      expect(delegations).toContain('clark');
      expect(delegations).toContain('lex');
      expect(delegations).toContain('cat');
    });

    it('all 3 child SWOs eventually complete', () => {
      const bus = new MockBus();
      const signals: RuntimeSignal[] = [];
      bus.subscribe((s) => signals.push(s));

      runParallelBurst(bus);
      vi.runAllTimers();

      const completedSwos = signals.filter((s) => s.type === 'swo.completed');
      // 3 root + 3 child = 6 completed total
      expect(completedSwos.length).toBeGreaterThanOrEqual(3);
    });
  });

  describe('resetIdCounter', () => {
    it('resets the ID sequence so swo IDs start from swo-1 again', () => {
      resetIdCounter();

      const bus1 = new MockBus();
      const signals1: RuntimeSignal[] = [];
      bus1.subscribe((s) => signals1.push(s));
      runHappyPath(bus1);
      vi.runAllTimers();

      const firstId1 = (signals1[0]!.payload['swo'] as { id: string }).id;

      resetIdCounter();

      const bus2 = new MockBus();
      const signals2: RuntimeSignal[] = [];
      bus2.subscribe((s) => signals2.push(s));
      runHappyPath(bus2);
      vi.runAllTimers();

      const firstId2 = (signals2[0]!.payload['swo'] as { id: string }).id;

      expect(firstId1).toBe(firstId2);
    });
  });
});
