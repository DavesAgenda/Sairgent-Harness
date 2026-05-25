/**
 * Pure adapter: maps real kernel RuntimeSignals (26 kinds, envelope-wrapped)
 * into workspace RuntimeSignals (8 flat types).
 *
 * One kernel signal may produce 0-N workspace signals.
 * Unknown signal kinds are logged and dropped — never crash.
 */
import type { Agent, InboxItem, RuntimeSignal, SwoRecord } from '../types';

// --- Kernel types (subset of @sairgent/chat-core) ---
// We define the shapes inline to avoid a hard dependency on chat-core
// in the standalone workspace build. When running inside Tauri, the
// real types flow through the IPC bridge.

export interface KernelEnvelope {
  id: string;
  correlationId: string;
  source: string;
  occurredAt: number;
  cursor: string;
}

export interface KernelSignal {
  envelope: KernelEnvelope;
  kind: string;
  payload: Record<string, unknown>;
}

export interface KernelSwoRecord {
  id: number;
  assignee: string;
  owner: string;
  status: string;
  workOrderTitle?: string | null;
  workOrderOutcome?: string | null;
  payload: string;
  createdAt: string;
  retryCount: number;
  actualChildAssignees: string[];
  childSwoCount: number;
  parentSwoId?: number | null;
  projectId?: string | null;
  priorityClass?: string | null;
  reviewResponse?: string | null;
  revisionFeedback?: string | null;
}

export interface KernelAgentTreeNode {
  id: string;
  name: string;
  role: string;
  depth: number;
  presence: string;
  children: KernelAgentTreeNode[];
  orgProfile: {
    orgClass: string;
    title: string;
    skills?: string[];
    tools?: string[];
  };
  /** Optional fields — available when kernel bootstrap includes agent details */
  defaultProvider?: string;
  model?: string;
  raisonDetre?: string;
  personaPrompt?: string;
}

export interface KernelOutboxArtifact {
  id: number;
  agent: string;
  agentId?: string | null;
  swoId?: number | null;
  parentSwoId?: number | null;
  sourceWorkOrderTitle?: string | null;
  absolutePath: string;
  filename: string;
  createdAt: string;
}

export interface KernelAttentionSummary {
  openInboxItems: number;
  openApprovalItems: number;
  openDeliverableItems: number;
  openBlockedItems: number;
}

export interface KernelInboxItem {
  id: string;
  kind: string;
  status: string;
  priority: string;
  title: string;
  summary: string;
  createdAt: string;
  updatedAt: string;
  projectId?: string | null;
  projectName?: string | null;
  swoId?: number | null;
  artifactId?: number | null;
  agentId?: string | null;
}

export interface KernelBootstrap {
  cursor: { value: string };
  queue: KernelSwoRecord[];
  roster: KernelAgentTreeNode[];
  recentArtifacts?: KernelOutboxArtifact[];
  attentionSummary?: KernelAttentionSummary;
  inboxItems?: KernelInboxItem[];
}

// --- Presence mapping ---

const PRESENCE_MAP: Record<string, RuntimeSignal['payload']['presence'] & string> = {
  READY: 'READY',
  IDLE: 'IDLE',
  COMPUTING: 'COMPUTING',
  STALE: 'IDLE', // Workspace treats STALE as IDLE
  OFFLINE: 'OFFLINE',
};

function mapPresence(kernelPresence: string): string {
  return PRESENCE_MAP[kernelPresence] ?? 'IDLE';
}

/** For bootstrap: treat OFFLINE as IDLE since agents haven't had a chance to heartbeat yet */
function mapBootstrapPresence(kernelPresence: string): string {
  if (kernelPresence === 'OFFLINE') return 'IDLE';
  return mapPresence(kernelPresence);
}

// --- SWO status mapping ---

function mapSwoStatus(kernelStatus: string): SwoRecord['status'] {
  switch (kernelStatus) {
    case 'PENDING':
      return 'PENDING';
    case 'IN_PROGRESS':
      return 'IN_PROGRESS';
    case 'BLOCKED':
      return 'BLOCKED';
    case 'WAITING_REVIEW':
      return 'WAITING_REVIEW';
    case 'COMPLETED':
    case 'FAILED':
    case 'CANCELLED':
      return 'COMPLETED';
    default:
      return 'PENDING';
  }
}

function isTerminal(status: string): boolean {
  return status === 'COMPLETED' || status === 'FAILED' || status === 'CANCELLED';
}

// --- Agent flattening ---

const ROLE_ICONS: Record<string, string> = {
  perry: '\u2318',
  felicity: '\u25CE',
  jimmy: '\u25EB',
  lois: '\u25C8',
  lex: '\u25C6',
  cat: '\u25C7',
  lucy: '\u25CB',
  raymond: '\u25CD',
  oracle: '\u25C9',
  red: '\u25D0',
  clark: '\u25D1',
  stacker: '\u25D2',
  vicki: '\u25D3',
  chloe: '\u25D4',
  mercy: '\u25D5',
};

export function flattenRosterNode(
  node: KernelAgentTreeNode,
  parentId: string | null = null,
): Agent[] {
  const agent: Agent = {
    id: node.id,
    name: node.name,
    role: node.role,
    title: node.orgProfile.title,
    parentId,
    skills: node.orgProfile.skills ?? [],
    tools: node.orgProfile.tools ?? [],
    icon: ROLE_ICONS[node.id] ?? '\u25CF',
    orgClass: node.orgProfile.orgClass,
    provider: node.defaultProvider,
    model: node.model,
    raisonDetre: node.raisonDetre,
    personaPrompt: node.personaPrompt,
  };

  const children = node.children.flatMap((child) =>
    flattenRosterNode(child, node.id),
  );

  return [agent, ...children];
}

export function flattenRoster(roster: KernelAgentTreeNode[]): Agent[] {
  const flat = roster.flatMap((node) => flattenRosterNode(node, null));
  // CHA-429 — dedup by id. If the kernel tree ever emits the same agent
  // under two parents (or re-seeds a dynamically-hired agent that also
  // appears in the bootstrap tree), the workspace would render both copies
  // on the idle bench. Last-write-wins so any updated field (persona,
  // parent) from the deeper entry takes effect.
  const byId = new Map<string, Agent>();
  for (const agent of flat) {
    byId.set(agent.id, agent);
  }
  return Array.from(byId.values());
}

// --- Agent name → ID resolution ---

/** Map of agent display names (lowercase) to agent IDs. Built from roster. */
export type AgentNameMap = Map<string, string>;

export function buildAgentNameMap(roster: KernelAgentTreeNode[]): AgentNameMap {
  const map = new Map<string, string>();
  function walk(nodes: KernelAgentTreeNode[]) {
    for (const node of nodes) {
      map.set(node.name.toLowerCase(), node.id);
      // Also map by id→id for cases where assignee is already an ID
      map.set(node.id.toLowerCase(), node.id);
      walk(node.children);
    }
  }
  walk(roster);
  return map;
}

function resolveAgentId(nameOrId: string, nameMap: AgentNameMap): string {
  const lower = nameOrId.toLowerCase();
  // Direct match (name or ID)
  if (nameMap.has(lower)) return nameMap.get(lower)!;
  // Handle "Name (Role)" format from kernel SWO views
  const withoutRole = lower.replace(/\s*\(.*?\)\s*$/, '');
  if (nameMap.has(withoutRole)) return nameMap.get(withoutRole)!;
  return nameOrId;
}

// --- SWO conversion ---

export function adaptSwo(k: KernelSwoRecord, nameMap?: AgentNameMap): SwoRecord {
  const assigneeId = nameMap
    ? resolveAgentId(k.assignee, nameMap)
    : k.assignee;
  return {
    id: String(k.id),
    parentSwoId: k.parentSwoId != null ? String(k.parentSwoId) : null,
    title: k.workOrderTitle ?? 'Untitled task',
    assigneeId,
    status: mapSwoStatus(k.status),
    progress: isTerminal(k.status) ? 1 : 0,
    createdAt: new Date(k.createdAt).getTime(),
    updatedAt: Date.now(),
    reviewResponse: k.reviewResponse ?? null,
    outcome: k.workOrderOutcome ?? null,
    payload: k.payload ?? null,
    revisionFeedback: k.revisionFeedback ?? null,
  };
}

// --- Bootstrap conversion ---

export function adaptBootstrap(bootstrap: KernelBootstrap): {
  agents: Agent[];
  swos: SwoRecord[];
  signals: RuntimeSignal[];
  nameMap: AgentNameMap;
} {
  const agents = flattenRoster(bootstrap.roster);
  const nameMap = buildAgentNameMap(bootstrap.roster);
  const swos = bootstrap.queue.map((k) => adaptSwo(k, nameMap));

  // Emit presence signals for all agents based on roster state
  const signals: RuntimeSignal[] = [];
  for (const node of bootstrap.roster) {
    for (const flat of flattenRosterNode(node, null)) {
      const rosterNode = findNode(bootstrap.roster, flat.id);
      if (rosterNode) {
        signals.push({
          type: 'agent.presence.changed',
          timestamp: Date.now(),
          payload: { agentId: flat.id, presence: mapBootstrapPresence(rosterNode.presence) },
        });
      }
    }
  }

  // Emit SWO signals for active work
  for (const swo of swos) {
    signals.push({
      type: 'swo.created',
      timestamp: Date.now(),
      payload: { swo },
    });
  }

  // Emit artifact signals from bootstrap data
  if (bootstrap.recentArtifacts) {
    for (const artifact of bootstrap.recentArtifacts) {
      signals.push({
        type: 'artifact.produced',
        timestamp: new Date(artifact.createdAt).getTime(),
        payload: {
          swoId: artifact.swoId != null ? String(artifact.swoId) : '',
          agentId: artifact.agentId ?? artifact.agent,
          artifact: {
            id: artifact.id,
            title: artifact.filename,
            path: artifact.absolutePath,
          },
        },
      });
    }
  }

  // Emit inbox item signals from bootstrap data
  if (bootstrap.inboxItems) {
    for (const item of bootstrap.inboxItems) {
      const agentId = item.agentId ?? '';
      signals.push({
        type: 'inbox.item.added',
        timestamp: new Date(item.updatedAt).getTime(),
        payload: {
          item: {
            id: item.id,
            swoId: item.swoId != null ? String(item.swoId) : '',
            agentName: nameMap?.get(agentId.toLowerCase()) ?? agentId,
            title: item.title,
            content: item.summary,
            timestamp: new Date(item.updatedAt).getTime(),
          },
        },
      });
    }
  }

  return { agents, swos, signals, nameMap };
}

function findNode(
  nodes: KernelAgentTreeNode[],
  id: string,
): KernelAgentTreeNode | undefined {
  for (const node of nodes) {
    if (node.id === id) return node;
    const found = findNode(node.children, id);
    if (found) return found;
  }
  return undefined;
}

// --- Live signal adaptation ---

/** Adapts a single kernel signal into 0-N workspace signals. */
export function adaptSignal(
  signal: KernelSignal,
  existingSwos: Map<string, SwoRecord>,
  nameMap?: AgentNameMap,
): RuntimeSignal[] {
  const now = signal.envelope.occurredAt || Date.now();
  const { kind, payload } = signal;

  switch (kind) {
    case 'swo.upserted': {
      const k = payload.swo as KernelSwoRecord | undefined;
      if (!k) return [];

      const id = String(k.id);
      const existing = existingSwos.get(id);
      const adapted = adaptSwo(k, nameMap);

      if (!existing) {
        return [{ type: 'swo.created', timestamp: now, payload: { swo: adapted } }];
      }

      const results: RuntimeSignal[] = [
        { type: 'swo.updated', timestamp: now, payload: { swo: { ...adapted } } },
      ];

      if (isTerminal(k.status) && existing.status !== 'COMPLETED') {
        results.push({
          type: 'swo.completed',
          timestamp: now,
          payload: { swo: { id, status: 'COMPLETED', progress: 1 } },
        });
      }

      return results;
    }

    case 'agent.presence.changed': {
      const p = payload as { agentId?: string; presence?: string };
      if (!p.agentId || !p.presence) return [];
      return [{
        type: 'agent.presence.changed',
        timestamp: now,
        payload: { agentId: p.agentId, presence: mapPresence(p.presence) },
      }];
    }

    case 'agent.upserted': {
      const a = payload.agent as { id?: string; name?: string; role?: string; parentId?: string | null; reason?: string; presence?: string } | undefined;
      if (!a?.id || !a?.name) return [];
      // Add the new agent to the live name map so subsequent activity log entries
      // can resolve its UUID immediately, not just after a roster reload.
      if (nameMap) nameMap.set(a.id, a.name);
      return [{
        type: 'agent.upserted',
        timestamp: now,
        payload: {
          id: a.id,
          name: a.name,
          role: a.role ?? 'Specialist',
          parentId: a.parentId ?? null,
          reason: a.reason ?? '',
          presence: mapPresence(a.presence ?? 'IDLE'),
        },
      }];
    }

    case 'inbox.item.upserted': {
      const item = payload.item as Record<string, unknown> | undefined;
      if (!item) return [];
      const agentId = String(item.agentId ?? '');
      const adapted: InboxItem = {
        id: String(item.id ?? ''),
        swoId: String(item.swoId ?? ''),
        agentName: nameMap?.get(agentId) ?? agentId,
        title: String(item.title ?? ''),
        content: String(item.summary ?? ''),
        timestamp: now,
      };
      return [{ type: 'inbox.item.added', timestamp: now, payload: { item: adapted } }];
    }

    case 'artifact.created': {
      // The workspace backend emits { artifact: OutboxArtifactView } as the payload
      const raw = payload as Record<string, unknown>;
      // Support both flat and nested { artifact: ... } shapes
      const a = (raw.artifact && typeof raw.artifact === 'object'
        ? raw.artifact
        : raw) as Record<string, unknown>;
      const artifactAgent = String(a.agentId ?? a.agentId ?? a.agent ?? '');
      const resolvedAgentId = nameMap
        ? resolveAgentId(artifactAgent, nameMap)
        : artifactAgent;
      return [{
        type: 'artifact.produced',
        timestamp: now,
        payload: {
          swoId: String(a.swoId ?? ''),
          agentId: resolvedAgentId,
          artifact: {
            id: typeof a.id === 'number' ? a.id : undefined,
            title: String(a.filename ?? 'Artifact'),
            path: typeof a.absolutePath === 'string' ? a.absolutePath : undefined,
            content: String(a.content ?? ''),
          },
        },
      }];
    }

    case 'delegation.decision.recorded': {
      const d = payload as Record<string, unknown>;

      // Shape A: enriched KernelEvent::DelegationStarted (parentSwoId + toAgentIds[])
      const toAgentIds = d.toAgentIds as string[] | undefined;
      if (Array.isArray(toAgentIds) && toAgentIds.length > 0) {
        const parentSwoId = String(d.parentSwoId ?? '');
        // Resolve the delegating (parent) agent from the parent SWO
        const parentSwo = existingSwos.get(parentSwoId);
        const fromAgentId = parentSwo?.assigneeId ?? '';
        // Emit one delegation.started per target agent
        return toAgentIds.map((toId) => ({
          type: 'delegation.started' as const,
          timestamp: now,
          payload: {
            fromAgentId,
            toAgentId: toId,
            swoId: parentSwoId,
          },
        }));
      }

      // Shape B: legacy decision-based format (fromAgentId/toAgentId/decision)
      const decision = d.decision as string | undefined;
      if (decision === 'delegate' || decision === 'accepted') {
        return [{
          type: 'delegation.started',
          timestamp: now,
          payload: {
            fromAgentId: String(d.fromAgentId ?? d.delegator ?? ''),
            toAgentId: String(d.toAgentId ?? d.delegatee ?? ''),
            swoId: String(d.swoId ?? ''),
          },
        }];
      }
      if (decision === 'completed' || decision === 'returned') {
        return [{
          type: 'delegation.completed',
          timestamp: now,
          payload: {
            fromAgentId: String(d.fromAgentId ?? d.delegator ?? ''),
            toAgentId: String(d.toAgentId ?? d.delegatee ?? ''),
            swoId: String(d.swoId ?? ''),
          },
        }];
      }
      return [];
    }

    case 'agent.activity.delta': {
      const p = payload as { agentId?: string; delta?: string; isFinal?: boolean };
      if (!p.agentId) return [];
      return [{
        type: 'agent.activity.delta',
        timestamp: now,
        payload: {
          agentId: p.agentId,
          delta: p.delta ?? '',
          isFinal: p.isFinal ?? false,
        },
      }];
    }

    case 'inbox.item.resolved':
      // No workspace equivalent needed — UI will update via SWO state
      return [];

    case 'runtime.status.changed': {
      const s = payload as { status?: string };
      console.log(`[signalAdapter] runtime status: ${s.status}`);
      return [];
    }

    case 'runtime.sync.required': {
      const r = payload as { reason?: string; detail?: string };
      console.warn(`[signalAdapter] sync required: ${r.reason} — ${r.detail}`);
      return [];
    }

    default:
      // Unknown signal kinds are logged
      console.debug(`[signalAdapter] unhandled kernel signal kind: ${kind}`);
      return [];
  }
}
