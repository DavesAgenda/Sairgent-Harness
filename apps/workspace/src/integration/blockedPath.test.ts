import { MockBus } from '../sim/mockBus';
import { runBlockedPath, resetIdCounter } from '../sim/mockScenarios';
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

describe('integration: blocked path', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetIdCounter();
  });

  afterEach(() => {
    vi.useRealTimers();
    resetIdCounter();
  });

  it('blocked tube appears mid-scenario after felicity hits blocker', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runBlockedPath(bus);
    // Advance to just after the BLOCKED signal at t=3500
    vi.advanceTimersByTime(3600);

    const { tubes } = buildWorldState(signals);
    const blockedTube = tubes.find(
      (t) => t.fromAgentId === 'perry' && t.toAgentId === 'felicity',
    );
    expect(blockedTube).toBeDefined();
    expect(blockedTube!.status).toBe('blocked');
  });

  it('tube resolves to active after felicity unblocked at t=6000', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runBlockedPath(bus);
    vi.advanceTimersByTime(6100);

    const { tubes } = buildWorldState(signals);
    const felicityTube = tubes.find(
      (t) => t.fromAgentId === 'perry' && t.toAgentId === 'felicity',
    );
    expect(felicityTube).toBeDefined();
    expect(felicityTube!.status).toBe('active');
  });

  it('final state: all SWOs completed', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runBlockedPath(bus);
    vi.runAllTimers();

    const { swos } = buildWorldState(signals);
    for (const swo of swos.values()) {
      expect(swo.status).toBe('COMPLETED');
    }
  });

  it('final state: tube direction is up (complete)', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runBlockedPath(bus);
    vi.runAllTimers();

    const { tubes } = buildWorldState(signals);
    expect(tubes.length).toBeGreaterThan(0);
    for (const tube of tubes) {
      expect(tube.status).toBe('complete');
      expect(tube.direction).toBe('up');
    }
  });

  it('final state: inbox has 1 item', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runBlockedPath(bus);
    vi.runAllTimers();

    const { inbox } = buildWorldState(signals);
    expect(inbox).toHaveLength(1);
  });

  it('final state: perry and felicity back to READY', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runBlockedPath(bus);
    vi.runAllTimers();

    const { presence } = buildWorldState(signals);
    expect(presence.get('perry')).toBe('READY');
    expect(presence.get('felicity')).toBe('READY');
  });
});
