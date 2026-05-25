// --- Agent Model ---
export type AgentPresence = 'READY' | 'IDLE' | 'COMPUTING' | 'STALE' | 'OFFLINE';

export interface Agent {
  id: string;
  name: string;
  role: string;
  title: string;
  parentId: string | null;
  skills: string[];
  tools: string[];
  icon: string;
  /** e.g. "Manager", "LeadIc", "Specialist" */
  orgClass?: string;
  /** LLM provider slug, e.g. "anthropic", "openai" */
  provider?: string;
  /** LLM model identifier, e.g. "claude-sonnet-4-6" */
  model?: string;
  /** Agent's mission / purpose statement */
  raisonDetre?: string;
  /** System prompt / persona instructions */
  personaPrompt?: string;
}

// --- Work Order Model ---
export type SwoStatus = 'PENDING' | 'IN_PROGRESS' | 'BLOCKED' | 'WAITING_REVIEW' | 'COMPLETED';

export interface SwoRecord {
  id: string;
  parentSwoId: string | null;
  title: string;
  assigneeId: string;
  status: SwoStatus;
  progress: number;
  createdAt: number;
  updatedAt: number;
  /** Deliverable text from the manager review (direct_answer / final_response). */
  reviewResponse?: string | null;
  /** The original request / outcome description. */
  outcome?: string | null;
  /** Raw payload JSON (fallback if outcome is empty). */
  payload?: string | null;
  /** Human revision feedback from the last Request Revision action. */
  revisionFeedback?: string | null;
}

// --- Signal Model ---
export type SignalType =
  | 'swo.created'
  | 'swo.updated'
  | 'swo.completed'
  | 'agent.upserted'
  | 'agent.presence.changed'
  | 'agent.activity.delta'
  | 'delegation.started'
  | 'delegation.completed'
  | 'artifact.produced'
  | 'inbox.item.added';

export interface RuntimeSignal {
  type: SignalType;
  timestamp: number;
  payload: Record<string, unknown>;
}

// --- Bus Interface ---
export interface Bus {
  subscribe(callback: (signal: RuntimeSignal) => void): () => void;
  emit(signal: RuntimeSignal): void;
}

/** Bootstrap data provided by a bus that connects to the real kernel. */
export interface BusBootstrap {
  agents: Agent[];
  swos: SwoRecord[];
  inbox: InboxItem[];
}

// --- Activity Log ---
export type ActivityKind =
  | 'task_started'
  | 'task_completed'
  | 'delegated'
  | 'blocked'
  | 'artifact_produced'
  | 'presence_changed';

export interface ActivityLogEntry {
  id: string;
  timestamp: number;
  agentId: string;
  agentName: string;
  kind: ActivityKind;
  summary: string;
  /** Root SWO ID this entry relates to, if any. */
  swoId?: string;
}

// --- Job History ---
export interface JobRecord {
  id: string;
  title: string;
  status: SwoStatus;
  assigneeId: string;
  assigneeName: string;
  createdAt: number;
  updatedAt: number;
  progress: number;
  /** Child SWO IDs representing the delegation tree. */
  childIds: string[];
  /** Deliverable text from the manager review, if available. */
  reviewResponse?: string | null;
}

// --- Token Usage ---
export interface TokenUsageRecord {
  id: number;
  runId: string;
  swoId: number | null;
  agentId: string;
  provider: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  requests: number;
  costUsd: number | null;
  createdAt: string;
}

export interface AgentTokenTotals {
  agentId: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  totalTokens: number;
  estimatedCostUsd: number | null;
  runCount: number;
}

// --- CLI Tool Model ---
export interface CliTool {
  id: string;
  slug: string;
  name: string;
  summary: string | null;
  command: string;
  args: string[] | null;
  env: Record<string, string> | null;
  cwd: string | null;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface CliToolUpsertRequest {
  id?: string;
  slug: string;
  name: string;
  summary?: string | null;
  command: string;
  args?: string[] | null;
  env?: Record<string, string> | null;
  cwd?: string | null;
  enabled?: boolean;
}

// --- Recurring Templates (Schedules) ---
export interface RecurringTemplateView {
  templateId: string;
  name: string;
  title: string;
  outcome: string;
  constraints: string | null;
  priority: string;
  assigneeAgentId: string | null;
  assigneeAgentName: string | null;
  scheduleJson: string;
  status: string;
  nextRunAt: string | null;
  lastRunAt: string | null;
  lastRunStatus: string | null;
  createdAt: string;
}

// --- MCP Connectors ---
export interface McpConnectorView {
  id: string;
  slug: string;
  name: string;
  summary: string;
  transport: string;
  command: string | null;
  args: string[] | null;
  url: string | null;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface McpConnectorUpsertRequest {
  connectorId?: string;
  slug: string;
  name: string;
  summary: string;
  transport: string;
  command?: string | null;
  args?: string[] | null;
  url?: string | null;
  enabled?: boolean;
}

/** Mirrors Rust DelegationDecisionRecordView */
export interface DelegationDecisionRecord {
  id: string;
  swoId: number;
  managerAgentId: string;
  decision: string;
  candidateAssignees: string[];
  selectedAgentId: string | null;
  fitReason: string | null;
  exceptionCode: string | null;
  exceptionReason: string | null;
  teamGapCode: string | null;
  createdAt: string;
}

/** Mirrors Rust TeamGoalView */
export interface TeamGoal {
  goalId: string;
  teamOwnerAgentId: string;
  title: string;
  summary: string;
  status: string;
  priority: string;
  successCriteria: string;
  managedDomainTags: string[];
  createdAt: string;
  updatedAt: string;
  archivedAt: string | null;
}

/** Mirrors Rust TeamGapRecordView */
export interface TeamGap {
  id: string;
  swoId: number;
  managerAgentId: string;
  gapCode: string;
  summary: string;
  recommendedAction: string;
  createdAt: string;
}

/** Mirrors Rust SkillBindingView */
export interface SkillBinding {
  id: string;
  name: string;
  slug: string;
  summary: string;
  tags: string[];
  triggerHints: string[];
  sourceUri: string | null;
  currentVersion: number;
  priority: number;
  bindingStatus: string;
  preselected: boolean;
  runtimePath: string | null;
}

/** Mirrors Rust AgentMcpBindingView */
export interface AgentMcpBinding {
  connectorId: string;
  connectorSlug: string;
  connectorName: string;
  transport: string;
  bindingStatus: string;
}

/** Mirrors Rust AgentToolBindingView */
export interface AgentToolBinding {
  slug: string;
  name: string;
  summary: string;
  toolKind: string;
  providerSlug: string;
  requiredCapability: string;
  bindingStatus: string;
}

// --- Artifacts ---

export interface OutboxArtifact {
  id: number;
  swoId: number | null;
  agentId: string | null;
  agentName?: string | null;
  filename: string;
  absolutePath?: string;
  createdAt: number;
  contentType?: string | null;
}

export interface ArtifactPreview {
  artifactId: number;
  filename: string;
  contentType: string;
  renderMode: 'markdown' | 'text' | 'json' | 'binary';
  content: string;
  sizeBytes: number;
  truncated: boolean;
}

// --- World State (skin contract) ---
export interface DeskState {
  agentId: string;
  name: string;
  role: string;
  icon: string;
  presence: AgentPresence;
  currentTask: string | null;
  /** Streaming status text (e.g. heartbeat/stdout snippets). */
  statusText: string | null;
  progress: number;
  /** Whether this desk has an active delegation in progress (for glow effects). */
  isDelegating: boolean;
  gridRow: number;
  gridCol: number;
}

export interface TubeState {
  id: string;
  fromAgentId: string;
  toAgentId: string;
  status: 'active' | 'blocked' | 'review' | 'complete';
  capsuleProgress: number;
  direction: 'down' | 'up';
}

export interface InboxItem {
  id: string;
  swoId: string;
  agentName: string;
  title: string;
  content: string;
  timestamp: number;
}

export interface WorkspaceWorld {
  desks: DeskState[];
  tubes: TubeState[];
  bench: DeskState[];
  inbox: InboxItem[];
  /** Root-level jobs, most recent first. */
  jobs: JobRecord[];
  /** Full SWO map for job detail lookups. */
  swoMap: Map<string, SwoRecord>;
  /** Per-agent streaming activity text, keyed by agentId. Cleared on isFinal or when agent goes idle. */
  agentLiveActivity?: Record<string, { text: string; updatedAt: number }>;
  /** Artifacts produced per root SWO ID. */
  artifactsBySwo?: Record<string, OutboxArtifact[]>;
}
