export interface FeedMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | string;
  sender: string;
  senderAgentId?: string | null;
  messageKind: string;
  content: string;
  timestamp: number;
}

export type ProjectStatus = 'ACTIVE' | 'PAUSED' | 'COMPLETED' | 'ARCHIVED';
export type Priority = 'URGENT' | 'HIGH' | 'NORMAL' | 'LOW';

export interface Project {
  projectId: string;
  name: string;
  summary: string;
  status: ProjectStatus;
  owner: string;
  priority: Priority;
  targetOutcome: string;
  tags: string[];
  createdAt: string;
  updatedAt: string;
}

export interface ProjectStatusUpdate {
  projectId: string;
  previousStatus?: ProjectStatus | null;
  nextStatus: ProjectStatus;
  reason?: string | null;
  updatedBy: string;
  updatedAt: string;
}

export interface SWODependencyEdge {
  dependencyId: string;
  fromSwoId: number;
  toSwoId: number;
  dependencyType: 'FINISH_TO_START';
  requiredState: 'COMPLETED' | 'APPROVED';
  createdAt: string;
}

export interface SwoRecord {
  id: number;
  assignee: string;
  owner: string;
  createdBy: string;
  status: string;
  kind: string;
  source: string;
  workOrderTitle?: string | null;
  workOrderOutcome?: string | null;
  workOrderConstraints?: string | null;
  requestedOwner?: string | null;
  requestedAssignee?: string | null;
  routingPolicy: string;
  initiativeId?: string | null;
  initiativeName?: string | null;
  initiativeOwner?: string | null;
  priorityClass?: string | null;
  payload: string;
  createdAt: string;
  retryCount: number;
  actualChildAssignees: string[];
  childSwoCount: number;
  reviewStatus: string;
  mismatchFlags: string[];
  projectId?: string | null;
  parentSwoId?: number | null;
  reviewResponse?: string | null;
  revisionFeedback?: string | null;
}

export type RecurringCadence = 'hourly' | 'daily' | 'weekly' | 'monthly' | 'custom';
export type RecurringTemplateStatus = 'ACTIVE' | 'PAUSED' | 'CANCELLED' | 'ARCHIVED';
export type RecurringRunStatus =
  | 'SCHEDULED'
  | 'QUEUED'
  | 'RUNNING'
  | 'COMPLETED'
  | 'FAILED'
  | 'CANCELLED'
  | 'SKIPPED';

export interface RecurringWorkOrderSchedule {
  cadence: RecurringCadence;
  interval: number;
  timezone: string;
  daysOfWeek?: number[];
  dayOfMonth?: number | null;
  hour?: number | null;
  minute?: number | null;
  cronExpression?: string | null;
}

export interface RecurringWorkOrderTemplate {
  templateId: string;
  projectId?: string | null;
  sourceSwoId?: number | null;
  name: string;
  title: string;
  outcome: string;
  constraints?: string | null;
  owner: string;
  assignee?: string | null;
  priority: Priority;
  includePriorArtifacts: boolean;
  schedule: RecurringWorkOrderSchedule;
  status: RecurringTemplateStatus;
  nextRunAt?: string | null;
  lastRunAt?: string | null;
  lastRunStatus?: RecurringRunStatus | null;
  createdAt: string;
  updatedAt: string;
}

export interface RecurringWorkOrderRun {
  runId: string;
  templateId: string;
  swoId?: number | null;
  projectId?: string | null;
  runNumber: number;
  status: RecurringRunStatus;
  triggerSource: 'schedule' | 'manual' | 'replay';
  queuedAt: string;
  startedAt?: string | null;
  completedAt?: string | null;
  errorMessage?: string | null;
  artifactIds: number[];
}

export type AgentPresenceState = 'READY' | 'IDLE' | 'COMPUTING' | 'STALE' | 'OFFLINE';

export interface AgentSummary {
  id: string;
  name: string;
  role: string;
}

export type AgentOrgClass = 'manager' | 'lead_ic' | 'specialist';
export type DelegationPolicy = 'must_delegate_when_fit_exists' | 'may_delegate' | 'may_not_delegate';
export type ReviewPolicy = 'synthesize_only' | 'direct_allowed';
export type DelegationDecision = 'DELEGATE' | 'SELF_EXECUTE' | 'HIRE_THEN_DELEGATE' | 'ESCALATE_TEAM_GAP';
export type SelfExecutionExceptionCode =
  | 'NO_QUALIFIED_REPORT'
  | 'NO_REQUIRED_TOOLING'
  | 'URGENT_DIRECT_RESPONSE'
  | 'CROSS_FUNCTION_SYNTHESIS_REQUIRED'
  | 'TEAM_GAP_PENDING_HIRE';
export type ManagerValueOutcome =
  | 'delivered_final_response'
  | 'accepted_child_synthesis'
  | 'artifact_or_output_delivered'
  | 'reviewed_team_gap_escalation'
  | 'closed_failed';

export interface AgentOrgProfile {
  agentId: string;
  orgClass: AgentOrgClass;
  teamGoalIds: string[];
  delegationPolicy: DelegationPolicy;
  reviewPolicy: ReviewPolicy;
  managedDomains: string[];
  qualityRubric: string;
  maxDelegationDepth: number;
  maxParallelDelegates: number;
  managerCanHire: boolean;
  managerCanRestructure: boolean;
  updatedAt: string;
}

export interface TeamGoal {
  goalId: string;
  teamOwnerAgentId: string;
  title: string;
  summary: string;
  status: 'ACTIVE' | 'PAUSED' | 'ARCHIVED';
  priority: string;
  successCriteria: string;
  managedDomainTags: string[];
  createdAt: string;
  updatedAt: string;
  archivedAt?: string | null;
}

export interface DelegationDecisionRecord {
  id: string;
  swoId: number;
  managerAgentId: string;
  decision: DelegationDecision;
  candidateAssignees: string[];
  selectedAgentId?: string | null;
  fitReason?: string | null;
  exceptionCode?: SelfExecutionExceptionCode | string | null;
  exceptionReason?: string | null;
  teamGapCode?: string | null;
  createdAt: string;
}

export interface TeamGapRecord {
  id: string;
  swoId: number;
  managerAgentId: string;
  gapCode: string;
  summary: string;
  recommendedAction: string;
  createdAt: string;
}

export interface AgentTreeNode {
  id: string;
  name: string;
  role: string;
  manager?: AgentSummary | null;
  orgProfile: AgentOrgProfile;
  depth: number;
  isDirectReport: boolean;
  directReportCount: number;
  descendantCount: number;
  cronEnabled: boolean;
  presence: AgentPresenceState;
  lastSeenUnixMs?: number | null;
  lastSeenAgeMs?: number | null;
  lastCronFiredAt?: string | null;
  children: AgentTreeNode[];
}

export interface HeartbeatEvent {
  runId: string;
  status: string;
  lastSeenUnixMs: number;
  lastSeenAgeMs: number;
  seq: number;
}

export interface DirectReportSummary extends AgentSummary {
  cronEnabled: boolean;
  presence: AgentPresenceState;
  lastSeenUnixMs?: number | null;
  lastSeenAgeMs?: number | null;
}

export interface AgentSwoSummary extends SwoRecord { }

export interface HireDebugRecord {
  id: number;
  swoId: number;
  manager: string;
  newAgent: string;
  specJson: string;
  createdAt: string;
  parentMatchesManager: boolean;
  actualParent?: string | null;
}

export interface DelegationDebug {
  requestedAssignee?: string | null;
  routingPolicy: string;
  actualChildAssignees: string[];
  childSwoCount: number;
  reviewStatus: string;
  mismatchFlags: string[];
}

export interface ExecutionLineage {
  rootSwo?: SwoRecord | null;
  parentSwo?: SwoRecord | null;
  childSwos: SwoRecord[];
  linkedSwos: SwoRecord[];
  hires: HireDebugRecord[];
}

export interface AgentDetail {
  id: string;
  name: string;
  role: string;
  manager?: AgentSummary | null;
  orgProfile: AgentOrgProfile;
  teamGoals: TeamGoal[];
  delegationDecisions: DelegationDecisionRecord[];
  teamGaps: TeamGapRecord[];
  directReports: DirectReportSummary[];
  personaPrompt: string;
  raisonDetre: string;
  provider: string;
  model: string;
  cronIntervalSeconds?: number | null;
  presence: AgentPresenceState;
  lastSeenUnixMs?: number | null;
  lastSeenAgeMs?: number | null;
  lastCronFiredAt?: string | null;
  bio?: string | null;
  activeSwoCount?: number;
  completedSwoCount?: number;
  heartbeatTimeline: HeartbeatEvent[];
  assignedSwos: AgentSwoSummary[];
  ownedSwos: AgentSwoSummary[];
  createdSwos: AgentSwoSummary[];
  recentHires: HireDebugRecord[];
  interactions: InteractionExcerpt[];
  charterSettings: {
    raisonDetre: string;
    provider: string;
    model: string;
    cronIntervalSeconds?: number | null;
  };
  charterHistoryPlaceholder: string;
  manifest: AgentManifest;
  boundSkills: SkillBinding[];
  boundTools: AgentToolBinding[];
  externalChannelBindings?: ExternalChannelBinding[];
  mcpBindings?: AgentMcpBinding[];
  skills?: SkillBinding[];
  tools?: AgentToolBinding[];
}

export type ExternalChannelKind = 'telegram' | 'discord';

export interface ExternalChannelBinding {
  agentId: string;
  channel: ExternalChannelKind;
  enabled: boolean;
  allowedChatId?: string | null;
  allowedUserId?: string | null;
  hasRouteToken: boolean;
  hasSecretToken: boolean;
  lastInboundAt?: string | null;
  lastDeliveryAt?: string | null;
  lastDeliveryStatus: string;
  lastDeliveryDetail?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface GovernedExternalChannelTarget {
  channel: ExternalChannelKind;
  agentId?: string | null;
  agentName: string;
  agentRole?: string | null;
  bindingPolicy: 'fixed_agent';
  reason: string;
}

export interface ExternalChannelDeliveryEvent {
  id: number;
  agentId: string;
  channel: ExternalChannelKind;
  sessionId?: string | null;
  direction: 'inbound' | 'outbound';
  status: string;
  detail: string;
  externalChatId?: string | null;
  externalUserId?: string | null;
  externalMessageId?: string | null;
  createdAt: string;
}

export interface AgentManifest {
  version: string;
  name: string;
  role: string;
  mission: string;
  personaPrompt: string;
  providerName: string;
  model: string;
  protocolFamily: string;
  capabilities: string[];
  guardrails: Guardrail[];
  cronIntervalSeconds?: number | null;
  autonomousHeartbeat: boolean;
}

export interface Guardrail {
  code: string;
  description: string;
}

export interface SkillBinding {
  id: string;
  name: string;
  slug: string;
  summary: string;
  description?: string;
  tags: string[];
  triggerHints: string[];
  sourceUri?: string | null;
  currentVersion: number;
  priority: number;
  bindingStatus: string;
  preselected: boolean;
  runtimePath?: string | null;
}

export interface SkillCatalogItem {
  id: string;
  slug: string;
  name: string;
  summary: string;
  description?: string;
  tags: string[];
  triggerHints: string[];
  sourceUri?: string | null;
  ownerAgentId?: string | null;
  currentVersion: number;
  createdAt: string;
  updatedAt: string;
}

export interface SkillVersion {
  id: number;
  skillId: string;
  version: number;
  rawMarkdown: string;
  summary: string;
  tags: string[];
  triggerHints: string[];
  sourceUri?: string | null;
  createdAt: string;
}

export interface AgentToolBinding {
  slug: string;
  name: string;
  summary: string;
  description?: string;
  toolKind: string;
  providerSlug: string;
  requiredCapability: string;
  bindingStatus: string;
}

export type McpTransport = 'stdio' | 'sse';

export interface McpConnector {
  id: string;
  slug: string;
  name: string;
  summary: string;
  transport: McpTransport;
  command?: string | null;
  args?: string[] | null;
  env?: Record<string, string> | null;
  url?: string | null;
  headers?: Record<string, string> | null;
  cwd?: string | null;
  enabled: boolean;
  hasCredential: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface McpOAuthStatus {
  authenticated: boolean;
  provider: string;
  expiresAt: string | null;
}

export interface AgentMcpBinding {
  connectorId: string;
  connectorSlug: string;
  connectorName: string;
  transport: string;
  bindingStatus: string;
}

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

export interface ToolInventory {
  mcpConnectors: McpConnector[];
  cliTools: CliTool[];
}

export interface ToolCatalogItem {
  id: string;
  slug: string;
  name: string;
  summary: string;
  description?: string;
  toolKind: string;
  providerSlug: string;
  requiredCapability: string;
  credentialStatus: string;
  assignable: boolean;
}

export type ProviderAuthStrategy = 'api_key' | 'cli' | 'builtin';
export type ProviderRuntimeKind = 'remote_api' | 'local_cli' | 'embedded';
export type ProviderAvailability = 'ready' | 'configured' | 'detected' | 'missing';

export interface ProviderDescriptor {
  slug: string;
  label: string;
  description?: string | null;
  kind: 'llm';
  authStrategy: ProviderAuthStrategy;
  runtimeKind: ProviderRuntimeKind;
  envVar?: string | null;
  enabled: boolean;
  hasSecret: boolean;
  detected: boolean;
  authenticated: boolean;
  available: boolean;
  availability: ProviderAvailability;
  assignable: boolean;
  defaultModel?: string | null;
  supportedModels?: string[];
}

export interface ToolProviderDescriptor {
  slug: string;
  label: string;
  description?: string | null;
  authStrategy: ProviderAuthStrategy;
  runtimeKind: ProviderRuntimeKind;
  envVar?: string | null;
  enabled: boolean;
  hasSecret: boolean;
  detected: boolean;
  authenticated: boolean;
  available: boolean;
  availability: ProviderAvailability;
  assignable: boolean;
  toolKinds: string[];
  tools: ToolCatalogItem[];
}

export type DecisionLogOutcome = 'SUCCESS' | 'PARTIAL' | 'FAILED' | 'UNKNOWN';

export interface DecisionLogEntry {
  entryId: string;
  agentId: string;
  mode: string;
  summary: string;
  rationale: string;
  outcome: DecisionLogOutcome;
  confidence?: number | null;
  selfNote?: string | null;
  linkedSwoId?: number | null;
  linkedRunId?: string | null;
  createdAt: string;
}

export type SairgentChannel = 'desktop' | 'telegram' | 'discord' | 'system';
export type SairgentToolCallStatus =
  | 'proposed'
  | 'pending_confirmation'
  | 'executing'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface SairgentToolCall {
  callId: string;
  toolName: string;
  summary: string;
  argumentsJson: string;
  status: SairgentToolCallStatus;
  requiresConfirmation: boolean;
  resultSummary?: string | null;
  errorMessage?: string | null;
}

export interface SairgentChatMessage {
  id: string;
  conversationId: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  channel: SairgentChannel;
  createdAt: string;
  relatedProjectId?: string | null;
  relatedSwoId?: number | null;
  pendingToolCall?: SairgentToolCall | null;
  toolCalls: SairgentToolCall[];
  isStreaming?: boolean;
}

export interface ChatDeltaSignalPayload {
  messageId: string;
  delta: string;
  isFinal: boolean;
}

export interface ChatReplyEvent {
  sender: string;
  senderAgentId?: string | null;
  messageKind: string;
  content: string;
  timestamp: number;
}

export type AttachmentSourceKind = 'local_file' | 'workspace_file' | 'outbox_artifact';

export interface AttachmentInput {
  attachmentId: string;
  sourceKind: AttachmentSourceKind;
  displayName: string;
  originalPath: string;
  contentType: string;
  sizeBytes: number;
  originatingSwoId?: number | null;
  originatingArtifactId?: number | null;
}

export interface AttachmentSummary extends AttachmentInput {
  workspacePath?: string | null;
  deliveryStatus?: string;
  deliveryError?: string | null;
  createdAt?: string;
}

export interface SwoResult {
  id: number;
  producer: string;
  resultJson: string;
  createdAt: string;
}

export interface ManagerReview {
  id: number;
  reviewer: string;
  action: string;
  reasoning: string;
  finalResponse?: string | null;
  createdAt: string;
}

export interface OutboxArtifact {
  id: number;
  agent: string;
  agentId?: string | null;
  swoId?: number | null;
  parentSwoId?: number | null;
  rootSwoId?: number | null;
  projectId?: string | null;
  projectName?: string | null;
  sourceWorkOrderTitle?: string | null;
  sourceWorkOrderOutcome?: string | null;
  sourceStatus?: string | null;
  absolutePath: string;
  filename: string;
  contentType: string;
  sizeBytes: number;
  createdAt: string;
}

export type ArtifactRenderMode = 'markdown' | 'text' | 'json' | 'image' | 'binary';

export interface ArtifactPreview {
  contentType: string;
  displayName: string;
  path: string;
  previewText: string;
  truncated: boolean;
  renderMode?: ArtifactRenderMode;
  languageHint?: string | null;
  sourceArtifactId?: number | null;
  sourceSwoId?: number | null;
  projectId?: string | null;
}

export interface AgentHire {
  id: number;
  manager: string;
  newAgent: string;
  specJson: string;
  createdAt: string;
}

export interface InteractionExcerpt {
  agent: string;
  role: string;
  mode: string;
  interactionKind: string;
  content: string;
  timestamp: string;
}

export interface WorkerRun {
  id: number;
  runId: string;
  agent: string;
  backend: string;
  mode: string;
  status: string;
  startedAt: string;
  finishedAt?: string | null;
  artifactCount: number;
  structuredOutputPresent: boolean;
  blockedReason?: string | null;
  failureReason?: string | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  cacheReadTokens?: number | null;
  cacheWriteTokens?: number | null;
  requests?: number | null;
  costUsd?: number | null;
}

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

export interface SwoDetail {
  swo: SwoRecord;
  delegationStatus: string;
  delegationDebug: DelegationDebug;
  attachments: AttachmentSummary[];
  results: SwoResult[];
  reviews: ManagerReview[];
  artifacts: OutboxArtifact[];
  hires: AgentHire[];
  childSwos: SwoRecord[];
  linkedSwos: SwoRecord[];
  interactions: InteractionExcerpt[];
  workerRuns: WorkerRun[];
  executionLineage: ExecutionLineage;
}

export type ProjectOutputKind = 'result' | 'artifact';

export interface ProjectOutputItem {
  id: string;
  outputKind: ProjectOutputKind;
  artifactId?: number | null;
  resultId?: number | null;
  swoId: number;
  projectId?: string | null;
  projectName?: string | null;
  agentId: string;
  agentName: string;
  displayName: string;
  contentType: string;
  sizeBytes: number;
  createdAt: string;
  absolutePath?: string | null;
  previewText?: string | null;
  sourceWorkOrderTitle?: string | null;
  sourceWorkOrderOutcome?: string | null;
  sourceStatus?: string | null;
}

export interface ProjectActivityItem {
  id: string;
  projectId: string;
  kind:
    | 'project_status'
    | 'swo'
    | 'artifact'
    | 'result'
    | 'review'
    | 'attachment'
    | 'agent_hire'
    | 'worker_run'
    | 'audit';
  actorId?: string | null;
  actorName?: string | null;
  actorType: 'agent' | 'operator' | 'system' | 'artifact';
  timestamp: string;
  title: string;
  summary: string;
  detail?: string | null;
  status?: string | null;
  swoId?: number | null;
  artifactId?: number | null;
  relatedAgentId?: string | null;
}

export interface ProjectWorkspace {
  project: Project;
  swos: SwoRecord[];
  statusUpdates: ProjectStatusUpdate[];
  activity: ProjectActivityItem[];
  outputs: ProjectOutputItem[];
}

export interface AgentFileRecord {
  id: string;
  agentId: string;
  kind: 'input' | 'output';
  sourceKind: 'attachment' | 'artifact';
  displayName: string;
  contentType: string;
  sizeBytes: number;
  createdAt: string;
  swoId?: number | null;
  projectId?: string | null;
  projectName?: string | null;
  artifactId?: number | null;
  attachmentId?: string | null;
  workspacePath?: string | null;
  absolutePath?: string | null;
  deliveryStatus?: string | null;
  sourceWorkOrderTitle?: string | null;
}

export interface AgentHistoryEvent {
  id: string;
  agentId: string;
  kind:
    | 'swo'
    | 'artifact'
    | 'attachment'
    | 'decision'
    | 'memory_sync'
    | 'heartbeat'
    | 'review'
    | 'audit';
  timestamp: string;
  title: string;
  summary: string;
  detail?: string | null;
  status?: string | null;
  swoId?: number | null;
  artifactId?: number | null;
  projectId?: string | null;
  projectName?: string | null;
  runId?: string | null;
}

export type HsmStatus = 'READY' | 'THINKING' | 'ERROR' | string;

export interface RuntimeContext {
  companyName?: string | null;
  profileId?: string | null;
  companyCharterSource?: string | null;
  companySummary?: string | null;
  operatingPrinciples?: string[] | null;
  nonGoals?: string[] | null;
  autonomousHiringMode?: string | null;
  activeSeedSpecPath?: string | null;
  lastArchivePath?: string | null;
  sairgentAgentId?: string | null;
  preferredViewMode?: 'classic' | 'pixel-office' | null;
}

export interface RuntimeSeedResult {
  companyName: string;
  profileId: string;
  perryAgentId: string;
  agentCount: number;
  swoCount: number;
  archiveSnapshotId?: string | null;
  archivePath?: string | null;
}

export type RuntimeAudience =
  | 'desktop'
  | 'operator'
  | 'internal'
  | 'external_adapter';

export type RuntimeRedactionClass =
  | 'operator_safe'
  | 'internal_only'
  | 'secret_adjacent';

export interface RuntimePrincipal {
  kind: 'system' | 'agent' | 'operator' | 'adapter';
  id?: string | null;
  displayName?: string | null;
}

export interface RuntimeEnvelope {
  id: string;
  correlationId: string;
  source: string;
  principal: RuntimePrincipal;
  audience: RuntimeAudience;
  redactionClass: RuntimeRedactionClass;
  occurredAt: number;
  cursor: string;
}

export interface ApprovalQueueItem {
  id: string;
  swoId: number;
  title: string;
  reason: string;
  owner: string;
  status: string;
}

export type InboxItemKind = 'approval' | 'deliverable' | 'blocked';
export type InboxItemStatus = 'OPEN' | 'ACKNOWLEDGED' | 'RESOLVED';

export interface InboxItem {
  id: string;
  kind: InboxItemKind;
  status: InboxItemStatus;
  priority: Priority;
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

export interface InboxAttentionSummary {
  openInboxItems: number;
  openApprovalItems: number;
  openDeliverableItems: number;
  openBlockedItems: number;
}

export interface RuntimeCursor {
  value: string;
}

export interface RuntimeBootstrap {
  cursor: RuntimeCursor;
  hsmStatus: HsmStatus;
  runtimeContext: RuntimeContext | null;
  queue: SwoRecord[];
  roster: AgentTreeNode[];
  approvals: ApprovalQueueItem[];
  recentArtifacts: OutboxArtifact[];
  recentFeed: FeedMessage[];
  attentionSummary: InboxAttentionSummary;
  projects: Project[];
  dependencyEdges: SWODependencyEdge[];
  projectWorkspaces?: ProjectWorkspace[];
  recurringTemplates?: RecurringWorkOrderTemplate[];
  recurringRuns?: RecurringWorkOrderRun[];
  projectStatusUpdates?: ProjectStatusUpdate[];
  sairgentMessages?: SairgentChatMessage[];
  recentDecisionLog?: DecisionLogEntry[];
  externalDeliveryEvents?: ExternalChannelDeliveryEvent[];
}

export interface RuntimeStatusSignalPayload {
  status: string;
}

export interface FeedMessageSignalPayload {
  message: FeedMessage;
}

export interface SwoSignalPayload {
  swo: SwoRecord;
}

export interface ApprovalSignalPayload {
  approval: ApprovalQueueItem;
}

export interface ApprovalRemovedSignalPayload {
  approvalId: string;
  swoId: number;
}

export interface AgentPresenceSignalPayload {
  agentId: string;
  presence: AgentPresenceState;
  lastSeenUnixMs?: number | null;
  lastSeenAgeMs?: number | null;
}

export interface ArtifactSignalPayload {
  artifact: OutboxArtifact;
}

export interface AttachmentSignalPayload {
  attachment: AttachmentSummary;
  swoId?: number | null;
  projectId?: string | null;
}

export interface RuntimeSyncRequiredPayload {
  reason: string;
}

export interface DeliverySignalPayload {
  channel: string;
  status: 'queued' | 'delivered' | 'failed';
  detail: string;
}

export interface ProjectSignalPayload {
  project: Project;
}

export interface DependencySignalPayload {
  edge: SWODependencyEdge;
}

export interface ProjectStatusSignalPayload {
  update: ProjectStatusUpdate;
}

export interface ProjectActivitySignalPayload {
  activity: ProjectActivityItem;
}

export interface ProjectOutputSignalPayload {
  output: ProjectOutputItem;
}

export interface RecurringTemplateSignalPayload {
  template: RecurringWorkOrderTemplate;
}

export interface RecurringRunSignalPayload {
  run: RecurringWorkOrderRun;
}

export interface DecisionLogSignalPayload {
  entry: DecisionLogEntry;
}

export interface AgentConfigurationSignalPayload {
  agentId: string;
  profile: AgentOrgProfile;
}

export interface AgentHiredSignalPayload {
  agent: AgentDetail;
}

export interface AgentReportingLineSignalPayload {
  agentId: string;
  managerAgentId?: string | null;
}

export interface TeamGoalSignalPayload {
  goal: TeamGoal;
}

export interface DelegationDecisionSignalPayload {
  record: DelegationDecisionRecord;
}

export interface TeamGapSignalPayload {
  gap: TeamGapRecord;
}

export interface InboxItemSignalPayload {
  item: InboxItem;
}

export interface InboxItemResolvedSignalPayload {
  itemId: string;
  kind: InboxItemKind;
  resolvedAt: string;
  resolution?: string | null;
  projectId?: string | null;
  swoId?: number | null;
}

export interface SairgentMessageSignalPayload {
  message: SairgentChatMessage;
}

export type RuntimeSignal =
  | { envelope: RuntimeEnvelope; kind: 'runtime.status.changed'; payload: RuntimeStatusSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'feed.message.appended'; payload: FeedMessageSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'swo.upserted'; payload: SwoSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'approval.upserted'; payload: ApprovalSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'approval.removed'; payload: ApprovalRemovedSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'agent.presence.changed'; payload: AgentPresenceSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'artifact.created'; payload: ArtifactSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'attachment.upserted'; payload: AttachmentSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'delivery.status.changed'; payload: DeliverySignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'runtime.sync.required'; payload: RuntimeSyncRequiredPayload }
  | { envelope: RuntimeEnvelope; kind: 'project.upserted'; payload: ProjectSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'dependency_edge.upserted'; payload: DependencySignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'project.status.updated'; payload: ProjectStatusSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'project.activity.appended'; payload: ProjectActivitySignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'project.output.created'; payload: ProjectOutputSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'recurring.template.upserted'; payload: RecurringTemplateSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'recurring.run.upserted'; payload: RecurringRunSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'decision_log.appended'; payload: DecisionLogSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'agent.configuration.updated'; payload: AgentConfigurationSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'agent.hired'; payload: AgentHiredSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'agent.reporting_line.updated'; payload: AgentReportingLineSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'team.goal.upserted'; payload: TeamGoalSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'team.goal.archived'; payload: TeamGoalSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'delegation.decision.recorded'; payload: DelegationDecisionSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'team.gap.detected'; payload: TeamGapSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'inbox.item.upserted'; payload: InboxItemSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'inbox.item.resolved'; payload: InboxItemResolvedSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'sairgent.message.appended'; payload: SairgentMessageSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'sairgent.chat.delta'; payload: ChatDeltaSignalPayload }
  | { envelope: RuntimeEnvelope; kind: 'agent.activity.delta'; payload: { agentId: string; delta: string; isFinal: boolean } };

export interface RuntimeCommandMeta {
  commandId: string;
  correlationId: string;
  source: string;
  principal: RuntimePrincipal;
}

export interface ApprovalDecisionRequest {
  swoId: number;
  decision: 'approve' | 'reject' | 'revise';
  reasoning: string;
  finalResponse?: string | null;
  meta: RuntimeCommandMeta;
}

export interface SubmitWorkOrderRequest {
  title: string;
  outcome: string;
  constraints?: string | null;
  priority: string;
  projectId?: string | null;
  parentSwoId?: number | null;
  attachments: AttachmentInput[];
}

export interface ReviseAndRetrySwoRequest {
  swoId: number;
  title?: string | null;
  outcome?: string | null;
  constraints?: string | null;
  attachments?: AttachmentInput[];
  operatorNote?: string | null;
}

export interface AgentConfigurationUpdateRequest {
  agentId: string;
  orgProfile: AgentOrgProfile;
}

export interface TeamGoalUpsertRequest {
  goal: TeamGoal;
}

export interface TeamGoalArchiveRequest {
  goalId: string;
}

export interface AgentHireSubmitRequest {
  name: string;
  role: string;
  mission: string;
  provider: string;
  model: string;
  managerAgentId?: string | null;
  orgProfile: AgentOrgProfile;
}

export interface AgentReportingLineSetRequest {
  agentId: string;
  managerAgentId?: string | null;
}

export type RuntimeCommand =
  | ({
    kind: 'chat.send';
    payload: {
      agentId: string;
      message: string;
      attachments: AttachmentInput[];
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'work_order.submit';
    payload: SubmitWorkOrderRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'swo.retry';
    payload: {
      swoId: number;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'swo.revise_and_retry';
    payload: ReviseAndRetrySwoRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'approval.decide';
    payload: ApprovalDecisionRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'agent.configuration.update';
    payload: AgentConfigurationUpdateRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'team.goal.upsert';
    payload: TeamGoalUpsertRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'team.goal.archive';
    payload: TeamGoalArchiveRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'agent.hire.submit';
    payload: AgentHireSubmitRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'agent.reporting_line.set';
    payload: AgentReportingLineSetRequest;
  } & RuntimeCommandMeta)
  | ({
    kind: 'swo.manual_close';
    payload: {
      swoId: number;
      status: 'FAILED' | 'CANCELLED';
      reason?: string | null;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'heartbeat.trigger';
    payload: {
      agentId: string;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'agent.create';
    payload: {
      name: string;
      role: string;
      mission: string;
      provider: string;
      model: string;
      managerAgentId?: string | null;
      capabilities: string[];
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'project.submit';
    payload: {
      name: string;
      summary: string;
      owner: string;
      priority: string;
      targetOutcome: string;
      tags: string[];
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'project.status.set';
    payload: {
      projectId: string;
      status: ProjectStatus;
      reason?: string | null;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'dependency_edge.submit';
    payload: {
      fromSwoId: number;
      toSwoId: number;
      dependencyType: 'FINISH_TO_START';
      requiredState: 'COMPLETED' | 'APPROVED';
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'recurring.template.create';
    payload: Omit<RecurringWorkOrderTemplate, 'templateId' | 'createdAt' | 'updatedAt'>;
  } & RuntimeCommandMeta)
  | ({
    kind: 'recurring.template.update';
    payload: Pick<RecurringWorkOrderTemplate, 'templateId'> & Partial<RecurringWorkOrderTemplate>;
  } & RuntimeCommandMeta)
  | ({
    kind: 'recurring.template.trigger_now';
    payload: {
      templateId: string;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'recurring.template.pause';
    payload: {
      templateId: string;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'recurring.template.cancel';
    payload: {
      templateId: string;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'sairgent.chat.send';
    payload: {
      message: string;
      attachments: AttachmentInput[];
      relatedProjectId?: string | null;
      relatedSwoId?: number | null;
    };
  } & RuntimeCommandMeta)
  | ({
    kind: 'sairgent.tool.confirm';
    payload: {
      callId: string;
      approved: boolean;
    };
  } & RuntimeCommandMeta);
