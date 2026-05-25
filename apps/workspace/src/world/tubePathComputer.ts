import type { SwoRecord, TubeState } from '../types';

/**
 * Derive tube connections from SWO parent-child relationships.
 * Each child SWO with a parentSwoId creates a tube from parent assignee → child assignee.
 */
export function computeTubes(swos: SwoRecord[]): TubeState[] {
  const swoById = new Map<string, SwoRecord>();
  for (const swo of swos) {
    swoById.set(swo.id, swo);
  }

  const tubes: TubeState[] = [];
  const seen = new Set<string>();

  for (const swo of swos) {
    if (!swo.parentSwoId) continue;

    const parent = swoById.get(swo.parentSwoId);
    if (!parent) continue;

    // Dedupe: one tube per parent-child agent pair per SWO
    const key = `${parent.assigneeId}->${swo.assigneeId}:${swo.id}`;
    if (seen.has(key)) continue;
    seen.add(key);

    const status = mapStatus(swo.status);
    const direction = swo.status === 'COMPLETED' ? 'up' as const : 'down' as const;

    tubes.push({
      id: `tube-${swo.id}`,
      fromAgentId: parent.assigneeId,
      toAgentId: swo.assigneeId,
      status,
      capsuleProgress: swo.progress,
      direction,
    });
  }

  return tubes;
}

function mapStatus(swoStatus: SwoRecord['status']): TubeState['status'] {
  switch (swoStatus) {
    case 'IN_PROGRESS':
    case 'PENDING':
      return 'active';
    case 'BLOCKED':
      return 'blocked';
    case 'WAITING_REVIEW':
      return 'review';
    case 'COMPLETED':
      return 'complete';
  }
}
