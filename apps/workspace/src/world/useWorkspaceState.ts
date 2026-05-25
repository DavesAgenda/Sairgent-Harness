import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  ActivityLogEntry,
  Agent,
  AgentPresence,
  Bus,
  InboxItem,
  JobRecord,
  OutboxArtifact,
  RuntimeSignal,
  SwoRecord,
  WorkspaceWorld,
} from '../types';
import { agents as mockAgents } from '../sim/mockRoster';
import { computeLayout } from './layoutEngine';
import { computeTubes } from './tubePathComputer';

export interface WorkspaceWorldWithLog extends WorkspaceWorld {
  activityLog: ActivityLogEntry[];
}

let logIdCounter = 0;
function nextLogId(): string {
  return `log-${++logIdCounter}`;
}

function resolveAgentName(agents: Agent[], agentId: string): string {
  if (!agentId) return 'Unknown';
  // Match by ID first (UUID)
  const byId = agents.find((a) => a.id === agentId);
  if (byId) return byId.name;
  // Match by name (case-insensitive, handles "Name (Role)" format)
  const lower = agentId.toLowerCase();
  const byName = agents.find((a) => lower.startsWith(a.name.toLowerCase()));
  if (byName) return byName.name;
  return agentId;
}

/**
 * Find the root SWO ID for a given SWO by walking up the parent chain.
 */
function findRootSwoId(swoId: string, swos: Map<string, SwoRecord>): string {
  let current = swos.get(swoId);
  while (current?.parentSwoId) {
    const parent = swos.get(current.parentSwoId);
    if (!parent) break;
    current = parent;
  }
  return current?.id ?? swoId;
}

/**
 * CHA-429 — dedup an agent array by id, last-write-wins. Used at every
 * point where agent arrays get merged to prevent duplicate desks/bench
 * cards in the workspace grid. Dave observed the idle bench rendering
 * 22 agent names when the actual org was smaller; this is the defensive
 * shield against any path that might double-append.
 */
function dedupAgentsById(agents: Agent[]): Agent[] {
  const byId = new Map<string, Agent>();
  for (const agent of agents) {
    if (!agent || !agent.id) continue;
    byId.set(agent.id, agent);
  }
  return Array.from(byId.values());
}

/**
 * Master hook: Bus -> WorkspaceWorld state.
 * Accepts an optional agents list. Falls back to mockRoster when not provided.
 */
export function useWorkspaceState(
  bus: Bus,
  initialAgents?: Agent[],
): WorkspaceWorldWithLog {
  // Roster is stateful so dynamically hired agents (via agent.upserted signals)
  // can be appended at runtime. Bootstrap fills it from initialAgents.
  const [agents, setAgents] = useState<Agent[]>(() => dedupAgentsById(initialAgents ?? mockAgents));
  // Re-seed when the bootstrap roster arrives after first render.
  useEffect(() => {
    if (initialAgents && initialAgents.length > 0) {
      setAgents((prev) => {
        // CHA-429 — dedup initialAgents itself before merging. Bootstrap
        // payloads from the kernel tree can contain duplicates if an agent
        // appears under two parents in the roster tree (flattenRoster now
        // dedups upstream too, but this is a defensive belt-and-braces pass).
        const boot = dedupAgentsById(initialAgents);
        const bootstrapIds = new Set(boot.map((a) => a.id));
        // Keep any dynamically-added agents that aren't in the bootstrap.
        const dynamic = prev.filter((a) => !bootstrapIds.has(a.id));
        return dedupAgentsById([...boot, ...dynamic]);
      });
    }
  }, [initialAgents]);
  const agentsRef = useRef(agents);
  agentsRef.current = agents;

  const [swos, setSwos] = useState<Map<string, SwoRecord>>(new Map());
  const [presence, setPresence] = useState<Map<string, AgentPresence>>(new Map());
  const [inbox, setInbox] = useState<InboxItem[]>([]);
  const [activityLog, setActivityLog] = useState<ActivityLogEntry[]>([]);
  // Track which agents are actively delegating (for glow effects)
  const [delegatingAgents, setDelegatingAgents] = useState<Set<string>>(new Set());
  // Track status text per agent (streaming heartbeat/stdout snippets)
  const [agentStatusTexts, setAgentStatusTexts] = useState<Map<string, string>>(new Map());
  // Track per-agent live streaming activity text (from agent.activity.delta signals)
  const [agentLiveActivity, setAgentLiveActivity] = useState<Record<string, { text: string; updatedAt: number }>>({});
  // Track artifacts per root SWO ID
  const [artifactsBySwo, setArtifactsBySwo] = useState<Record<string, OutboxArtifact[]>>({});

  // Use refs for stable callback identity
  const swosRef = useRef(swos);
  swosRef.current = swos;

  const addLogEntry = useCallback(
    (agentId: string, kind: ActivityLogEntry['kind'], summary: string, swoId?: string) => {
      const resolvedRootSwoId = swoId
        ? findRootSwoId(swoId, swosRef.current)
        : undefined;
      const entry: ActivityLogEntry = {
        id: nextLogId(),
        timestamp: Date.now(),
        agentId,
        agentName: resolveAgentName(agentsRef.current, agentId),
        kind,
        summary,
        swoId: resolvedRootSwoId,
      };
      setActivityLog((prev) => [...prev, entry]);
    },
    [],
  );

  const handleSignal = useCallback(
    (signal: RuntimeSignal) => {
      const { type, payload } = signal;

      switch (type) {
        case 'swo.created': {
          const swo = payload.swo as SwoRecord;
          setSwos((prev) => {
            const next = new Map(prev);
            next.set(swo.id, swo);
            return next;
          });
          addLogEntry(swo.assigneeId, 'task_started', `Started: ${swo.title}`, swo.id);
          break;
        }
        case 'swo.updated': {
          const partial = payload.swo as Partial<SwoRecord> & { id: string };
          setSwos((prev) => {
            const existing = prev.get(partial.id);
            if (!existing) return prev;
            const next = new Map(prev);
            const updated = { ...existing, ...partial, updatedAt: Date.now() };
            next.set(partial.id, updated);

            // Log status transitions
            if (partial.status && partial.status !== existing.status) {
              if (partial.status === 'BLOCKED') {
                addLogEntry(existing.assigneeId, 'blocked', `Blocked: ${existing.title}`, existing.id);
              }
            }

            return next;
          });

          // Update status text with progress info when actively computing
          if (partial.status === 'IN_PROGRESS' || (partial as Record<string, unknown>).progress !== undefined) {
            setSwos((prev) => {
              const existing = prev.get(partial.id);
              if (existing) {
                const progressPct = Math.round((existing.progress ?? 0) * 100);
                setAgentStatusTexts((prevTexts) => {
                  const next = new Map(prevTexts);
                  next.set(existing.assigneeId, `Processing... ${progressPct}%`);
                  return next;
                });
              }
              return prev;
            });
          }
          break;
        }
        case 'swo.completed': {
          const partial = payload.swo as Partial<SwoRecord> & { id: string };
          setSwos((prev) => {
            const existing = prev.get(partial.id);
            if (!existing) return prev;
            const next = new Map(prev);
            next.set(partial.id, {
              ...existing,
              ...partial,
              status: 'COMPLETED',
              updatedAt: Date.now(),
            });
            addLogEntry(
              existing.assigneeId,
              'task_completed',
              `Completed: ${existing.title}`,
              existing.id,
            );
            // Clear status text on completion
            setAgentStatusTexts((prevTexts) => {
              const nextTexts = new Map(prevTexts);
              nextTexts.delete(existing.assigneeId);
              return nextTexts;
            });
            return next;
          });
          break;
        }
        case 'agent.upserted': {
          const a = payload as {
            id: string;
            name: string;
            role: string;
            parentId: string | null;
            reason: string;
            presence: AgentPresence;
          };
          if (!a.id || !a.name) break;
          // Append (or update) the dynamically hired agent in the live roster.
          // This makes its desk render, its UUID resolve to a name in the
          // activity log, and any subsequent delegation visible.
          setAgents((prev) => {
            const idx = prev.findIndex((existing) => existing.id === a.id);
            const newAgent: Agent = {
              id: a.id,
              name: a.name,
              role: a.role,
              title: a.role,
              parentId: a.parentId ?? null,
              skills: [],
              tools: [],
              icon: '🤖',
              orgClass: 'Specialist',
              raisonDetre: a.reason,
            };
            if (idx >= 0) {
              const next = [...prev];
              next[idx] = { ...next[idx], ...newAgent };
              return dedupAgentsById(next);
            }
            return dedupAgentsById([...prev, newAgent]);
          });
          // Initial presence
          setPresence((prev) => {
            const next = new Map(prev);
            next.set(a.id, a.presence ?? 'IDLE');
            return next;
          });
          // Log the hire so the user can see it happened
          addLogEntry(
            a.parentId ?? a.id,
            'task_started',
            `Hired ${a.name} (${a.role})${a.reason ? ` — ${a.reason}` : ''}`,
          );
          break;
        }
        case 'agent.presence.changed': {
          const { agentId, presence: p } = payload as {
            agentId: string;
            presence: AgentPresence;
          };
          setPresence((prev) => {
            const next = new Map(prev);
            next.set(agentId, p);
            return next;
          });
          // Clear status text and live activity when agent goes READY/IDLE
          if (p === 'READY' || p === 'IDLE') {
            setAgentStatusTexts((prev) => {
              const next = new Map(prev);
              next.delete(agentId);
              return next;
            });
            setAgentLiveActivity((prev) => {
              const next = { ...prev };
              delete next[agentId];
              return next;
            });
          }
          break;
        }
        case 'agent.activity.delta': {
          const { agentId, delta, isFinal } = payload as {
            agentId: string;
            delta: string;
            isFinal: boolean;
          };
          if (isFinal) {
            setAgentLiveActivity((prev) => {
              const next = { ...prev };
              delete next[agentId];
              return next;
            });
          } else {
            setAgentLiveActivity((prev) => {
              const existing = prev[agentId];
              const newText = ((existing?.text ?? '') + delta).slice(-160);
              return {
                ...prev,
                [agentId]: { text: newText, updatedAt: Date.now() },
              };
            });
          }
          break;
        }
        case 'delegation.started': {
          const d = payload as {
            fromAgentId: string;
            toAgentId: string;
            swoId: string;
          };
          addLogEntry(
            d.fromAgentId,
            'delegated',
            `Delegated to ${resolveAgentName(agentsRef.current, d.toAgentId)}`,
            d.swoId,
          );
          // Mark the from-agent as actively delegating
          setDelegatingAgents((prev) => {
            const next = new Set(prev);
            next.add(d.fromAgentId);
            return next;
          });
          break;
        }
        case 'delegation.completed': {
          const d = payload as {
            fromAgentId: string;
            toAgentId: string;
            swoId: string;
          };
          // Remove delegation flag when delegation completes
          setDelegatingAgents((prev) => {
            const next = new Set(prev);
            next.delete(d.fromAgentId);
            return next;
          });
          break;
        }
        case 'artifact.produced': {
          const a = payload as {
            agentId: string;
            artifact: { title: string; id?: number; path?: string };
            swoId?: string;
          };
          addLogEntry(
            a.agentId,
            'artifact_produced',
            `Produced: ${a.artifact.title}`,
            a.swoId,
          );
          // Track artifact in the artifactsBySwo map keyed by root SWO ID
          if (a.artifact.id != null && a.swoId) {
            const rootSwoId = findRootSwoId(a.swoId, swosRef.current);
            const artifact: OutboxArtifact = {
              id: a.artifact.id,
              swoId: a.swoId ? Number(a.swoId) : null,
              agentId: a.agentId,
              filename: a.artifact.title,
              absolutePath: a.artifact.path,
              createdAt: signal.timestamp,
            };
            setArtifactsBySwo((prev) => {
              const existing = prev[rootSwoId] ?? [];
              // Deduplicate by id
              if (existing.some((x) => x.id === artifact.id)) return prev;
              return { ...prev, [rootSwoId]: [...existing, artifact] };
            });
          }
          break;
        }
        case 'inbox.item.added': {
          const item = payload.item as InboxItem;
          setInbox((prev) => {
            if (prev.some((i) => i.id === item.id)) return prev;
            return [...prev, item];
          });
          break;
        }
      }
    },
    [addLogEntry],
  );

  useEffect(() => {
    return bus.subscribe(handleSignal);
  }, [bus, handleSignal]);

  const swoArray = useMemo(() => Array.from(swos.values()), [swos]);

  // Build the job list: root-level SWOs (parentSwoId === null), most recent first
  const jobs = useMemo((): JobRecord[] => {
    const rootSwos = swoArray.filter((s) => s.parentSwoId === null);
    const childMap = new Map<string, string[]>();

    // Build parent -> child ID mapping
    for (const swo of swoArray) {
      if (swo.parentSwoId) {
        const rootId = findRootSwoId(swo.id, swos);
        const existing = childMap.get(rootId) ?? [];
        existing.push(swo.id);
        childMap.set(rootId, existing);
      }
    }

    return rootSwos
      .map((swo): JobRecord => ({
        id: swo.id,
        title: swo.title,
        status: swo.status,
        assigneeId: swo.assigneeId,
        assigneeName: resolveAgentName(agents, swo.assigneeId),
        createdAt: swo.createdAt,
        updatedAt: swo.updatedAt,
        progress: swo.progress,
        childIds: childMap.get(swo.id) ?? [],
        reviewResponse: swo.reviewResponse,
      }))
      .sort((a, b) => b.createdAt - a.createdAt);
  }, [swoArray, swos, agents]);

  const world = useMemo((): WorkspaceWorldWithLog => {
    const { desks, bench } = computeLayout(agents, swoArray, presence, delegatingAgents, agentStatusTexts);
    const tubes = computeTubes(swoArray);
    return { desks, tubes, bench, inbox, activityLog, jobs, swoMap: swos, agentLiveActivity, artifactsBySwo };
  }, [agents, swoArray, presence, inbox, activityLog, delegatingAgents, agentStatusTexts, jobs, swos, agentLiveActivity, artifactsBySwo]);

  return world;
}

export function useResetWorkspace(bus: Bus): () => void {
  return useCallback(() => {
    bus.emit({ type: 'swo.created', timestamp: 0, payload: { _reset: true } });
  }, [bus]);
}

export function resetLogCounter(): void {
  logIdCounter = 0;
}
