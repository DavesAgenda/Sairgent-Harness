import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { motion } from 'motion/react';
import { X, ChevronRight, Pencil, Save } from 'lucide-react';
import { ModelSelect } from './settings/ModelSelect';
import { isTauriRuntime } from '../sim/platform';
import type { ActivityLogEntry, Agent, AgentMcpBinding, AgentToolBinding, CliTool, DelegationDecisionRecord, DeskState, JobRecord, McpConnectorView, SkillBinding, SwoRecord, SwoStatus, TeamGap, TeamGoal } from '../types';

type Tab = 'overview' | 'history' | 'memory' | 'tools' | 'skills';

const TABS: { id: Tab; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'history', label: 'History' },
  { id: 'memory', label: 'Memory' },
  { id: 'tools', label: 'Tools' },
  { id: 'skills', label: 'Skills' },
];

const STATUS_BADGE: Record<SwoStatus, { label: string; color: string }> = {
  PENDING:        { label: 'Queued',  color: 'rgb(163 163 163)' },
  IN_PROGRESS:    { label: 'Running', color: 'rgb(96 165 250)' },
  BLOCKED:        { label: 'Blocked', color: 'rgb(248 113 113)' },
  WAITING_REVIEW: { label: 'Review',  color: 'rgb(168 85 247)' },
  COMPLETED:      { label: 'Done',    color: 'rgb(74 222 128)' },
};

function formatTimestamp(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

function formatDate(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

/** Save agent identity fields to the kernel via Tauri IPC. */
async function saveAgentIdentity(agentId: string, fields: {
  role?: string;
  raisonDetre?: string;
  personaPrompt?: string;
  defaultProvider?: string;
  defaultModel?: string;
}): Promise<void> {
  if (isTauriRuntime()) {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('agent_identity_update', {
      request: { agentId, ...fields },
    });
  }
}

/** Fields that were saved — used for optimistic local update */
export interface AgentSavedFields {
  agentId: string;
  role?: string;
  provider?: string;
  model?: string;
  raisonDetre?: string;
  personaPrompt?: string;
}

interface AgentCardModalProps {
  agentId: string;
  agents: Agent[];
  desks: DeskState[];
  jobs: JobRecord[];
  swoMap: Map<string, SwoRecord>;
  activityLog: ActivityLogEntry[];
  onClose: () => void;
  onJobClick?: (jobId: string) => void;
  /** Called after a successful save so the parent can update local agent state */
  onSaved?: (fields: AgentSavedFields) => void;
}

export function AgentCardModal({
  agentId: initialAgentId,
  agents,
  desks,
  jobs,
  swoMap,
  activityLog,
  onClose,
  onJobClick,
  onSaved,
}: AgentCardModalProps) {
  const [navStack, setNavStack] = useState<string[]>([]);
  const [activeTab, setActiveTab] = useState<Tab>('overview');
  const [editing, setEditing] = useState(false);
  const [saving, setSaving] = useState(false);
  const dirtyRef = useRef(false);
  const saveRef = useRef<(() => Promise<AgentSavedFields | undefined>) | null>(null);
  // Track mousedown origin to prevent accidental close when dragging text selection
  const mouseDownOnBackdropRef = useRef(false);

  // Current agent is the tip of the nav stack, or the initial agent
  const currentAgentId = navStack.length > 0 ? navStack[navStack.length - 1]! : initialAgentId;
  const agent = agents.find((a) => a.id === currentAgentId);

  // Breadcrumb trail: initial agent + each nav step
  const breadcrumbs = useMemo(() => {
    const ids = [initialAgentId, ...navStack];
    return ids.map((id) => agents.find((a) => a.id === id)).filter(Boolean) as Agent[];
  }, [initialAgentId, navStack, agents]);

  const navigateToAgent = useCallback((targetId: string) => {
    if (targetId === initialAgentId) {
      setNavStack([]);
    } else if (targetId === currentAgentId) {
      return;
    } else {
      setNavStack((prev) => [...prev, targetId]);
    }
    setActiveTab('overview');
    setEditing(false);
    dirtyRef.current = false;
  }, [initialAgentId, currentAgentId]);

  const navigateToBreadcrumb = useCallback((index: number) => {
    if (index === 0) {
      setNavStack([]);
    } else {
      setNavStack((prev) => prev.slice(0, index));
    }
    setActiveTab('overview');
    setEditing(false);
    dirtyRef.current = false;
  }, []);

  const handleBackdropClick = useCallback(() => {
    if (editing && dirtyRef.current) {
      if (!window.confirm('You have unsaved changes. Discard them?')) return;
    }
    onClose();
  }, [editing, onClose]);

  // Desk state for current agent (for current task / status text)
  const currentDesk = desks.find((d) => d.agentId === currentAgentId);

  // Reporting line
  const manager = useMemo(
    () => (agent?.parentId ? agents.find((a) => a.id === agent.parentId) : null),
    [agent, agents],
  );
  const directReports = useMemo(
    () => agents.filter((a) => a.parentId === currentAgentId),
    [agents, currentAgentId],
  );

  // Jobs this agent worked on
  const agentJobs = useMemo(() => {
    return jobs.filter((j) => {
      if (j.assigneeId === currentAgentId) return true;
      for (const swo of swoMap.values()) {
        if (swo.assigneeId === currentAgentId) {
          let current: SwoRecord | undefined = swo;
          while (current) {
            if (current.id === j.id) return true;
            current = current.parentSwoId ? swoMap.get(current.parentSwoId) : undefined;
          }
        }
      }
      return false;
    });
  }, [jobs, swoMap, currentAgentId]);

  // Recent activity for this agent
  const agentActivity = useMemo(
    () => activityLog.filter((e) => e.agentId === currentAgentId).slice(0, 20),
    [activityLog, currentAgentId],
  );

  if (!agent) return null;

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 90,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: '24px',
        backgroundColor: 'var(--ws-bg-overlay)',
      }}
      onMouseDown={(e) => {
        // Only set flag if mousedown was directly on the backdrop (not bubbled from modal)
        mouseDownOnBackdropRef.current = e.target === e.currentTarget;
      }}
      onClick={(e) => {
        // Only close if both mousedown AND mouseup were on the backdrop
        if (e.target === e.currentTarget && mouseDownOnBackdropRef.current) {
          handleBackdropClick();
        }
        mouseDownOnBackdropRef.current = false;
      }}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 12 }}
        transition={{ duration: 0.2, ease: 'easeOut' }}
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(720px, 100%)',
          maxHeight: 'calc(100vh - 48px)',
          backgroundColor: 'var(--ws-bg)',
          border: '1px solid var(--ws-border)',
          borderRadius: 'var(--ws-radius-md)',
          boxShadow: 'var(--ws-shadow-overlay)',
          display: 'flex',
          flexDirection: 'column',
          fontFamily: 'var(--font-mono, monospace)',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: 'var(--ws-space-lg) var(--ws-space-xl)',
            borderBottom: '1px solid var(--ws-border-subtle)',
            backgroundColor: 'var(--ws-bg-elevated)',
            flexShrink: 0,
          }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: '4px', minWidth: 0, flex: 1 }}>
            {breadcrumbs.length > 1 && (
              <div style={{ display: 'flex', alignItems: 'center', gap: '4px', marginBottom: '4px', flexWrap: 'wrap' }}>
                {breadcrumbs.map((crumb, i) => (
                  <span key={crumb.id} style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                    {i > 0 && <ChevronRight size={10} style={{ color: 'var(--ws-fg-dim)' }} />}
                    <button
                      onClick={(e) => { e.stopPropagation(); navigateToBreadcrumb(i); }}
                      style={{
                        fontFamily: 'inherit',
                        fontSize: 'var(--ws-font-xs)',
                        color: i < breadcrumbs.length - 1 ? 'var(--ws-accent)' : 'var(--ws-fg-muted)',
                        background: 'none',
                        border: 'none',
                        cursor: i < breadcrumbs.length - 1 ? 'pointer' : 'default',
                        padding: '2px 4px',
                        letterSpacing: '0.05em',
                        textDecoration: i < breadcrumbs.length - 1 ? 'underline' : 'none',
                        textUnderlineOffset: '2px',
                      }}
                    >
                      {crumb.icon} {crumb.name}
                    </button>
                  </span>
                ))}
              </div>
            )}
            <div style={{ display: 'flex', alignItems: 'center', gap: '16px' }}>
              <div
                style={{
                  width: '52px',
                  height: '52px',
                  border: '1px solid var(--ws-border)',
                  borderRadius: 'var(--ws-radius-sm)',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  fontSize: '1.6rem',
                  color: 'var(--ws-fg-primary)',
                  backgroundColor: 'var(--ws-accent-soft)',
                  flexShrink: 0,
                }}
              >
                {agent.icon}
              </div>
              <div style={{ minWidth: 0 }}>
                <div
                  style={{
                    fontSize: 'var(--ws-font-xl)',
                    fontWeight: 700,
                    color: 'var(--ws-fg-primary)',
                    letterSpacing: '0.06em',
                    textTransform: 'uppercase',
                  }}
                >
                  {agent.name}
                </div>
                <div style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-secondary)', marginTop: '2px' }}>
                  {agent.title || agent.role}
                </div>
              </div>
            </div>
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: '6px', alignSelf: 'flex-start', flexShrink: 0 }}>
            {editing ? (
              <>
                {/* Save */}
                <button
                  onClick={async () => {
                    if (!saveRef.current) return;
                    setSaving(true);
                    try {
                      const saved = await saveRef.current();
                      if (saved) onSaved?.(saved);
                      setEditing(false);
                      dirtyRef.current = false;
                    } catch (err) {
                      console.error('[AgentCard] Save failed:', err);
                    } finally {
                      setSaving(false);
                    }
                  }}
                  disabled={saving}
                  style={{
                    fontFamily: 'inherit',
                    fontSize: 'var(--ws-font-xs)',
                    letterSpacing: '0.06em',
                    color: 'var(--ws-bg)',
                    backgroundColor: 'var(--ws-accent)',
                    border: 'none',
                    borderRadius: 'var(--ws-radius-sm)',
                    padding: '4px 12px',
                    cursor: saving ? 'wait' : 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    gap: '4px',
                    textTransform: 'uppercase',
                    opacity: saving ? 0.6 : 1,
                  }}
                >
                  <Save size={11} />
                  {saving ? 'Saving...' : 'Save'}
                </button>
                {/* Cancel */}
                <button
                  onClick={() => {
                    setEditing(false);
                    dirtyRef.current = false;
                  }}
                  style={{
                    fontFamily: 'inherit',
                    fontSize: 'var(--ws-font-xs)',
                    letterSpacing: '0.06em',
                    color: 'var(--ws-fg-muted)',
                    backgroundColor: 'transparent',
                    border: '1px solid var(--ws-border-subtle)',
                    borderRadius: 'var(--ws-radius-sm)',
                    padding: '4px 10px',
                    cursor: 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    gap: '4px',
                    textTransform: 'uppercase',
                    transition: 'color 0.15s ease',
                  }}
                  onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--ws-fg-primary)'; }}
                  onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--ws-fg-muted)'; }}
                >
                  Cancel
                </button>
              </>
            ) : (
              <button
                onClick={() => setEditing(true)}
                style={{
                  fontFamily: 'inherit',
                  fontSize: 'var(--ws-font-xs)',
                  letterSpacing: '0.06em',
                  color: 'var(--ws-fg-muted)',
                  backgroundColor: 'transparent',
                  border: '1px solid var(--ws-border-subtle)',
                  borderRadius: 'var(--ws-radius-sm)',
                  padding: '4px 10px',
                  cursor: 'pointer',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '4px',
                  textTransform: 'uppercase',
                  transition: 'all 0.15s ease',
                }}
                onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--ws-fg-primary)'; }}
                onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--ws-fg-muted)'; }}
              >
                <Pencil size={11} />
                Edit
              </button>
            )}
            {/* Close */}
            <button
              onClick={handleBackdropClick}
              style={{
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                color: 'var(--ws-fg-muted)',
                padding: '4px',
                display: 'flex',
                alignItems: 'center',
                transition: 'color 0.15s ease',
              }}
              onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--ws-fg-primary)'; }}
              onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--ws-fg-muted)'; }}
            >
              <X size={16} />
            </button>
          </div>
        </div>

        {/* Tab bar */}
        <div
          style={{
            display: 'flex',
            gap: '0',
            borderBottom: '1px solid var(--ws-border-subtle)',
            backgroundColor: 'var(--ws-bg)',
            flexShrink: 0,
            paddingLeft: 'var(--ws-space-xl)',
          }}
        >
          {TABS.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              style={{
                fontFamily: 'inherit',
                fontSize: 'var(--ws-font-sm)',
                letterSpacing: '0.06em',
                textTransform: 'uppercase',
                color: activeTab === tab.id ? 'var(--ws-fg-primary)' : 'var(--ws-fg-muted)',
                background: 'none',
                border: 'none',
                borderBottom: activeTab === tab.id
                  ? '2px solid var(--ws-accent)'
                  : '2px solid transparent',
                padding: '10px 16px',
                cursor: 'pointer',
                transition: 'color 0.15s ease, border-color 0.15s ease',
              }}
              onMouseEnter={(e) => {
                if (activeTab !== tab.id) e.currentTarget.style.color = 'var(--ws-fg-secondary)';
              }}
              onMouseLeave={(e) => {
                if (activeTab !== tab.id) e.currentTarget.style.color = 'var(--ws-fg-muted)';
              }}
            >
              {tab.label}
            </button>
          ))}
        </div>

        {/* Tab content */}
        <div style={{ flex: 1, overflow: 'auto', padding: 'var(--ws-space-xl)' }}>
          {activeTab === 'overview' && (
            <OverviewTab
              agent={agent}
              manager={manager ?? null}
              directReports={directReports}
              currentTask={currentDesk?.currentTask ?? null}
              statusText={currentDesk?.statusText ?? null}
              recentActivity={agentActivity}
              onAgentClick={navigateToAgent}
              editing={editing}
              onDirtyChange={(dirty) => { dirtyRef.current = dirty; }}
              saveRef={saveRef}
            />
          )}
          {activeTab === 'history' && (
            <HistoryTab agentJobs={agentJobs} onJobClick={onJobClick} />
          )}
          {activeTab === 'memory' && <MemoryTab agent={agent} />}
          {activeTab === 'tools' && <ToolsTab agent={agent} />}
          {activeTab === 'skills' && <SkillsTab agent={agent} />}
        </div>

        {/* Footer */}
        <div
          style={{
            padding: '6px 16px',
            borderTop: '1px solid var(--ws-border-subtle)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            backgroundColor: 'var(--ws-bg-elevated)',
            flexShrink: 0,
          }}
        >
          <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', letterSpacing: '0.06em' }}>
            {agent.role.toUpperCase()} {agent.parentId ? '' : ' \u00b7 TOP-LEVEL'}
          </span>
          <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', letterSpacing: '0.06em' }}>
            ESC / CLICK OUTSIDE TO CLOSE
          </span>
        </div>
      </motion.div>
    </motion.div>
  );
}

/* ---------------------------------------------------------------
   Overview Tab
   --------------------------------------------------------------- */

function OverviewTab({
  agent,
  manager,
  directReports,
  currentTask,
  statusText,
  recentActivity,
  onAgentClick,
  editing,
  onDirtyChange,
  saveRef,
}: {
  agent: Agent;
  manager: Agent | null;
  directReports: Agent[];
  currentTask: string | null;
  statusText: string | null;
  recentActivity: ActivityLogEntry[];
  onAgentClick: (agentId: string) => void;
  editing: boolean;
  onDirtyChange: (dirty: boolean) => void;
  saveRef: React.MutableRefObject<(() => Promise<AgentSavedFields | undefined>) | null>;
}) {
  const [draftRole, setDraftRole] = useState(agent.role ?? '');
  const [draftProvider, setDraftProvider] = useState(agent.provider ?? '');
  const [draftModel, setDraftModel] = useState(agent.model ?? '');
  const [draftRaison, setDraftRaison] = useState(agent.raisonDetre ?? '');
  const [draftPrompt, setDraftPrompt] = useState(agent.personaPrompt ?? '');

  // Sync drafts when navigating to a different agent
  const [prevAgentId, setPrevAgentId] = useState(agent.id);
  if (agent.id !== prevAgentId) {
    setPrevAgentId(agent.id);
    setDraftRole(agent.role ?? '');
    setDraftProvider(agent.provider ?? '');
    setDraftModel(agent.model ?? '');
    setDraftRaison(agent.raisonDetre ?? '');
    setDraftPrompt(agent.personaPrompt ?? '');
  }

  const isDirty =
    draftRole !== (agent.role ?? '') ||
    draftProvider !== (agent.provider ?? '') ||
    draftModel !== (agent.model ?? '') ||
    draftRaison !== (agent.raisonDetre ?? '') ||
    draftPrompt !== (agent.personaPrompt ?? '');

  // Notify parent of dirty state
  const prevDirty = useRef(false);
  if (isDirty !== prevDirty.current) {
    prevDirty.current = isDirty;
    onDirtyChange(isDirty);
  }

  // Register save function for the header Save button to call
  saveRef.current = useCallback(async (): Promise<AgentSavedFields | undefined> => {
    const fields: Record<string, string | undefined> = {};
    if (draftRole !== (agent.role ?? '')) fields.role = draftRole;
    if (draftRaison !== (agent.raisonDetre ?? '')) fields.raisonDetre = draftRaison;
    if (draftPrompt !== (agent.personaPrompt ?? '')) fields.personaPrompt = draftPrompt;
    if (draftProvider !== (agent.provider ?? '')) fields.defaultProvider = draftProvider;
    if (draftModel !== (agent.model ?? '')) fields.defaultModel = draftModel;

    await saveAgentIdentity(agent.id, fields);

    // Return saved values for optimistic local update
    return {
      agentId: agent.id,
      role: draftRole,
      provider: draftProvider,
      model: draftModel,
      raisonDetre: draftRaison,
      personaPrompt: draftPrompt,
    };
  }, [agent, draftRole, draftProvider, draftModel, draftRaison, draftPrompt]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ws-space-xl)' }}>
      {/* Current status */}
      {(currentTask || statusText) && (
        <div
          style={{
            border: '1px solid var(--ws-border)',
            borderRadius: 'var(--ws-radius-sm)',
            padding: 'var(--ws-space-md) var(--ws-space-lg)',
            backgroundColor: 'var(--ws-accent-soft)',
          }}
        >
          <SectionLabel>Current Task</SectionLabel>
          <p style={{ fontSize: 'var(--ws-font-base)', color: 'var(--ws-fg-primary)', margin: '6px 0 0' }}>
            {currentTask}
          </p>
          {statusText && (
            <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-secondary)', margin: '4px 0 0', fontStyle: 'italic' }}>
              {statusText}
            </p>
          )}
        </div>
      )}

      {/* Role + Org Class */}
      <div>
        <SectionLabel>Role</SectionLabel>
        {editing ? (
          <input
            value={draftRole}
            onChange={(e) => setDraftRole(e.target.value)}
            style={editInputStyle}
            placeholder="e.g. CTO, Research Lead"
          />
        ) : (
          <p style={{ fontSize: 'var(--ws-font-base)', color: 'var(--ws-fg-primary)', margin: '6px 0 0' }}>
            {agent.role}{agent.title ? ` \u2014 ${agent.title}` : ''}
            {agent.orgClass && (
              <span
                style={{
                  marginLeft: '8px',
                  fontSize: 'var(--ws-font-xs)',
                  color: 'var(--ws-fg-muted)',
                  border: '1px solid var(--ws-border-subtle)',
                  borderRadius: 'var(--ws-radius-sm)',
                  padding: '1px 6px',
                }}
              >
                {agent.orgClass}
              </span>
            )}
          </p>
        )}
      </div>

      {/* Model — uses ModelSelect dropdown in edit mode */}
      <div>
        <SectionLabel>Model</SectionLabel>
        {editing ? (
          <div style={{ marginTop: '6px' }}>
            <div style={{ display: 'flex', gap: '8px', marginBottom: '6px' }}>
              <select
                value={draftProvider}
                onChange={(e) => { setDraftProvider(e.target.value); setDraftModel(''); }}
                style={{
                  ...editInputStyle,
                  marginTop: 0,
                  width: 'auto',
                  minWidth: '140px',
                  cursor: 'pointer',
                }}
              >
                <option value="">Select provider</option>
                <option value="anthropic">Anthropic</option>
                <option value="openai">OpenAI</option>
                <option value="openrouter">OpenRouter</option>
                <option value="groq">Groq</option>
              </select>
            </div>
            {draftProvider && (
              <ModelSelect
                provider={draftProvider}
                value={draftModel}
                onChange={setDraftModel}
                placeholder="Search models..."
              />
            )}
          </div>
        ) : (
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', margin: '6px 0 0', fontFamily: 'inherit' }}>
            {agent.model ? (
              <>
                {agent.model}
                {agent.provider && (
                  <span style={{ color: 'var(--ws-fg-muted)', marginLeft: '6px' }}>
                    ({agent.provider})
                  </span>
                )}
              </>
            ) : (
              <span style={{ color: 'var(--ws-fg-dim)' }}>Not set</span>
            )}
          </p>
        )}
      </div>

      {/* Mission */}
      <div>
        <SectionLabel>Mission</SectionLabel>
        {editing ? (
          <textarea
            value={draftRaison}
            onChange={(e) => setDraftRaison(e.target.value)}
            rows={3}
            style={{ ...editInputStyle, resize: 'vertical', minHeight: '60px' }}
            placeholder="What is this agent's purpose?"
          />
        ) : (
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', margin: '6px 0 0', lineHeight: '1.5' }}>
            {agent.raisonDetre || <span style={{ color: 'var(--ws-fg-dim)' }}>Not set</span>}
          </p>
        )}
      </div>

      {/* Persona Prompt */}
      <div>
        <SectionLabel>System Prompt</SectionLabel>
        {editing ? (
          <textarea
            value={draftPrompt}
            onChange={(e) => setDraftPrompt(e.target.value)}
            rows={4}
            style={{ ...editInputStyle, resize: 'vertical', minHeight: '80px' }}
            placeholder="Agent's persona / system prompt instructions"
          />
        ) : (
          agent.personaPrompt ? (
            <p style={{
              fontSize: 'var(--ws-font-sm)',
              color: 'var(--ws-fg-secondary)',
              margin: '6px 0 0',
              lineHeight: '1.5',
              whiteSpace: 'pre-wrap',
              maxHeight: '120px',
              overflow: 'auto',
            }}>
              {agent.personaPrompt}
            </p>
          ) : (
            <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-dim)', margin: '6px 0 0' }}>
              Not set
            </p>
          )
        )}
      </div>

      {/* Reporting line */}
      <div>
        <SectionLabel>Reporting Line</SectionLabel>
        <div style={{ marginTop: '8px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
          {manager ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', width: '80px', flexShrink: 0 }}>
                Reports to
              </span>
              <AgentChip agent={manager} onClick={() => onAgentClick(manager.id)} />
            </div>
          ) : (
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', width: '80px', flexShrink: 0 }}>
                Reports to
              </span>
              <span style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-dim)' }}>
                None (top-level)
              </span>
            </div>
          )}
          {directReports.length > 0 && (
            <div style={{ display: 'flex', alignItems: 'flex-start', gap: '8px' }}>
              <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', width: '80px', flexShrink: 0, paddingTop: '4px' }}>
                Manages
              </span>
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px' }}>
                {directReports.map((r) => (
                  <AgentChip key={r.id} agent={r} onClick={() => onAgentClick(r.id)} />
                ))}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Recent activity */}
      {recentActivity.length > 0 && (
        <div>
          <SectionLabel>Recent Activity</SectionLabel>
          <div style={{ marginTop: '8px' }}>
            {recentActivity.slice(0, 8).map((entry) => (
              <div
                key={entry.id}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: '8px',
                  paddingTop: '3px',
                  paddingBottom: '3px',
                  borderBottom: '1px solid var(--ws-border-subtle)',
                }}
              >
                <span style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-xs)', fontVariantNumeric: 'tabular-nums', flexShrink: 0 }}>
                  {formatTimestamp(entry.timestamp)}
                </span>
                <span style={{ color: 'var(--ws-fg-secondary)', fontSize: 'var(--ws-font-xs)', flex: 1 }}>
                  {entry.summary}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

const editInputStyle: React.CSSProperties = {
  fontFamily: 'inherit',
  fontSize: 'var(--ws-font-sm)',
  color: 'var(--ws-fg-primary)',
  backgroundColor: 'var(--ws-bg)',
  border: '1px solid var(--ws-border)',
  borderRadius: 'var(--ws-radius-sm)',
  padding: '8px 10px',
  width: '100%',
  marginTop: '6px',
  outline: 'none',
  lineHeight: '1.5',
};

/* ---------------------------------------------------------------
   History Tab
   --------------------------------------------------------------- */

function HistoryTab({
  agentJobs,
  onJobClick,
}: {
  agentJobs: JobRecord[];
  onJobClick?: (jobId: string) => void;
}) {
  if (agentJobs.length === 0) {
    return (
      <div style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)', textAlign: 'center', padding: 'var(--ws-space-2xl)' }}>
        No job history yet.
      </div>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
      {agentJobs.map((job) => {
        const badge = STATUS_BADGE[job.status];
        return (
          <button
            key={job.id}
            type="button"
            onClick={() => onJobClick?.(job.id)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '10px',
              padding: '8px 12px',
              width: '100%',
              background: 'none',
              border: 'none',
              borderRadius: 'var(--ws-radius-sm)',
              cursor: onJobClick ? 'pointer' : 'default',
              fontFamily: 'inherit',
              textAlign: 'left',
              transition: 'background 0.15s ease',
            }}
            onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--ws-accent-soft)'; }}
            onMouseLeave={(e) => { e.currentTarget.style.background = 'none'; }}
          >
            <span style={{ width: '8px', height: '8px', borderRadius: '50%', backgroundColor: badge.color, flexShrink: 0 }} />
            <span style={{ color: 'var(--ws-fg-primary)', fontSize: 'var(--ws-font-sm)', flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {job.title}
            </span>
            <span style={{ fontSize: 'var(--ws-font-xs)', color: badge.color, backgroundColor: badge.color.replace(')', ' / 0.1)'), border: `1px solid ${badge.color.replace(')', ' / 0.25)')}`, padding: '2px 6px', borderRadius: 'var(--ws-radius-sm)', flexShrink: 0, letterSpacing: '0.04em' }}>
              {badge.label}
            </span>
            <span style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-xs)', flexShrink: 0, fontVariantNumeric: 'tabular-nums' }}>
              {formatDate(job.createdAt)}
            </span>
            {onJobClick && <ChevronRight size={12} style={{ color: 'var(--ws-fg-dim)', flexShrink: 0 }} />}
          </button>
        );
      })}
    </div>
  );
}

/* ---------------------------------------------------------------
   Memory Tab — decision log, team goals, team gaps
   --------------------------------------------------------------- */

function MemoryTab({ agent }: { agent: Agent }) {
  const [decisions, setDecisions] = useState<DelegationDecisionRecord[]>([]);
  const [teamGoals, setTeamGoals] = useState<TeamGoal[]>([]);
  const [teamGaps, setTeamGaps] = useState<TeamGap[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadData();
  }, [agent.id]);

  async function loadData() {
    if (!isTauriRuntime()) {
      setLoading(false);
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const detail = await invoke<{
        delegationDecisions: DelegationDecisionRecord[];
        teamGoals: TeamGoal[];
        teamGaps: TeamGap[];
      }>('agent_detail', { agentId: agent.id });
      setDecisions(detail.delegationDecisions ?? []);
      setTeamGoals(detail.teamGoals ?? []);
      setTeamGaps(detail.teamGaps ?? []);
    } catch (err) {
      console.error('Failed to load agent memory', err);
    } finally {
      setLoading(false);
    }
  }

  if (loading) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 'var(--ws-space-2xl)', color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)' }}>
        Loading memory...
      </div>
    );
  }

  if (!isTauriRuntime()) {
    return (
      <div style={{ border: '1px dashed var(--ws-border)', borderRadius: 'var(--ws-radius-md)', padding: 'var(--ws-space-xl)', textAlign: 'center' }}>
        <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-muted)', margin: 0 }}>Memory view requires live kernel</p>
        <p style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', margin: '6px 0 0' }}>Run the Tauri app to see decision log, team goals, and gaps.</p>
      </div>
    );
  }

  const sectionRowStyle: React.CSSProperties = {
    padding: '10px 12px',
    borderBottom: '1px solid var(--ws-border-soft)',
  };
  const sectionWrapStyle: React.CSSProperties = {
    marginTop: '10px',
    borderRadius: 'var(--ws-radius-md)',
    overflow: 'hidden',
    border: '1px solid var(--ws-border-soft)',
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ws-space-xl)' }}>

      {/* Delegation Decisions */}
      <div>
        <SectionLabel>Decision Log</SectionLabel>
        {decisions.length === 0 ? (
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-dim)', margin: '10px 0 0' }}>
            No decisions recorded yet.
          </p>
        ) : (
          <div style={sectionWrapStyle}>
            {decisions.slice(0, 20).map(d => (
              <div key={d.id} style={sectionRowStyle}>
                <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: '10px' }}>
                  <span style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', fontWeight: 500 }}>
                    {d.decision.toUpperCase()}
                    {d.selectedAgentId && <span style={{ color: 'var(--ws-fg-muted)', fontWeight: 400 }}> → {d.selectedAgentId}</span>}
                  </span>
                  <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', flexShrink: 0 }}>
                    {formatRelativeTime(d.createdAt)}
                  </span>
                </div>
                {d.fitReason && (
                  <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', marginTop: '4px', lineHeight: 1.4 }}>
                    {d.fitReason}
                  </div>
                )}
                {d.exceptionReason && (
                  <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-status-warning, #e0a458)', marginTop: '4px', lineHeight: 1.4 }}>
                    Exception: {d.exceptionReason}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Team Goals */}
      <div>
        <SectionLabel>Team Goals</SectionLabel>
        {teamGoals.length === 0 ? (
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-dim)', margin: '10px 0 0' }}>
            No goals set.
          </p>
        ) : (
          <div style={sectionWrapStyle}>
            {teamGoals.map(g => (
              <div key={g.goalId} style={sectionRowStyle}>
                <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: '10px' }}>
                  <span style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', fontWeight: 500 }}>
                    {g.title}
                  </span>
                  <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', border: '1px solid var(--ws-border)', borderRadius: 'var(--ws-radius-sm)', padding: '1px 6px', flexShrink: 0 }}>
                    {g.status}
                  </span>
                </div>
                {g.summary && (
                  <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', marginTop: '4px', lineHeight: 1.4 }}>
                    {g.summary}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Team Gaps */}
      <div>
        <SectionLabel>Team Gaps</SectionLabel>
        {teamGaps.length === 0 ? (
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-dim)', margin: '10px 0 0' }}>
            No gaps identified.
          </p>
        ) : (
          <div style={sectionWrapStyle}>
            {teamGaps.map(gap => (
              <div key={gap.id} style={sectionRowStyle}>
                <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: '10px' }}>
                  <span style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', fontWeight: 500 }}>
                    {gap.gapCode}
                  </span>
                  <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', flexShrink: 0 }}>
                    {formatRelativeTime(gap.createdAt)}
                  </span>
                </div>
                {gap.summary && (
                  <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', marginTop: '4px', lineHeight: 1.4 }}>
                    {gap.summary}
                  </div>
                )}
                {gap.recommendedAction && (
                  <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-accent)', marginTop: '4px', lineHeight: 1.4 }}>
                    → {gap.recommendedAction}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function formatRelativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return '';
  const diff = Date.now() - then;
  const s = Math.floor(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.floor(h / 24);
  if (d < 30) return `${d}d ago`;
  return new Date(iso).toLocaleDateString();
}

/* ---------------------------------------------------------------
   Tools Tab — equip/unequip MCP connectors and CLI tools per agent
   --------------------------------------------------------------- */

function ToolsTab({ agent }: { agent: Agent }) {
  const [mcpBindings, setMcpBindings] = useState<AgentMcpBinding[]>([]);
  const [toolBindings, setToolBindings] = useState<AgentToolBinding[]>([]);
  const [allConnectors, setAllConnectors] = useState<McpConnectorView[]>([]);
  const [allTools, setAllTools] = useState<CliTool[]>([]);
  const [loading, setLoading] = useState(true);
  const [actionPending, setActionPending] = useState<string | null>(null);

  useEffect(() => {
    loadData();
  }, [agent.id]);

  async function loadData() {
    if (!isTauriRuntime()) {
      setLoading(false);
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const [detail, connectors, tools] = await Promise.all([
        invoke<{ mcpBindings: AgentMcpBinding[]; boundTools: AgentToolBinding[] }>('agent_detail', { agentId: agent.id }),
        invoke<McpConnectorView[]>('mcp_connectors_list'),
        invoke<CliTool[]>('cli_tools_list'),
      ]);
      setMcpBindings(detail.mcpBindings ?? []);
      setToolBindings(detail.boundTools ?? []);
      setAllConnectors(connectors.filter(c => c.enabled));
      setAllTools(tools.filter(t => t.enabled));
    } catch (err) {
      console.error('Failed to load tool inventory', err);
    } finally {
      setLoading(false);
    }
  }

  async function bindMcp(connectorId: string) {
    setActionPending(connectorId);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('agent_bind_mcp', { agentId: agent.id, connectorId });
      await loadData();
    } catch (err) { console.error('bind mcp failed', err); }
    finally { setActionPending(null); }
  }

  async function unbindMcp(connectorId: string) {
    setActionPending(connectorId);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('agent_unbind_mcp', { agentId: agent.id, connectorId });
      await loadData();
    } catch (err) { console.error('unbind mcp failed', err); }
    finally { setActionPending(null); }
  }

  async function bindTool(toolSlug: string) {
    setActionPending(toolSlug);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('agent_bind_tool', { agentId: agent.id, toolSlug });
      await loadData();
    } catch (err) { console.error('bind tool failed', err); }
    finally { setActionPending(null); }
  }

  async function unbindTool(toolSlug: string) {
    setActionPending(toolSlug);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('agent_unbind_tool', { agentId: agent.id, toolSlug });
      await loadData();
    } catch (err) { console.error('unbind tool failed', err); }
    finally { setActionPending(null); }
  }

  if (loading) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 'var(--ws-space-2xl)', color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)' }}>
        Loading tools...
      </div>
    );
  }

  // Browser / mock mode — show the existing read-only view
  if (!isTauriRuntime()) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ws-space-xl)' }}>
        <div>
          <SectionLabel>Equipped Tools</SectionLabel>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: '8px', marginTop: '10px' }}>
            {(agent.tools ?? []).length === 0 ? (
              <span style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)' }}>No tools equipped.</span>
            ) : (
              (agent.tools ?? []).map((tool) => (
                <span key={tool} style={{ fontFamily: 'inherit', fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', border: '1px solid var(--ws-border)', borderRadius: 'var(--ws-radius-sm)', padding: '4px 10px', backgroundColor: 'var(--ws-accent-soft)' }}>
                  {tool}
                </span>
              ))
            )}
          </div>
        </div>
        <div style={{ border: '1px dashed var(--ws-border)', borderRadius: 'var(--ws-radius-md)', padding: 'var(--ws-space-xl)', textAlign: 'center' }}>
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-muted)', margin: 0 }}>Tool inventory coming soon</p>
          <p style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', margin: '6px 0 0' }}>Add MCP servers and CLI tools in Settings, then equip them here.</p>
        </div>
      </div>
    );
  }

  const equippedMcpIds = new Set(mcpBindings.map(b => b.connectorId));
  const equippedToolSlugs = new Set(toolBindings.map(b => b.slug));
  const availableConnectors = allConnectors.filter(c => !equippedMcpIds.has(c.id));
  const availableTools = allTools.filter(t => !equippedToolSlugs.has(t.slug));

  const rowStyle: React.CSSProperties = {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    padding: '8px 12px',
    borderBottom: '1px solid var(--ws-border-soft)',
  };

  const equippedRowStyle: React.CSSProperties = {
    ...rowStyle,
    borderLeft: '2px solid var(--ws-accent)',
    backgroundColor: 'var(--ws-accent-soft)',
  };

  const availableRowStyle: React.CSSProperties = {
    ...rowStyle,
    borderLeft: '2px solid transparent',
  };

  const actionBtnBase: React.CSSProperties = {
    fontFamily: 'inherit',
    fontSize: 'var(--ws-font-xs)',
    border: '1px solid var(--ws-border)',
    borderRadius: 'var(--ws-radius-sm)',
    padding: '3px 10px',
    cursor: 'pointer',
    background: 'none',
    flexShrink: 0,
  };

  const equipBtnStyle: React.CSSProperties = {
    ...actionBtnBase,
    color: 'var(--ws-accent)',
  };

  const unequipBtnStyle: React.CSSProperties = {
    ...actionBtnBase,
    color: 'var(--ws-fg-dim)',
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--ws-space-xl)' }}>

      {/* MCP Connectors */}
      <div>
        <SectionLabel>MCP Connectors</SectionLabel>
        {allConnectors.length === 0 ? (
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-dim)', margin: '10px 0 0' }}>
            Add MCP servers in Settings → MCP Connectors
          </p>
        ) : (
          <div style={{ marginTop: '10px', borderRadius: 'var(--ws-radius-md)', overflow: 'hidden', border: '1px solid var(--ws-border-soft)' }}>
            {mcpBindings.map(binding => (
              <div key={binding.connectorId} style={equippedRowStyle}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px', overflow: 'hidden' }}>
                  <span style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {binding.connectorName}
                  </span>
                  <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', border: '1px solid var(--ws-border)', borderRadius: 'var(--ws-radius-sm)', padding: '1px 6px', flexShrink: 0 }}>
                    {binding.transport}
                  </span>
                </div>
                <button
                  style={unequipBtnStyle}
                  disabled={actionPending === binding.connectorId}
                  onClick={() => unbindMcp(binding.connectorId)}
                >
                  {actionPending === binding.connectorId ? '...' : 'Unequip'}
                </button>
              </div>
            ))}
            {availableConnectors.map(connector => (
              <div key={connector.id} style={availableRowStyle}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px', overflow: 'hidden' }}>
                  <span style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {connector.name}
                  </span>
                  <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', border: '1px solid var(--ws-border-soft)', borderRadius: 'var(--ws-radius-sm)', padding: '1px 6px', flexShrink: 0 }}>
                    {connector.transport}
                  </span>
                </div>
                <button
                  style={equipBtnStyle}
                  disabled={actionPending === connector.id}
                  onClick={() => bindMcp(connector.id)}
                >
                  {actionPending === connector.id ? '...' : 'Equip'}
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* CLI Tools */}
      <div>
        <SectionLabel>CLI Tools</SectionLabel>
        {allTools.length === 0 ? (
          <p style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-dim)', margin: '10px 0 0' }}>
            Add CLI tools in Settings → Tool Inventory
          </p>
        ) : (
          <div style={{ marginTop: '10px', borderRadius: 'var(--ws-radius-md)', overflow: 'hidden', border: '1px solid var(--ws-border-soft)' }}>
            {toolBindings.map(binding => (
              <div key={binding.slug} style={equippedRowStyle}>
                <div style={{ overflow: 'hidden' }}>
                  <div style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {binding.name}
                  </div>
                  {binding.summary && (
                    <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', marginTop: '2px' }}>
                      {binding.summary}
                    </div>
                  )}
                </div>
                <button
                  style={unequipBtnStyle}
                  disabled={actionPending === binding.slug}
                  onClick={() => unbindTool(binding.slug)}
                >
                  {actionPending === binding.slug ? '...' : 'Unequip'}
                </button>
              </div>
            ))}
            {availableTools.map(tool => (
              <div key={tool.slug} style={availableRowStyle}>
                <div style={{ overflow: 'hidden' }}>
                  <div style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {tool.name}
                  </div>
                  {tool.summary && (
                    <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', marginTop: '2px' }}>
                      {tool.summary}
                    </div>
                  )}
                </div>
                <button
                  style={equipBtnStyle}
                  disabled={actionPending === tool.slug}
                  onClick={() => bindTool(tool.slug)}
                >
                  {actionPending === tool.slug ? '...' : 'Equip'}
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

    </div>
  );
}

/* ---------------------------------------------------------------
   Skills Tab
   --------------------------------------------------------------- */

function SkillsTab({ agent }: { agent: Agent }) {
  const [skills, setSkills] = useState<SkillBinding[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadData();
  }, [agent.id]);

  async function loadData() {
    if (!isTauriRuntime()) {
      const fallback = Array.isArray(agent.skills) ? agent.skills : [];
      setSkills(fallback.map((s) => ({
        id: s, name: s, slug: s, summary: '', tags: [], triggerHints: [],
        sourceUri: null, currentVersion: 1, priority: 0, bindingStatus: 'active',
        preselected: false, runtimePath: null,
      })));
      setLoading(false);
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const detail = await invoke<{ boundSkills: SkillBinding[] }>('agent_detail', { agentId: agent.id });
      setSkills(detail.boundSkills ?? []);
    } catch (err) {
      console.error('Failed to load agent skills', err);
    } finally {
      setLoading(false);
    }
  }

  if (loading) {
    return (
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', padding: 'var(--ws-space-2xl)', color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)' }}>
        Loading skills...
      </div>
    );
  }

  if (skills.length === 0) {
    return (
      <div style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)', textAlign: 'center', padding: 'var(--ws-space-2xl)' }}>
        No skills assigned.
      </div>
    );
  }

  return (
    <div>
      <SectionLabel>Assigned Skills</SectionLabel>
      <div style={{ marginTop: '10px', borderRadius: 'var(--ws-radius-md)', overflow: 'hidden', border: '1px solid var(--ws-border-soft)' }}>
        {skills.map((skill) => (
          <div key={skill.id} style={{ padding: '10px 12px', borderBottom: '1px solid var(--ws-border-soft)' }}>
            <div style={{ display: 'flex', alignItems: 'baseline', justifyContent: 'space-between', gap: '10px' }}>
              <span style={{ fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', fontWeight: 500 }}>
                {skill.name}
              </span>
              <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', flexShrink: 0 }}>
                v{skill.currentVersion}
              </span>
            </div>
            {skill.summary && (
              <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', marginTop: '4px', lineHeight: 1.4 }}>
                {skill.summary}
              </div>
            )}
            {skill.tags.length > 0 && (
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '4px', marginTop: '6px' }}>
                {skill.tags.map((tag) => (
                  <span key={tag} style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)', border: '1px solid var(--ws-border-soft)', borderRadius: 'var(--ws-radius-sm)', padding: '1px 6px' }}>
                    {tag}
                  </span>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

/* ---------------------------------------------------------------
   Shared components
   --------------------------------------------------------------- */

function AgentChip({ agent, onClick }: { agent: Agent; onClick?: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: '6px',
        fontFamily: 'inherit',
        fontSize: 'var(--ws-font-sm)',
        color: 'var(--ws-fg-primary)',
        border: '1px solid var(--ws-border)',
        borderRadius: 'var(--ws-radius-sm)',
        padding: '3px 10px',
        backgroundColor: 'var(--ws-bg-elevated)',
        cursor: onClick ? 'pointer' : 'default',
        transition: 'border-color 0.15s ease, background-color 0.15s ease',
      }}
      onMouseEnter={(e) => {
        if (onClick) {
          e.currentTarget.style.borderColor = 'var(--ws-accent)';
          e.currentTarget.style.backgroundColor = 'var(--ws-accent-soft)';
        }
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.borderColor = 'var(--ws-border)';
        e.currentTarget.style.backgroundColor = 'var(--ws-bg-elevated)';
      }}
    >
      <span style={{ fontSize: '0.9em' }}>{agent.icon}</span>
      {agent.name}
      <span style={{ color: 'var(--ws-fg-muted)', fontSize: 'var(--ws-font-xs)' }}>{agent.role}</span>
    </button>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <span style={{ fontFamily: 'inherit', fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', letterSpacing: '0.1em', textTransform: 'uppercase', fontWeight: 600 }}>
      {children}
    </span>
  );
}
