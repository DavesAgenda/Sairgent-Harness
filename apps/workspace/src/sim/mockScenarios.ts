import type { Bus, RuntimeSignal } from '../types';

let nextSwoId = 1;
function swoId(): string {
  return `swo-${nextSwoId++}`;
}

function emit(bus: Bus, type: RuntimeSignal['type'], payload: Record<string, unknown>, delay: number): void {
  setTimeout(() => {
    bus.emit({ type, timestamp: Date.now(), payload });
  }, delay);
}

export function resetIdCounter(): void {
  nextSwoId = 1;
}

/**
 * Happy path: Submit → Perry assesses → delegates to Lois + Lex →
 * they process → Lois sub-delegates to Stacker → completions flow back up →
 * artifact produced → inbox item
 */
export function runHappyPath(bus: Bus): void {
  const rootId = swoId();
  const loisId = swoId();
  const lexId = swoId();
  const stackerId = swoId();

  // t=0: Job submitted, root SWO created, assigned to Perry
  emit(bus, 'swo.created', {
    swo: { id: rootId, parentSwoId: null, title: 'Market analysis report', assigneeId: 'perry', status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
  }, 0);

  // t=500: Perry starts assessing
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'COMPUTING' }, 500);
  emit(bus, 'swo.updated', {
    swo: { id: rootId, status: 'IN_PROGRESS', progress: 0.1 },
  }, 500);

  // t=2000: Perry delegates to Lois (research) and Lex (financial analysis)
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'READY' }, 2000);
  emit(bus, 'swo.updated', {
    swo: { id: rootId, status: 'IN_PROGRESS', progress: 0.2 },
  }, 2000);

  emit(bus, 'delegation.started', { fromAgentId: 'perry', toAgentId: 'lois', swoId: loisId }, 2000);
  emit(bus, 'swo.created', {
    swo: { id: loisId, parentSwoId: rootId, title: 'Competitor research', assigneeId: 'lois', status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
  }, 2100);
  emit(bus, 'agent.presence.changed', { agentId: 'lois', presence: 'COMPUTING' }, 2200);
  emit(bus, 'swo.updated', { swo: { id: loisId, status: 'IN_PROGRESS', progress: 0.1 } }, 2200);

  emit(bus, 'delegation.started', { fromAgentId: 'perry', toAgentId: 'lex', swoId: lexId }, 2300);
  emit(bus, 'swo.created', {
    swo: { id: lexId, parentSwoId: rootId, title: 'Financial projections', assigneeId: 'lex', status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
  }, 2400);
  emit(bus, 'agent.presence.changed', { agentId: 'lex', presence: 'COMPUTING' }, 2500);
  emit(bus, 'swo.updated', { swo: { id: lexId, status: 'IN_PROGRESS', progress: 0.1 } }, 2500);

  // t=4000: Lois sub-delegates to Stacker
  emit(bus, 'swo.updated', { swo: { id: loisId, progress: 0.4 } }, 3500);
  emit(bus, 'delegation.started', { fromAgentId: 'lois', toAgentId: 'stacker', swoId: stackerId }, 4000);
  emit(bus, 'swo.created', {
    swo: { id: stackerId, parentSwoId: loisId, title: 'Data synthesis', assigneeId: 'stacker', status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
  }, 4100);
  emit(bus, 'agent.presence.changed', { agentId: 'stacker', presence: 'COMPUTING' }, 4200);
  emit(bus, 'swo.updated', { swo: { id: stackerId, status: 'IN_PROGRESS', progress: 0.1 } }, 4200);

  // t=5000: Lex progresses
  emit(bus, 'swo.updated', { swo: { id: lexId, progress: 0.5 } }, 5000);

  // t=6000: Stacker completes
  emit(bus, 'swo.updated', { swo: { id: stackerId, progress: 1 } }, 6000);
  emit(bus, 'swo.completed', { swo: { id: stackerId, status: 'COMPLETED', progress: 1 } }, 6200);
  emit(bus, 'agent.presence.changed', { agentId: 'stacker', presence: 'READY' }, 6200);
  emit(bus, 'delegation.completed', { fromAgentId: 'lois', toAgentId: 'stacker', swoId: stackerId }, 6300);

  // t=7000: Lex completes
  emit(bus, 'swo.updated', { swo: { id: lexId, progress: 1 } }, 7000);
  emit(bus, 'swo.completed', { swo: { id: lexId, status: 'COMPLETED', progress: 1 } }, 7200);
  emit(bus, 'agent.presence.changed', { agentId: 'lex', presence: 'READY' }, 7200);
  emit(bus, 'delegation.completed', { fromAgentId: 'perry', toAgentId: 'lex', swoId: lexId }, 7300);

  // t=7500: Lois completes (after Stacker result)
  emit(bus, 'swo.updated', { swo: { id: loisId, progress: 1 } }, 7500);
  emit(bus, 'swo.completed', { swo: { id: loisId, status: 'COMPLETED', progress: 1 } }, 7700);
  emit(bus, 'agent.presence.changed', { agentId: 'lois', presence: 'READY' }, 7700);
  emit(bus, 'delegation.completed', { fromAgentId: 'perry', toAgentId: 'lois', swoId: loisId }, 7800);

  // t=8500: Perry synthesizes and completes root
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'COMPUTING' }, 8500);
  emit(bus, 'swo.updated', { swo: { id: rootId, progress: 0.8 } }, 8500);

  // t=9500: Artifact produced
  emit(bus, 'artifact.produced', {
    swoId: rootId,
    agentId: 'perry',
    artifact: { title: 'Market Analysis Report', content: '# Market Analysis Report\n\n## Executive Summary\nBased on comprehensive competitor research (Lois) and financial projections (Lex), with data synthesis by Stacker.\n\n## Key Findings\n- Market growing at 23% CAGR\n- Three viable entry points identified\n- Recommended pricing: $49/mo starter, $199/mo pro\n\n## Competitor Landscape\n- Competitor A: Strong brand, weak on automation\n- Competitor B: Good pricing, poor UX\n- Competitor C: Enterprise focus, not SMB\n\n## Financial Projections\n- Break-even at Month 8\n- Year 1 ARR target: $480K\n- CAC payback: 4.2 months' },
  }, 9500);

  // t=10000: Root completes, inbox item
  emit(bus, 'swo.updated', { swo: { id: rootId, progress: 1 } }, 10000);
  emit(bus, 'swo.completed', { swo: { id: rootId, status: 'COMPLETED', progress: 1 } }, 10000);
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'READY' }, 10000);

  emit(bus, 'inbox.item.added', {
    item: { id: `inbox-${rootId}`, swoId: rootId, agentName: 'Perry', title: 'Market Analysis Report', content: '# Market Analysis Report\n\n## Executive Summary\nBased on comprehensive competitor research (Lois) and financial projections (Lex), with data synthesis by Stacker.\n\n## Key Findings\n- Market growing at 23% CAGR\n- Three viable entry points identified\n- Recommended pricing: $49/mo starter, $199/mo pro\n\n## Competitor Landscape\n- Competitor A: Strong brand, weak on automation\n- Competitor B: Good pricing, poor UX\n- Competitor C: Enterprise focus, not SMB\n\n## Financial Projections\n- Break-even at Month 8\n- Year 1 ARR target: $480K\n- CAC payback: 4.2 months', timestamp: Date.now() },
  }, 10200);
}

/**
 * Blocked path: Submit → delegate to Felicity → blocked → escalate → resolved → complete
 */
export function runBlockedPath(bus: Bus): void {
  const rootId = swoId();
  const felicityId = swoId();

  emit(bus, 'swo.created', {
    swo: { id: rootId, parentSwoId: null, title: 'Security audit', assigneeId: 'perry', status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
  }, 0);

  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'COMPUTING' }, 500);
  emit(bus, 'swo.updated', { swo: { id: rootId, status: 'IN_PROGRESS', progress: 0.1 } }, 500);

  // Delegate to Felicity
  emit(bus, 'delegation.started', { fromAgentId: 'perry', toAgentId: 'felicity', swoId: felicityId }, 1500);
  emit(bus, 'swo.created', {
    swo: { id: felicityId, parentSwoId: rootId, title: 'Code security review', assigneeId: 'felicity', status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
  }, 1600);
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'READY' }, 1600);
  emit(bus, 'agent.presence.changed', { agentId: 'felicity', presence: 'COMPUTING' }, 1700);
  emit(bus, 'swo.updated', { swo: { id: felicityId, status: 'IN_PROGRESS', progress: 0.2 } }, 1700);

  // Felicity hits a blocker
  emit(bus, 'swo.updated', { swo: { id: felicityId, status: 'BLOCKED', progress: 0.3 } }, 3500);

  // Escalation — Perry re-engages
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'COMPUTING' }, 4500);
  emit(bus, 'swo.updated', { swo: { id: rootId, progress: 0.4 } }, 4500);

  // Resolved — Felicity unblocked
  emit(bus, 'swo.updated', { swo: { id: felicityId, status: 'IN_PROGRESS', progress: 0.5 } }, 6000);
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'READY' }, 6000);

  // Felicity completes
  emit(bus, 'swo.updated', { swo: { id: felicityId, progress: 1 } }, 8000);
  emit(bus, 'swo.completed', { swo: { id: felicityId, status: 'COMPLETED', progress: 1 } }, 8200);
  emit(bus, 'agent.presence.changed', { agentId: 'felicity', presence: 'READY' }, 8200);
  emit(bus, 'delegation.completed', { fromAgentId: 'perry', toAgentId: 'felicity', swoId: felicityId }, 8300);

  // Root completes
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'COMPUTING' }, 8500);
  emit(bus, 'swo.updated', { swo: { id: rootId, progress: 1 } }, 9500);
  emit(bus, 'swo.completed', { swo: { id: rootId, status: 'COMPLETED', progress: 1 } }, 9500);
  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'READY' }, 9500);

  emit(bus, 'inbox.item.added', {
    item: { id: `inbox-${rootId}`, swoId: rootId, agentName: 'Perry', title: 'Security Audit Complete', content: '# Security Audit\n\nCode review completed by Felicity. Initial blocker on dependency vulnerability resolved.\n\n## Findings\n- 2 critical vulnerabilities patched\n- Auth flow hardened\n- All endpoints validated', timestamp: Date.now() },
  }, 9700);
}

/**
 * Parallel burst: 3 jobs submitted 500ms apart
 */
export function runParallelBurst(bus: Bus): void {
  const jobs = [
    { title: 'Update docs', assignee: 'clark', delay: 0 },
    { title: 'Review pricing', assignee: 'lex', delay: 500 },
    { title: 'Brand audit', assignee: 'cat', delay: 1000 },
  ];

  for (const job of jobs) {
    const rootId = swoId();
    const childId = swoId();

    emit(bus, 'swo.created', {
      swo: { id: rootId, parentSwoId: null, title: job.title, assigneeId: 'perry', status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
    }, job.delay);

    emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'COMPUTING' }, job.delay + 300);
    emit(bus, 'swo.updated', { swo: { id: rootId, status: 'IN_PROGRESS', progress: 0.1 } }, job.delay + 300);

    emit(bus, 'delegation.started', { fromAgentId: 'perry', toAgentId: job.assignee, swoId: childId }, job.delay + 1000);
    emit(bus, 'swo.created', {
      swo: { id: childId, parentSwoId: rootId, title: job.title, assigneeId: job.assignee, status: 'PENDING', progress: 0, createdAt: Date.now(), updatedAt: Date.now() },
    }, job.delay + 1100);
    emit(bus, 'agent.presence.changed', { agentId: job.assignee, presence: 'COMPUTING' }, job.delay + 1200);
    emit(bus, 'swo.updated', { swo: { id: childId, status: 'IN_PROGRESS', progress: 0.1 } }, job.delay + 1200);

    // Progress
    emit(bus, 'swo.updated', { swo: { id: childId, progress: 0.5 } }, job.delay + 3000);
    emit(bus, 'swo.updated', { swo: { id: childId, progress: 1 } }, job.delay + 5000);
    emit(bus, 'swo.completed', { swo: { id: childId, status: 'COMPLETED', progress: 1 } }, job.delay + 5200);
    emit(bus, 'agent.presence.changed', { agentId: job.assignee, presence: 'READY' }, job.delay + 5200);
    emit(bus, 'delegation.completed', { fromAgentId: 'perry', toAgentId: job.assignee, swoId: childId }, job.delay + 5300);

    emit(bus, 'swo.updated', { swo: { id: rootId, progress: 1 } }, job.delay + 5500);
    emit(bus, 'swo.completed', { swo: { id: rootId, status: 'COMPLETED', progress: 1 } }, job.delay + 5500);
  }

  emit(bus, 'agent.presence.changed', { agentId: 'perry', presence: 'READY' }, 7000);
}
