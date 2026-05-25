import { computeTubes } from './tubePathComputer';
import type { SwoRecord } from '../types';

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

describe('computeTubes', () => {
  it('no SWOs → no tubes', () => {
    expect(computeTubes([])).toHaveLength(0);
  });

  it('root SWO only (no parent) → no tubes', () => {
    const swos = [makeSwo({ id: 'swo-1', assigneeId: 'perry' })];
    expect(computeTubes(swos)).toHaveLength(0);
  });

  it('child SWO with parent → one tube from parent assignee to child assignee', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root' }),
    ];
    const tubes = computeTubes(swos);

    expect(tubes).toHaveLength(1);
    expect(tubes[0]!.fromAgentId).toBe('perry');
    expect(tubes[0]!.toAgentId).toBe('lois');
    expect(tubes[0]!.id).toBe('tube-swo-child');
  });

  it('status mapping: IN_PROGRESS → active', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', status: 'IN_PROGRESS' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.status).toBe('active');
  });

  it('status mapping: PENDING → active', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', status: 'PENDING' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.status).toBe('active');
  });

  it('status mapping: BLOCKED → blocked', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', status: 'BLOCKED' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.status).toBe('blocked');
  });

  it('status mapping: WAITING_REVIEW → review', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', status: 'WAITING_REVIEW' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.status).toBe('review');
  });

  it('status mapping: COMPLETED → complete', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', status: 'COMPLETED' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.status).toBe('complete');
  });

  it('direction: non-completed → down', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', status: 'IN_PROGRESS' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.direction).toBe('down');
  });

  it('direction: completed → up', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', status: 'COMPLETED' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.direction).toBe('up');
  });

  it('progress correctly mapped from SWO', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-root', progress: 0.75 }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes[0]!.capsuleProgress).toBe(0.75);
  });

  it('multiple child SWOs each produce their own tube', () => {
    const swos = [
      makeSwo({ id: 'swo-root', assigneeId: 'perry' }),
      makeSwo({ id: 'swo-lois', assigneeId: 'lois', parentSwoId: 'swo-root' }),
      makeSwo({ id: 'swo-lex', assigneeId: 'lex', parentSwoId: 'swo-root' }),
    ];
    const tubes = computeTubes(swos);
    expect(tubes).toHaveLength(2);

    const agentPairs = tubes.map((t) => `${t.fromAgentId}->${t.toAgentId}`);
    expect(agentPairs).toContain('perry->lois');
    expect(agentPairs).toContain('perry->lex');
  });

  it('child SWO with missing parent in list → no tube', () => {
    const swos = [
      makeSwo({ id: 'swo-child', assigneeId: 'lois', parentSwoId: 'swo-missing' }),
    ];
    expect(computeTubes(swos)).toHaveLength(0);
  });
});
