import { computeLayout } from './layoutEngine';
import { agents } from '../sim/mockRoster';
import type { AgentPresence, SwoRecord } from '../types';

function makeSwo(overrides: Partial<SwoRecord> & Pick<SwoRecord, 'id' | 'assigneeId'>): SwoRecord {
  return {
    parentSwoId: null,
    title: 'Test task',
    status: 'IN_PROGRESS',
    progress: 0,
    createdAt: 1000,
    updatedAt: 2000,
    ...overrides,
  };
}

const emptyPresence = new Map<string, AgentPresence>();

describe('computeLayout', () => {
  it('given no SWOs, all agents should be on the bench', () => {
    const { desks, bench } = computeLayout(agents, [], emptyPresence);
    expect(desks).toHaveLength(0);
    expect(bench).toHaveLength(agents.length);
  });

  it('given a root SWO assigned to perry, perry should be at row 0, all others on bench', () => {
    const swos = [makeSwo({ id: 'swo-1', assigneeId: 'perry' })];
    const { desks, bench } = computeLayout(agents, swos, emptyPresence);

    expect(desks).toHaveLength(1);
    const perryDesk = desks.find((d) => d.agentId === 'perry');
    expect(perryDesk).toBeDefined();
    expect(perryDesk!.gridRow).toBe(0);

    expect(bench).toHaveLength(agents.length - 1);
    expect(bench.find((d) => d.agentId === 'perry')).toBeUndefined();
  });

  it('given root SWO (perry) + child SWOs (lois, lex), perry at row 0, lois+lex at row 1', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-lois', assigneeId: 'lois', parentSwoId: 'swo-root' }),
      makeSwo({ id: 'swo-lex', assigneeId: 'lex', parentSwoId: 'swo-root' }),
    ];
    const { desks, bench } = computeLayout(agents, swos, emptyPresence);

    expect(desks).toHaveLength(3);

    const perryDesk = desks.find((d) => d.agentId === 'perry');
    const loisDesk = desks.find((d) => d.agentId === 'lois');
    const lexDesk = desks.find((d) => d.agentId === 'lex');

    expect(perryDesk!.gridRow).toBe(0);
    expect(loisDesk!.gridRow).toBe(1);
    expect(lexDesk!.gridRow).toBe(1);

    expect(bench).toHaveLength(agents.length - 3);
  });

  it('given 3-level deep delegation (perry→lois→stacker), correct row assignments', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-lois', assigneeId: 'lois', parentSwoId: 'swo-root' }),
      makeSwo({ id: 'swo-stacker', assigneeId: 'stacker', parentSwoId: 'swo-lois' }),
    ];
    const { desks } = computeLayout(agents, swos, emptyPresence);

    const perryDesk = desks.find((d) => d.agentId === 'perry');
    const loisDesk = desks.find((d) => d.agentId === 'lois');
    const stackerDesk = desks.find((d) => d.agentId === 'stacker');

    expect(perryDesk!.gridRow).toBe(0);
    expect(loisDesk!.gridRow).toBe(1);
    expect(stackerDesk!.gridRow).toBe(2);
  });

  it('progress and currentTask correctly derived from SWO data', () => {
    const swos = [
      makeSwo({ id: 'swo-1', assigneeId: 'perry', title: 'Analyze market', progress: 0.6 }),
    ];
    const { desks } = computeLayout(agents, swos, emptyPresence);

    const perryDesk = desks.find((d) => d.agentId === 'perry');
    expect(perryDesk!.currentTask).toBe('Analyze market');
    expect(perryDesk!.progress).toBe(0.6);
  });

  it('presence map correctly flows to desk presence', () => {
    const swos = [makeSwo({ id: 'swo-1', assigneeId: 'perry' })];
    const presence = new Map<string, AgentPresence>([['perry', 'COMPUTING']]);
    const { desks } = computeLayout(agents, swos, presence);

    const perryDesk = desks.find((d) => d.agentId === 'perry');
    expect(perryDesk!.presence).toBe('COMPUTING');
  });

  it('completed SWOs do not put agents on the active grid', () => {
    const swos = [
      makeSwo({ id: 'swo-1', assigneeId: 'perry', status: 'COMPLETED' }),
    ];
    const { desks, bench } = computeLayout(agents, swos, emptyPresence);
    expect(desks).toHaveLength(0);
    expect(bench).toHaveLength(agents.length);
  });

  it('agents with no presence in map default to IDLE', () => {
    const swos = [makeSwo({ id: 'swo-1', assigneeId: 'perry' })];
    const { desks } = computeLayout(agents, swos, emptyPresence);
    const perryDesk = desks.find((d) => d.agentId === 'perry');
    expect(perryDesk!.presence).toBe('IDLE');
  });
});
