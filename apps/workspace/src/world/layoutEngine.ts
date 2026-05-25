import type { Agent, AgentPresence, DeskState, SwoRecord } from '../types';

/**
 * Compute grid layout for active agents and bench for idle ones.
 * Active = assigned to any non-completed SWO.
 * Tree layout: root at row 0, children at row 1, sub-children at row 2+.
 */
export function computeLayout(
  agents: Agent[],
  swos: SwoRecord[],
  presenceMap: Map<string, AgentPresence>,
  delegatingAgentIds?: Set<string>,
  agentStatusTexts?: Map<string, string>,
): { desks: DeskState[]; bench: DeskState[] } {
  // CHA-429 — defensive dedup by agent id, last-write-wins. Even though
  // upstream state hooks dedup at write time, any caller that constructs
  // an agent array directly (tests, integration harnesses, mock replays)
  // would otherwise render duplicate desks/bench cards. Dave observed 22
  // idle bench cards on a smaller org; this is the defensive shield.
  {
    const byId = new Map<string, Agent>();
    for (const a of agents) {
      if (!a || !a.id) continue;
      byId.set(a.id, a);
    }
    agents = Array.from(byId.values());
  }

  // Find active agent IDs (assigned to non-completed SWOs)
  const activeAgentIds = new Set<string>();
  const agentCurrentTask = new Map<string, { title: string; progress: number }>();

  for (const swo of swos) {
    if (swo.status !== 'COMPLETED') {
      activeAgentIds.add(swo.assigneeId);
      // Use the most recently updated SWO as current task
      const existing = agentCurrentTask.get(swo.assigneeId);
      if (!existing || swo.updatedAt > (existing as { title: string; progress: number }).progress) {
        agentCurrentTask.set(swo.assigneeId, { title: swo.title, progress: swo.progress });
      }
    }
  }

  // Build parent-child SWO tree to determine rows
  const swoById = new Map<string, SwoRecord>();
  for (const swo of swos) {
    swoById.set(swo.id, swo);
  }

  // Compute depth for each SWO (root=0, child=1, etc.)
  function swoDepth(swo: SwoRecord): number {
    let depth = 0;
    let current = swo;
    while (current.parentSwoId) {
      const parent = swoById.get(current.parentSwoId);
      if (!parent) break;
      current = parent;
      depth++;
    }
    return depth;
  }

  // Map agent → row (based on deepest SWO they're assigned to)
  const agentRow = new Map<string, number>();
  for (const swo of swos) {
    if (swo.status === 'COMPLETED') continue;
    const depth = swoDepth(swo);
    const existing = agentRow.get(swo.assigneeId);
    if (existing === undefined || depth > existing) {
      agentRow.set(swo.assigneeId, depth);
    }
  }

  // Group agents by row for column assignment
  const rowGroups = new Map<number, string[]>();
  for (const [agentId, row] of agentRow) {
    const group = rowGroups.get(row) ?? [];
    group.push(agentId);
    rowGroups.set(row, group);
  }

  // Assign columns: center agents in each row
  const agentCol = new Map<string, number>();
  for (const [_row, agentIds] of rowGroups) {
    const count = agentIds.length;
    const startCol = Math.floor((6 - count) / 2); // 6-column grid
    for (let i = 0; i < agentIds.length; i++) {
      agentCol.set(agentIds[i]!, startCol + i);
    }
  }

  const desks: DeskState[] = [];
  const bench: DeskState[] = [];

  for (const agent of agents) {
    const presence = presenceMap.get(agent.id) ?? 'IDLE';
    const task = agentCurrentTask.get(agent.id);

    const desk: DeskState = {
      agentId: agent.id,
      name: agent.name,
      role: agent.role,
      icon: agent.icon,
      presence,
      currentTask: task?.title ?? null,
      statusText: agentStatusTexts?.get(agent.id) ?? null,
      progress: task?.progress ?? 0,
      isDelegating: delegatingAgentIds?.has(agent.id) ?? false,
      gridRow: agentRow.get(agent.id) ?? 0,
      gridCol: agentCol.get(agent.id) ?? 0,
    };

    if (activeAgentIds.has(agent.id)) {
      desks.push(desk);
    } else {
      bench.push(desk);
    }
  }

  return { desks, bench };
}
