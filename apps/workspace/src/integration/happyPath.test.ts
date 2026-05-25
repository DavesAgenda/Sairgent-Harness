import { MockBus } from '../sim/mockBus';
import { runHappyPath, resetIdCounter } from '../sim/mockScenarios';
import { agents } from '../sim/mockRoster';
import { computeLayout } from '../world/layoutEngine';
import { computeTubes } from '../world/tubePathComputer';
import type { AgentPresence, InboxItem, RuntimeSignal, SwoRecord } from '../types';

/**
 * Minimal world-state reducer — mirrors useWorkspaceState logic without React hooks.
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

describe('integration: happy path', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetIdCounter();
  });

  afterEach(() => {
    vi.useRealTimers();
    resetIdCounter();
  });

  it('collects all signals after running happy path', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runHappyPath(bus);
    vi.runAllTimers();

    expect(signals.length).toBeGreaterThan(0);
  });

  it('final state: inbox has 1 item', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runHappyPath(bus);
    vi.runAllTimers();

    const { inbox } = buildWorldState(signals);
    expect(inbox).toHaveLength(1);
    expect(inbox[0]!.agentName).toBe('Perry');
  });

  it('final state: all SWOs are COMPLETED', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runHappyPath(bus);
    vi.runAllTimers();

    const { swos } = buildWorldState(signals);
    for (const swo of swos.values()) {
      expect(swo.status).toBe('COMPLETED');
    }
  });

  it('final state: all tubes show direction up (complete)', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runHappyPath(bus);
    vi.runAllTimers();

    const { tubes } = buildWorldState(signals);
    expect(tubes.length).toBeGreaterThan(0);
    for (const tube of tubes) {
      expect(tube.status).toBe('complete');
      expect(tube.direction).toBe('up');
    }
  });

  it('final state: perry, lois, lex, stacker all READY', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runHappyPath(bus);
    vi.runAllTimers();

    const { presence } = buildWorldState(signals);
    expect(presence.get('perry')).toBe('READY');
    expect(presence.get('lois')).toBe('READY');
    expect(presence.get('lex')).toBe('READY');
    expect(presence.get('stacker')).toBe('READY');
  });

  it('final state: all active agents moved back to bench', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runHappyPath(bus);
    vi.runAllTimers();

    const { desks } = buildWorldState(signals);
    // All SWOs completed → no active desks
    expect(desks).toHaveLength(0);
  });

  it('mid-scenario: lois and stacker have nested tube at depth 2', () => {
    const bus = new MockBus();
    const signals: RuntimeSignal[] = [];
    bus.subscribe((s) => signals.push(s));

    runHappyPath(bus);
    // Advance timers to just past stacker delegation (t=4200)
    vi.advanceTimersByTime(4500);

    const { tubes } = buildWorldState(signals);
    const loisStackerTube = tubes.find(
      (t) => t.fromAgentId === 'lois' && t.toAgentId === 'stacker',
    );
    expect(loisStackerTube).toBeDefined();
    expect(loisStackerTube!.direction).toBe('down');
  });
});
