import { MockBus } from '../sim/mockBus';
import { runParallelBurst, resetIdCounter } from '../sim/mockScenarios';
import { agents } from '../sim/mockRoster';
import { computeLayout } from '../world/layoutEngine';
import { computeTubes } from '../world/tubePathComputer';
import type { AgentPresence, InboxItem, RuntimeSignal, SwoRecord } from '../types';

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

describe('integration: parallel burst', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetIdCounter();
  });

  afterEach(() => {
    vi.useRealTimers();
    resetIdCounter();
  });

  it('mid-scenario: 3 simultaneous root SWOs are in flight', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runParallelBurst(bus);
    // All three jobs submitted by t=1000, all active by t=2500
    vi.advanceTimersByTime(2500);

    const { swos } = buildWorldState(signals);
    const rootSwos = Array.from(swos.values()).filter((s) => s.parentSwoId === null);
    expect(rootSwos.length).toBe(3);

    const inProgress = rootSwos.filter((s) => s.status === 'IN_PROGRESS');
    expect(inProgress.length).toBeGreaterThanOrEqual(1);
  });

  it('mid-scenario: 3 child SWOs are active across clark, lex, cat', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runParallelBurst(bus);
    // All child delegations done by t=2200
    vi.advanceTimersByTime(2500);

    const { swos } = buildWorldState(signals);
    const childSwos = Array.from(swos.values()).filter((s) => s.parentSwoId !== null);
    const assignees = childSwos.map((s) => s.assigneeId);

    expect(assignees).toContain('clark');
    expect(assignees).toContain('lex');
    expect(assignees).toContain('cat');
  });

  it('mid-scenario: 3 tubes exist connecting perry to child assignees', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runParallelBurst(bus);
    vi.advanceTimersByTime(2500);

    const { tubes } = buildWorldState(signals);
    expect(tubes.length).toBe(3);

    const toAgents = tubes.map((t) => t.toAgentId);
    expect(toAgents).toContain('clark');
    expect(toAgents).toContain('lex');
    expect(toAgents).toContain('cat');
  });

  it('final state: all 6 SWOs completed', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runParallelBurst(bus);
    vi.runAllTimers();

    const { swos } = buildWorldState(signals);
    expect(swos.size).toBe(6);
    for (const swo of swos.values()) {
      expect(swo.status).toBe('COMPLETED');
    }
  });

  it('final state: all tubes complete', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runParallelBurst(bus);
    vi.runAllTimers();

    const { tubes } = buildWorldState(signals);
    expect(tubes.length).toBe(3);
    for (const tube of tubes) {
      expect(tube.status).toBe('complete');
      expect(tube.direction).toBe('up');
    }
  });

  it('final state: no active desks remain', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runParallelBurst(bus);
    vi.runAllTimers();

    const { desks } = buildWorldState(signals);
    expect(desks).toHaveLength(0);
  });
});
