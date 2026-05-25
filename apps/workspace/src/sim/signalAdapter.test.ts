import { describe, it, expect } from 'vitest';
import {
  adaptSignal,
  adaptSwo,
  adaptBootstrap,
  flattenRoster,
  type KernelSignal,
  type KernelSwoRecord,
  type KernelAgentTreeNode,
  type KernelBootstrap,
  type KernelEnvelope,
  type KernelOutboxArtifact,
} from './signalAdapter';
import type { SwoRecord } from '../types';

function makeEnvelope(overrides?: Partial<KernelEnvelope>): KernelEnvelope {
  return {
    id: 'sig-1',
    correlationId: 'corr-1',
    source: 'test',
    occurredAt: 1000,
    cursor: 'cursor-1',
    ...overrides,
  };
}

function makeKernelSignal(kind: string, payload: Record<string, unknown>): KernelSignal {
  return { envelope: makeEnvelope(), kind, payload };
}

function makeKernelSwo(overrides?: Partial<KernelSwoRecord>): KernelSwoRecord {
  return {
    id: 1,
    assignee: 'perry',
    owner: 'operator',
    status: 'IN_PROGRESS',
    workOrderTitle: 'Test task',
    workOrderOutcome: null,
    payload: '{}',
    createdAt: '2026-04-04T00:00:00Z',
    retryCount: 0,
    actualChildAssignees: [],
    childSwoCount: 0,
    parentSwoId: null,
    projectId: null,
    priorityClass: null,
    ...overrides,
  };
}

function makeAgentNode(overrides?: Partial<KernelAgentTreeNode>): KernelAgentTreeNode {
  return {
    id: 'perry',
    name: 'Perry',
    role: 'COO',
    depth: 0,
    presence: 'READY',
    children: [],
    orgProfile: {
      orgClass: 'Manager',
      title: 'Chief Operating Officer',
      skills: ['delegation'],
      tools: ['linear'],
    },
    ...overrides,
  };
}

describe('adaptSwo', () => {
  it('maps kernel SWO to workspace SWO', () => {
    const k = makeKernelSwo();
    const result = adaptSwo(k);
    expect(result.id).toBe('1');
    expect(result.assigneeId).toBe('perry');
    expect(result.title).toBe('Test task');
    expect(result.status).toBe('IN_PROGRESS');
    expect(result.parentSwoId).toBeNull();
  });

  it('maps parent SWO ID as string', () => {
    const k = makeKernelSwo({ parentSwoId: 42 });
    expect(adaptSwo(k).parentSwoId).toBe('42');
  });

  it('maps terminal statuses to COMPLETED with progress 1', () => {
    for (const status of ['COMPLETED', 'FAILED', 'CANCELLED']) {
      const k = makeKernelSwo({ status });
      const result = adaptSwo(k);
      expect(result.status).toBe('COMPLETED');
      expect(result.progress).toBe(1);
    }
  });

  it('falls back to "Untitled task" when title is null', () => {
    const k = makeKernelSwo({ workOrderTitle: null });
    expect(adaptSwo(k).title).toBe('Untitled task');
  });
});

describe('flattenRoster', () => {
  it('flattens a single root node', () => {
    const roster = [makeAgentNode()];
    const agents = flattenRoster(roster);
    expect(agents).toHaveLength(1);
    expect(agents[0]!.id).toBe('perry');
    expect(agents[0]!.parentId).toBeNull();
  });

  it('flattens nested children with correct parentId', () => {
    const roster = [
      makeAgentNode({
        children: [
          makeAgentNode({
            id: 'felicity',
            name: 'Felicity',
            role: 'CTO',
            children: [
              makeAgentNode({ id: 'jimmy', name: 'Jimmy', role: 'CDO' }),
            ],
          }),
        ],
      }),
    ];
    const agents = flattenRoster(roster);
    expect(agents).toHaveLength(3);
    expect(agents.find((a) => a.id === 'felicity')!.parentId).toBe('perry');
    expect(agents.find((a) => a.id === 'jimmy')!.parentId).toBe('felicity');
  });
});

describe('adaptSignal', () => {
  const emptySwoMap = new Map<string, SwoRecord>();

  it('swo.upserted with new SWO emits swo.created', () => {
    const signal = makeKernelSignal('swo.upserted', {
      swo: makeKernelSwo({ id: 1, status: 'PENDING' }),
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(1);
    expect(results[0]!.type).toBe('swo.created');
  });

  it('swo.upserted with existing SWO emits swo.updated', () => {
    const existingSwos = new Map<string, SwoRecord>([
      ['1', { id: '1', parentSwoId: null, title: 'Old', assigneeId: 'perry', status: 'PENDING', progress: 0, createdAt: 0, updatedAt: 0 }],
    ]);
    const signal = makeKernelSignal('swo.upserted', {
      swo: makeKernelSwo({ id: 1, status: 'IN_PROGRESS' }),
    });
    const results = adaptSignal(signal, existingSwos);
    expect(results.length).toBeGreaterThanOrEqual(1);
    expect(results[0]!.type).toBe('swo.updated');
  });

  it('swo.upserted with terminal status on existing SWO emits swo.updated + swo.completed', () => {
    const existingSwos = new Map<string, SwoRecord>([
      ['1', { id: '1', parentSwoId: null, title: 'Old', assigneeId: 'perry', status: 'IN_PROGRESS', progress: 0.5, createdAt: 0, updatedAt: 0 }],
    ]);
    const signal = makeKernelSignal('swo.upserted', {
      swo: makeKernelSwo({ id: 1, status: 'COMPLETED' }),
    });
    const results = adaptSignal(signal, existingSwos);
    expect(results).toHaveLength(2);
    expect(results[0]!.type).toBe('swo.updated');
    expect(results[1]!.type).toBe('swo.completed');
  });

  it('agent.presence.changed maps directly', () => {
    const signal = makeKernelSignal('agent.presence.changed', {
      agentId: 'perry',
      presence: 'COMPUTING',
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(1);
    expect(results[0]!.type).toBe('agent.presence.changed');
    expect(results[0]!.payload.presence).toBe('COMPUTING');
  });

  it('agent.presence.changed maps STALE to IDLE', () => {
    const signal = makeKernelSignal('agent.presence.changed', {
      agentId: 'perry',
      presence: 'STALE',
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results[0]!.payload.presence).toBe('IDLE');
  });

  it('inbox.item.upserted maps to inbox.item.added', () => {
    const signal = makeKernelSignal('inbox.item.upserted', {
      item: { id: 'inbox-1', swoId: 1, agentId: 'perry', title: 'Review needed', summary: 'Please review' },
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(1);
    expect(results[0]!.type).toBe('inbox.item.added');
  });

  it('artifact.created maps to artifact.produced', () => {
    const signal = makeKernelSignal('artifact.created', {
      swoId: 1,
      agent: 'perry',
      filename: 'report.md',
      content: '# Report',
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(1);
    expect(results[0]!.type).toBe('artifact.produced');
  });

  it('delegation.decision.recorded with delegate emits delegation.started', () => {
    const signal = makeKernelSignal('delegation.decision.recorded', {
      decision: 'delegate',
      fromAgentId: 'perry',
      toAgentId: 'felicity',
      swoId: '1',
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(1);
    expect(results[0]!.type).toBe('delegation.started');
  });

  it('delegation.decision.recorded with toAgentIds array emits delegation.started per agent', () => {
    const signal = makeKernelSignal('delegation.decision.recorded', {
      parentSwoId: 10,
      childSwoIds: [11, 12],
      toAgentIds: ['felicity', 'jimmy'],
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(2);
    expect(results[0]!.type).toBe('delegation.started');
    expect(results[0]!.payload.toAgentId).toBe('felicity');
    expect(results[1]!.payload.toAgentId).toBe('jimmy');
    expect(results[0]!.payload.swoId).toBe('10');
  });

  it('delegation.decision.recorded with completed emits delegation.completed', () => {
    const signal = makeKernelSignal('delegation.decision.recorded', {
      decision: 'completed',
      fromAgentId: 'perry',
      toAgentId: 'felicity',
      swoId: '1',
    });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(1);
    expect(results[0]!.type).toBe('delegation.completed');
  });

  it('inbox.item.resolved returns empty array', () => {
    const signal = makeKernelSignal('inbox.item.resolved', { itemId: 'inbox-1' });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(0);
  });

  it('unknown signal kind returns empty array', () => {
    const signal = makeKernelSignal('some.future.signal.kind', { data: 'whatever' });
    const results = adaptSignal(signal, emptySwoMap);
    expect(results).toHaveLength(0);
  });
});

describe('adaptBootstrap', () => {
  it('converts roster and queue to agents, swos, and signals', () => {
    const bootstrap: KernelBootstrap = {
      cursor: { value: 'cursor-abc' },
      roster: [
        makeAgentNode({
          children: [makeAgentNode({ id: 'felicity', name: 'Felicity', role: 'CTO' })],
        }),
      ],
      queue: [makeKernelSwo({ id: 1 }), makeKernelSwo({ id: 2, parentSwoId: 1, assignee: 'felicity' })],
    };

    const result = adaptBootstrap(bootstrap);

    expect(result.agents).toHaveLength(2);
    expect(result.agents.map((a) => a.id)).toEqual(['perry', 'felicity']);

    expect(result.swos).toHaveLength(2);
    expect(result.swos[0]!.id).toBe('1');
    expect(result.swos[1]!.parentSwoId).toBe('1');

    // Should have presence signals for each agent + swo.created signals
    expect(result.signals.length).toBeGreaterThanOrEqual(4);
  });

  it('emits artifact.produced signals from recentArtifacts', () => {
    const artifacts: KernelOutboxArtifact[] = [
      {
        id: 1,
        agent: 'Felicity',
        agentId: 'felicity',
        swoId: 10,
        parentSwoId: null,
        sourceWorkOrderTitle: 'Build report',
        absolutePath: '/home/agent/artifacts/report.md',
        filename: 'report.md',
        createdAt: '2026-04-06T12:00:00Z',
      },
      {
        id: 2,
        agent: 'Lex',
        agentId: 'lex',
        swoId: 11,
        parentSwoId: null,
        sourceWorkOrderTitle: 'Financial analysis',
        absolutePath: '/home/agent/artifacts/analysis.md',
        filename: 'analysis.md',
        createdAt: '2026-04-06T12:01:00Z',
      },
    ];

    const bootstrap: KernelBootstrap = {
      cursor: { value: 'cursor-abc' },
      roster: [makeAgentNode()],
      queue: [],
      recentArtifacts: artifacts,
    };

    const result = adaptBootstrap(bootstrap);

    const artifactSignals = result.signals.filter((s) => s.type === 'artifact.produced');
    expect(artifactSignals).toHaveLength(2);
    expect(artifactSignals[0]!.payload.agentId).toBe('felicity');
    expect((artifactSignals[0]!.payload.artifact as { title: string }).title).toBe('report.md');
    expect(artifactSignals[1]!.payload.agentId).toBe('lex');
  });

  it('handles missing recentArtifacts gracefully', () => {
    const bootstrap: KernelBootstrap = {
      cursor: { value: 'cursor-abc' },
      roster: [makeAgentNode()],
      queue: [],
      // No recentArtifacts field
    };

    const result = adaptBootstrap(bootstrap);
    const artifactSignals = result.signals.filter((s) => s.type === 'artifact.produced');
    expect(artifactSignals).toHaveLength(0);
  });
});
