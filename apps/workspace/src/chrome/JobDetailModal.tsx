import { useCallback, useEffect, useMemo, useState } from 'react';
import { motion } from 'motion/react';
import { X, RotateCcw, MessageSquarePlus, ChevronRight, Copy, Check, FileText, Loader2, ChevronDown } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { ActivityLogEntry, Agent, ArtifactPreview, OutboxArtifact, SwoRecord, SwoStatus, TokenUsageRecord } from '../types';
import { TauriBus } from '../sim/tauriBus';

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

function resolveAgentName(agents: Agent[], agentId: string): string {
  return agents.find((a) => a.id === agentId)?.name ?? agentId;
}

/**
 * Build an indented delegation tree from the root SWO down through children.
 */
interface DelegationNode {
  swo: SwoRecord;
  agentName: string;
  depth: number;
  children: DelegationNode[];
}

function buildDelegationTree(
  rootSwoId: string,
  swoMap: Map<string, SwoRecord>,
  agents: Agent[],
): DelegationNode | null {
  const root = swoMap.get(rootSwoId);
  if (!root) return null;

  // Collect all SWOs and group by parentSwoId
  const childMap = new Map<string, SwoRecord[]>();
  for (const swo of swoMap.values()) {
    if (swo.parentSwoId) {
      const existing = childMap.get(swo.parentSwoId) ?? [];
      existing.push(swo);
      childMap.set(swo.parentSwoId, existing);
    }
  }

  function buildNode(swo: SwoRecord, depth: number): DelegationNode {
    const children = (childMap.get(swo.id) ?? [])
      .sort((a, b) => a.createdAt - b.createdAt)
      .map((child) => buildNode(child, depth + 1));

    return {
      swo,
      agentName: resolveAgentName(agents, swo.assigneeId),
      depth,
      children,
    };
  }

  return buildNode(root, 0);
}

function flattenTree(node: DelegationNode): DelegationNode[] {
  const result: DelegationNode[] = [node];
  for (const child of node.children) {
    result.push(...flattenTree(child));
  }
  return result;
}

interface JobDetailModalProps {
  jobId: string;
  swoMap: Map<string, SwoRecord>;
  agents: Agent[];
  activityLog: ActivityLogEntry[];
  onClose: () => void;
  onRerun?: (title: string) => void;
  onRequestRevision?: (swoId: string, feedback: string) => Promise<void>;
  /** Optional bus for loading token usage (only available in Tauri context). */
  bus?: TauriBus;
  /** Artifacts for the root job, keyed by root SWO ID. */
  artifacts?: OutboxArtifact[];
}

export function JobDetailModal({
  jobId,
  swoMap,
  agents,
  activityLog,
  onClose,
  onRerun,
  onRequestRevision,
  bus,
  artifacts,
}: JobDetailModalProps) {
  const [showRevisionInput, setShowRevisionInput] = useState(false);
  const [revisionFeedback, setRevisionFeedback] = useState('');
  const [revisionSubmitting, setRevisionSubmitting] = useState(false);
  const [revisionError, setRevisionError] = useState<string | null>(null);
  const [tokenUsage, setTokenUsage] = useState<TokenUsageRecord[] | null>(null);
  const rootSwo = swoMap.get(jobId);
  const [navStack, setNavStack] = useState<string[]>([]);
  const currentSwoId = navStack.length > 0 ? navStack[navStack.length - 1]! : jobId;
  const currentSwo = swoMap.get(currentSwoId) ?? rootSwo;

  // Load token usage for the current SWO when it changes (Tauri only)
  useEffect(() => {
    if (!bus) return;
    let cancelled = false;
    setTokenUsage(null);
    bus.loadTokenUsageForSwo(currentSwoId).then((records) => {
      if (!cancelled) setTokenUsage(records);
    }).catch(() => {
      if (!cancelled) setTokenUsage([]);
    });
    return () => { cancelled = true; };
  }, [bus, currentSwoId]);

  const tree = useMemo(
    () => (rootSwo ? buildDelegationTree(jobId, swoMap, agents) : null),
    [jobId, swoMap, agents, rootSwo],
  );

  const flatNodes = useMemo(() => (tree ? flattenTree(tree) : []), [tree]);

  /**
   * Build a map from SWO id → set of descendant SWO ids (inclusive of self).
   * Used to scope artifact rendering: when viewing a child, only show files
   * produced by that child or its descendants — not files from sibling
   * branches or the top-level synthesizer.
   */
  const descendantIds = useMemo(() => {
    const map = new Map<string, Set<string>>();
    function collect(node: DelegationNode): Set<string> {
      const set = new Set<string>([node.swo.id]);
      for (const child of node.children) {
        const childSet = collect(child);
        for (const id of childSet) set.add(id);
      }
      map.set(node.swo.id, set);
      return set;
    }
    if (tree) collect(tree);
    return map;
  }, [tree]);

  // Compute the actual parent→child path from root to a given SWO
  const computePathFromRoot = useCallback((targetId: string): string[] => {
    if (targetId === jobId) return [];
    const path: string[] = [];
    let current = swoMap.get(targetId);
    while (current && current.id !== jobId) {
      path.unshift(current.id);
      current = current.parentSwoId ? swoMap.get(current.parentSwoId) : undefined;
    }
    return path;
  }, [jobId, swoMap]);

  // Build breadcrumb path from root to current
  const breadcrumbs = useMemo(() => {
    const path: SwoRecord[] = [];
    if (!rootSwo) return path;
    path.push(rootSwo);
    for (const swoId of navStack) {
      const swo = swoMap.get(swoId);
      if (swo) path.push(swo);
    }
    return path;
  }, [rootSwo, navStack, swoMap]);

  const navigateToSwo = useCallback((swoId: string) => {
    if (swoId === jobId) {
      setNavStack([]);
    } else {
      // Compute the real path from root to this node
      setNavStack(computePathFromRoot(swoId));
    }
  }, [jobId, computePathFromRoot]);

  const navigateToBreadcrumb = useCallback((index: number) => {
    if (index === 0) {
      setNavStack([]);
    } else {
      setNavStack((prev) => prev.slice(0, index));
    }
  }, []);

  // Activity log entries related to the current SWO
  const relatedEntries = useMemo(
    () => activityLog.filter((e) => e.swoId === currentSwoId),
    [activityLog, currentSwoId],
  );

  if (!rootSwo || !currentSwo) return null;

  const badge = STATUS_BADGE[currentSwo.status];

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
        backgroundColor: 'rgb(0 0 0 / 0.82)',
      }}
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: 12 }}
        transition={{ duration: 0.2, ease: 'easeOut' }}
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(860px, 100%)',
          maxHeight: 'calc(100vh - 48px)',
          backgroundColor: 'var(--ws-bg)',
          border: '1px solid var(--ws-border)',
          borderRadius: 'var(--ws-radius-md)',
          boxShadow: 'var(--ws-shadow-overlay)',
          display: 'flex',
          flexDirection: 'column',
          fontFamily: 'monospace',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            padding: 'var(--ws-space-md) var(--ws-space-lg)',
            borderBottom: '1px solid var(--ws-border)',
            backgroundColor: 'var(--ws-bg-elevated)',
            flexShrink: 0,
          }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: '4px', flex: 1, minWidth: 0 }}>
            {/* Breadcrumbs */}
            {breadcrumbs.length > 1 && (
              <div style={{ display: 'flex', alignItems: 'center', gap: '4px', marginBottom: '4px', flexWrap: 'wrap' }}>
                {breadcrumbs.map((crumb, i) => (
                  <span key={crumb.id} style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                    {i > 0 && <ChevronRight size={10} style={{ color: 'var(--ws-fg-dim)' }} />}
                    <button
                      onClick={(e) => { e.stopPropagation(); navigateToBreadcrumb(i); }}
                      style={{
                        fontFamily: 'monospace',
                        fontSize: '0.6rem',
                        color: i < breadcrumbs.length - 1 ? 'rgb(74 222 128)' : 'var(--ws-fg-muted)',
                        background: 'none',
                        border: 'none',
                        cursor: i < breadcrumbs.length - 1 ? 'pointer' : 'default',
                        padding: '2px 4px',
                        letterSpacing: '0.05em',
                        textDecoration: i < breadcrumbs.length - 1 ? 'underline' : 'none',
                        textUnderlineOffset: '2px',
                      }}
                    >
                      {crumb.title.length > 30 ? `${crumb.title.slice(0, 27)}...` : crumb.title}
                    </button>
                  </span>
                ))}
              </div>
            )}
            <div
              style={{
                fontSize: 'var(--ws-font-base)',
                fontWeight: 700,
                color: 'var(--ws-fg-primary)',
                letterSpacing: '0.1em',
                textTransform: 'uppercase',
              }}
            >
              {currentSwo.title}
            </div>
            <div className="flex items-center gap-3">
              <span
                className="text-[10px] px-1.5 py-0.5 rounded-sm font-mono"
                style={{
                  color: badge.color,
                  backgroundColor: badge.color.replace(')', ' / 0.1)'),
                  border: `1px solid ${badge.color.replace(')', ' / 0.3)')}`,
                  letterSpacing: '0.06em',
                }}
              >
                {badge.label.toUpperCase()}
              </span>
              <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)' }}>
                Assigned to {resolveAgentName(agents, currentSwo.assigneeId)}
              </span>
              <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)' }}>
                {formatTimestamp(currentSwo.createdAt)}
              </span>
            </div>
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flexShrink: 0 }}>
            {/* Request Revision button — preserves SWO lineage + delegation tree */}
            {onRequestRevision && currentSwo.status === 'COMPLETED' && !showRevisionInput && (
              <button
                onClick={() => { setShowRevisionInput(true); setRevisionError(null); }}
                style={{
                  fontFamily: 'monospace',
                  fontSize: '0.65rem',
                  letterSpacing: '0.08em',
                  color: 'rgb(251 191 36)',
                  backgroundColor: 'transparent',
                  border: '1px solid rgb(251 191 36 / 0.5)',
                  padding: '4px 10px',
                  cursor: 'pointer',
                  textTransform: 'uppercase',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '5px',
                  transition: 'all 0.15s ease',
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.backgroundColor = 'rgb(251 191 36 / 0.1)';
                  e.currentTarget.style.borderColor = 'rgb(251 191 36)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.backgroundColor = 'transparent';
                  e.currentTarget.style.borderColor = 'rgb(251 191 36 / 0.5)';
                }}
                title="Ask the same agents to revise this deliverable"
              >
                <MessageSquarePlus size={11} />
                REQUEST REVISION
              </button>
            )}

            {/* Re-run button — fresh job with the same title (loses prior context) */}
            {onRerun && currentSwo.status === 'COMPLETED' && !showRevisionInput && (
              <button
                onClick={() => onRerun(currentSwo.title)}
                style={{
                  fontFamily: 'monospace',
                  fontSize: '0.65rem',
                  letterSpacing: '0.08em',
                  color: 'rgb(74 222 128)',
                  backgroundColor: 'transparent',
                  border: '1px solid rgb(34 197 94 / 0.5)',
                  padding: '4px 10px',
                  cursor: 'pointer',
                  textTransform: 'uppercase',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '5px',
                  transition: 'all 0.15s ease',
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.backgroundColor = 'rgb(34 197 94 / 0.1)';
                  e.currentTarget.style.borderColor = 'rgb(74 222 128)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.backgroundColor = 'transparent';
                  e.currentTarget.style.borderColor = 'rgb(34 197 94 / 0.5)';
                }}
                title="Start a fresh job with the same title"
              >
                <RotateCcw size={11} />
                RE-RUN
              </button>
            )}

            {/* Close */}
            <button
              onClick={onClose}
              style={{
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                color: 'rgb(34 197 94 / 0.4)',
                padding: '4px',
                display: 'flex',
                alignItems: 'center',
                transition: 'color 0.15s ease',
              }}
              onMouseEnter={(e) => { e.currentTarget.style.color = 'rgb(74 222 128)'; }}
              onMouseLeave={(e) => { e.currentTarget.style.color = 'rgb(34 197 94 / 0.4)'; }}
            >
              <X size={14} />
            </button>
          </div>
        </div>

        {/* Separator */}
        <div
          style={{
            padding: '0 16px',
            backgroundColor: 'rgb(34 197 94 / 0.03)',
            flexShrink: 0,
            borderBottom: '1px solid rgb(34 197 94 / 0.12)',
          }}
        >
          <span style={{ fontSize: '0.55rem', color: 'rgb(34 197 94 / 0.3)', letterSpacing: '0.05em' }}>
            {'='.repeat(60)}
          </span>
        </div>

        {/* Content */}
        <div style={{ flex: 1, overflow: 'auto', padding: '20px' }}>
          {/* Revision input form — shown when user clicks REQUEST REVISION */}
          {showRevisionInput && onRequestRevision && (
            <div style={{ marginBottom: '24px' }}>
              <SectionLabel>REQUEST REVISION</SectionLabel>
              <div
                style={{
                  marginTop: '8px',
                  padding: '12px 16px',
                  backgroundColor: 'rgb(251 191 36 / 0.04)',
                  border: '1px solid rgb(251 191 36 / 0.25)',
                  borderRadius: 'var(--ws-radius-sm)',
                }}
              >
                <div style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-muted)', marginBottom: '8px', lineHeight: 1.5 }}>
                  Describe what needs to change. The same agents will re-enter the task with your feedback, preserving the delegation tree and prior context.
                </div>
                <textarea
                  autoFocus
                  rows={4}
                  value={revisionFeedback}
                  onChange={(e) => setRevisionFeedback(e.target.value)}
                  placeholder="e.g. The operational section needs more concrete metrics and fewer generic recommendations..."
                  style={{
                    width: '100%',
                    backgroundColor: 'rgb(0 0 0 / 0.4)',
                    border: '1px solid rgb(251 191 36 / 0.3)',
                    borderRadius: 'var(--ws-radius-sm)',
                    padding: '10px 12px',
                    fontFamily: 'monospace',
                    fontSize: 'var(--ws-font-sm)',
                    color: 'var(--ws-fg-primary)',
                    resize: 'vertical',
                    outline: 'none',
                  }}
                  onFocus={(e) => { e.currentTarget.style.borderColor = 'rgb(251 191 36 / 0.6)'; }}
                  onBlur={(e) => { e.currentTarget.style.borderColor = 'rgb(251 191 36 / 0.3)'; }}
                />
                {revisionError && (
                  <div style={{ marginTop: '8px', fontSize: 'var(--ws-font-xs)', color: 'rgb(248 113 113)' }}>
                    {revisionError}
                  </div>
                )}
                <div style={{ display: 'flex', gap: '8px', marginTop: '12px' }}>
                  <button
                    disabled={!revisionFeedback.trim() || revisionSubmitting}
                    onClick={async () => {
                      setRevisionSubmitting(true);
                      setRevisionError(null);
                      try {
                        await onRequestRevision(currentSwo.id, revisionFeedback.trim());
                        setShowRevisionInput(false);
                        setRevisionFeedback('');
                        onClose();
                      } catch (err) {
                        setRevisionError(err instanceof Error ? err.message : 'Revision request failed');
                      } finally {
                        setRevisionSubmitting(false);
                      }
                    }}
                    style={{
                      fontFamily: 'monospace',
                      fontSize: '0.65rem',
                      letterSpacing: '0.08em',
                      color: 'rgb(251 191 36)',
                      backgroundColor: !revisionFeedback.trim() || revisionSubmitting ? 'rgb(251 191 36 / 0.03)' : 'rgb(251 191 36 / 0.12)',
                      border: '1px solid rgb(251 191 36 / 0.5)',
                      padding: '6px 14px',
                      cursor: !revisionFeedback.trim() || revisionSubmitting ? 'not-allowed' : 'pointer',
                      textTransform: 'uppercase',
                      opacity: !revisionFeedback.trim() || revisionSubmitting ? 0.5 : 1,
                      transition: 'all 0.15s ease',
                    }}
                  >
                    {revisionSubmitting ? 'SUBMITTING…' : 'SUBMIT REVISION'}
                  </button>
                  <button
                    disabled={revisionSubmitting}
                    onClick={() => { setShowRevisionInput(false); setRevisionFeedback(''); setRevisionError(null); }}
                    style={{
                      fontFamily: 'monospace',
                      fontSize: '0.65rem',
                      letterSpacing: '0.08em',
                      color: 'var(--ws-fg-muted)',
                      backgroundColor: 'transparent',
                      border: '1px solid rgb(255 255 255 / 0.1)',
                      padding: '6px 14px',
                      cursor: 'pointer',
                      textTransform: 'uppercase',
                      transition: 'all 0.15s ease',
                    }}
                  >
                    CANCEL
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* Previous revision feedback badge */}
          {currentSwo.revisionFeedback && !showRevisionInput && (
            <div style={{ marginBottom: '24px' }}>
              <SectionLabel>PREVIOUS FEEDBACK</SectionLabel>
              <div
                style={{
                  marginTop: '8px',
                  padding: '12px 16px',
                  backgroundColor: 'rgb(251 191 36 / 0.05)',
                  border: '1px solid rgb(251 191 36 / 0.2)',
                  borderRadius: 'var(--ws-radius-sm)',
                  fontSize: 'var(--ws-font-sm)',
                  color: 'rgb(251 191 36 / 0.9)',
                  lineHeight: 1.6,
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                }}
              >
                {currentSwo.revisionFeedback}
              </div>
            </div>
          )}

          {/* Original prompt / request */}
          {(currentSwo.outcome || currentSwo.payload) && (
            <div style={{ marginBottom: '24px' }}>
              <SectionLabel>REQUEST</SectionLabel>
              <div
                style={{
                  marginTop: '8px',
                  padding: '12px 16px',
                  backgroundColor: 'rgb(96 165 250 / 0.04)',
                  border: '1px solid rgb(96 165 250 / 0.15)',
                  borderRadius: 'var(--ws-radius-sm)',
                  fontSize: 'var(--ws-font-sm)',
                  color: 'var(--ws-fg-primary)',
                  lineHeight: '1.6',
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-word',
                }}
              >
                {currentSwo.outcome || currentSwo.payload}
              </div>
            </div>
          )}

          {/* Delegation tree */}
          <div style={{ marginBottom: '24px' }}>
            <SectionLabel>DELEGATION TREE</SectionLabel>
            <div style={{ marginTop: '8px' }}>
              {flatNodes.map((node) => {
                const nodeBadge = STATUS_BADGE[node.swo.status];
                const isActive = node.swo.id === currentSwoId;
                return (
                  <button
                    type="button"
                    key={node.swo.id}
                    onClick={() => navigateToSwo(node.swo.id)}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: '8px',
                      paddingLeft: `${node.depth * 24}px`,
                      paddingTop: '4px',
                      paddingBottom: '4px',
                      width: '100%',
                      background: isActive ? 'rgb(34 197 94 / 0.08)' : 'none',
                      border: 'none',
                      cursor: 'pointer',
                      fontFamily: 'monospace',
                      textAlign: 'left',
                      borderRadius: 'var(--ws-radius-sm)',
                      transition: 'background 0.15s ease',
                    }}
                    onMouseEnter={(e) => { if (!isActive) e.currentTarget.style.background = 'rgb(255 255 255 / 0.03)'; }}
                    onMouseLeave={(e) => { if (!isActive) e.currentTarget.style.background = 'none'; }}
                  >
                    {/* Depth indicator */}
                    {node.depth > 0 && (
                      <span style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-xs)' }}>
                        {'  '.repeat(node.depth - 1)}+-
                      </span>
                    )}
                    {/* Agent name */}
                    <span style={{ color: isActive ? 'rgb(74 222 128)' : 'rgb(74 222 128 / 0.7)', fontSize: 'var(--ws-font-sm)', fontWeight: isActive ? 700 : 600 }}>
                      {node.agentName}
                    </span>
                    {/* Task */}
                    <span style={{ color: 'var(--ws-fg-muted)', fontSize: 'var(--ws-font-xs)', flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {node.swo.title}
                    </span>
                    {/* Status badge */}
                    <span
                      className="text-[10px] px-1 py-0.5 rounded-sm"
                      style={{
                        color: nodeBadge.color,
                        backgroundColor: nodeBadge.color.replace(')', ' / 0.08)'),
                        flexShrink: 0,
                      }}
                    >
                      {nodeBadge.label}
                    </span>
                    {/* Timestamp */}
                    <span style={{ color: 'var(--ws-fg-dim)', fontSize: '0.6rem', flexShrink: 0, fontVariantNumeric: 'tabular-nums' }}>
                      {formatTimestamp(node.swo.createdAt)}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Deliverable content (direct answer / review response) */}
          {currentSwo.reviewResponse && (
            <div style={{ marginBottom: '24px' }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <SectionLabel>DELIVERABLE</SectionLabel>
                <CopyButton text={currentSwo.reviewResponse} />
              </div>
              <div
                className="ws-markdown"
                style={{
                  marginTop: '8px',
                  padding: '12px 16px',
                  backgroundColor: 'rgb(34 197 94 / 0.04)',
                  border: '1px solid rgb(34 197 94 / 0.15)',
                  borderRadius: 'var(--ws-radius-sm)',
                  fontSize: 'var(--ws-font-sm)',
                  color: 'var(--ws-fg-primary)',
                  lineHeight: '1.6',
                  wordBreak: 'break-word',
                  maxHeight: '400px',
                  overflowY: 'auto',
                  scrollbarWidth: 'thin',
                  scrollbarColor: 'rgb(34 197 94 / 0.2) transparent',
                }}
              >
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{currentSwo.reviewResponse}</ReactMarkdown>
              </div>
            </div>
          )}

          {/* Subordinate deliverables — show each direct child's deliverable
              (text reviewResponse and/or artifact files) so the user sees the
              raw inputs to the synthesis. Every direct child gets a card
              regardless of content so drill-in is always available. Artifacts
              are attributed to a child if they were produced by the child OR
              any of its descendants — this captures dynamically hired agents
              that produce files under a child's subtree. */}
          {(() => {
            const directChildren = flatNodes
              .filter((n) => n.swo.parentSwoId === currentSwo.id && n.swo.id !== currentSwo.id);
            if (directChildren.length === 0) return null;
            return (
              <div style={{ marginBottom: '24px' }}>
                <SectionLabel>SUBORDINATE DELIVERABLES</SectionLabel>
                <div style={{ marginTop: '8px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                  {directChildren.map((node, index) => {
                    const childSubtree = descendantIds.get(node.swo.id) ?? new Set([node.swo.id]);
                    const childArtifacts = (artifacts ?? []).filter((a) => {
                      if (a.swoId == null) return false;
                      return childSubtree.has(String(a.swoId));
                    });
                    const hasSubChildren = node.children.length > 0;
                    return (
                      <SubordinateDeliverableCard
                        key={node.swo.id}
                        node={node}
                        artifacts={childArtifacts}
                        bus={bus}
                        onDrillIn={() => navigateToSwo(node.swo.id)}
                        defaultExpanded={index === 0}
                        hasSubChildren={hasSubChildren}
                      />
                    );
                  })}
                </div>
              </div>
            );
          })()}

          {/* Artifact files — scope to the currently-viewed SWO's subtree.
              On the root view this shows everything (since the root's subtree
              is the whole tree). On a child view it shows only files produced
              by that child or its descendants, not sibling-branch files.
              Single-file case auto-expands so leaf deliverables are visible
              immediately. */}
          {(() => {
            const currentSubtree = descendantIds.get(currentSwo.id) ?? new Set([currentSwo.id]);
            const scopedArtifacts = (artifacts ?? []).filter((a) => {
              if (a.swoId == null) return true; // unattributed files: always show
              return currentSubtree.has(String(a.swoId));
            });
            if (scopedArtifacts.length === 0) return null;
            return (
              <div style={{ marginBottom: '24px' }}>
                <SectionLabel>FILES</SectionLabel>
                <div style={{ marginTop: '8px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                  {scopedArtifacts.map((a) => (
                    <InlineArtifactCard
                      key={a.id}
                      artifact={a}
                      bus={bus}
                      defaultExpanded={scopedArtifacts.length === 1}
                    />
                  ))}
                </div>
              </div>
            );
          })()}

          {/* Token usage for this SWO */}
          {bus && (
            <div style={{ marginBottom: '24px' }}>
              <SectionLabel>USAGE</SectionLabel>
              <div
                style={{
                  marginTop: '8px',
                  padding: '12px 16px',
                  backgroundColor: 'rgb(168 85 247 / 0.04)',
                  border: '1px solid rgb(168 85 247 / 0.15)',
                  borderRadius: 'var(--ws-radius-sm)',
                  fontFamily: 'monospace',
                  fontSize: 'var(--ws-font-xs)',
                }}
              >
                {tokenUsage === null ? (
                  <span style={{ color: 'var(--ws-fg-dim)' }}>Loading…</span>
                ) : tokenUsage.length === 0 ? (
                  <span style={{ color: 'var(--ws-fg-dim)' }}>No token data available.</span>
                ) : (
                  <TokenUsageSummary records={tokenUsage} />
                )}
              </div>
            </div>
          )}

          {/* Timeline — built from SWO data + activity log, deduplicated */}
          <div>
            <SectionLabel>TIMELINE</SectionLabel>
            <div style={{ marginTop: '8px' }}>
              {(() => {
                // Build timeline from SWO state + activity log
                const timelineEntries: Array<{ key: string; time: number; agent: string; text: string }> = [];

                // Created event from SWO itself
                timelineEntries.push({
                  key: `created-${currentSwo.id}`,
                  time: currentSwo.createdAt,
                  agent: resolveAgentName(agents, currentSwo.assigneeId),
                  text: 'Job started',
                });

                // Completion event
                if (currentSwo.status === 'COMPLETED') {
                  timelineEntries.push({
                    key: `completed-${currentSwo.id}`,
                    time: currentSwo.updatedAt,
                    agent: resolveAgentName(agents, currentSwo.assigneeId),
                    text: 'Job completed',
                  });
                }
                if (currentSwo.status === 'BLOCKED') {
                  timelineEntries.push({
                    key: `blocked-${currentSwo.id}`,
                    time: currentSwo.updatedAt,
                    agent: resolveAgentName(agents, currentSwo.assigneeId),
                    text: 'Job blocked',
                  });
                }

                // Child delegation events
                for (const node of flatNodes) {
                  if (node.swo.parentSwoId === currentSwo.id) {
                    timelineEntries.push({
                      key: `delegated-${node.swo.id}`,
                      time: node.swo.createdAt,
                      agent: node.agentName,
                      text: `Delegated: ${node.swo.title}`,
                    });
                    if (node.swo.status === 'COMPLETED') {
                      timelineEntries.push({
                        key: `child-done-${node.swo.id}`,
                        time: node.swo.updatedAt,
                        agent: node.agentName,
                        text: `Completed: ${node.swo.title}`,
                      });
                    }
                  }
                }

                // Deduplicated activity log entries not already covered
                const seen = new Set(timelineEntries.map((e) => e.key));
                for (const entry of relatedEntries) {
                  const dedupKey = `${entry.kind}-${entry.agentId}-${Math.floor(entry.timestamp / 2000)}`;
                  if (seen.has(dedupKey)) continue;
                  seen.add(dedupKey);
                  timelineEntries.push({
                    key: entry.id,
                    time: entry.timestamp,
                    agent: entry.agentName,
                    text: entry.summary,
                  });
                }

                timelineEntries.sort((a, b) => a.time - b.time);

                if (timelineEntries.length === 0) {
                  return (
                    <span style={{ fontSize: 'var(--ws-font-xs)', color: 'var(--ws-fg-dim)' }}>
                      No timeline events recorded.
                    </span>
                  );
                }

                return timelineEntries.map((entry) => (
                  <div
                    key={entry.key}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: '8px',
                      paddingTop: '3px',
                      paddingBottom: '3px',
                      borderBottom: '1px solid rgb(255 255 255 / 0.03)',
                    }}
                  >
                    <span style={{ color: 'var(--ws-fg-dim)', fontSize: '0.6rem', fontVariantNumeric: 'tabular-nums', flexShrink: 0 }}>
                      {formatTimestamp(entry.time)}
                    </span>
                    <span style={{ color: 'rgb(74 222 128)', fontSize: 'var(--ws-font-xs)' }}>
                      {entry.agent}
                    </span>
                    <span style={{ color: 'var(--ws-fg-muted)', fontSize: 'var(--ws-font-xs)' }}>
                      {entry.text}
                    </span>
                  </div>
                ));
              })()}
            </div>
          </div>
        </div>

        {/* Footer */}
        <div
          style={{
            padding: '6px 16px',
            borderTop: '1px solid rgb(34 197 94 / 0.15)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            backgroundColor: 'rgb(34 197 94 / 0.02)',
            flexShrink: 0,
          }}
        >
          <span style={{ fontSize: '0.55rem', color: 'rgb(34 197 94 / 0.25)', letterSpacing: '0.08em' }}>
            {flatNodes.length} TASK{flatNodes.length !== 1 ? 'S' : ''} IN TREE
          </span>
          <span style={{ fontSize: '0.55rem', color: 'rgb(34 197 94 / 0.25)', letterSpacing: '0.08em' }}>
            ESC / CLICK OUTSIDE TO CLOSE
          </span>
        </div>
      </motion.div>
    </motion.div>
  );
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        void navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 2000);
        });
      }}
      style={{
        fontFamily: 'monospace',
        fontSize: '0.6rem',
        letterSpacing: '0.06em',
        color: copied ? 'rgb(74 222 128)' : 'var(--ws-fg-dim)',
        backgroundColor: 'transparent',
        border: '1px solid rgb(255 255 255 / 0.1)',
        padding: '3px 8px',
        cursor: 'pointer',
        display: 'flex',
        alignItems: 'center',
        gap: '4px',
        borderRadius: 'var(--ws-radius-sm)',
        transition: 'all 0.15s ease',
      }}
    >
      {copied ? <Check size={10} /> : <Copy size={10} />}
      {copied ? 'COPIED' : 'COPY'}
    </button>
  );
}

function formatTokenCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function TokenUsageSummary({ records }: { records: TokenUsageRecord[] }) {
  const totalInput = records.reduce((s, r) => s + r.inputTokens, 0);
  const totalOutput = records.reduce((s, r) => s + r.outputTokens, 0);
  const totalCacheRead = records.reduce((s, r) => s + r.cacheReadTokens, 0);
  const totalCost = records.reduce((s, r) => s + (r.costUsd ?? 0), 0);
  const hasCost = records.some((r) => r.costUsd !== null);

  const rowStyle: React.CSSProperties = {
    display: 'flex',
    justifyContent: 'space-between',
    paddingTop: '3px',
    paddingBottom: '3px',
    borderBottom: '1px solid rgb(255 255 255 / 0.04)',
  };
  const labelStyle: React.CSSProperties = { color: 'var(--ws-fg-muted)', letterSpacing: '0.06em' };
  const valueStyle: React.CSSProperties = { color: 'var(--ws-fg-primary)', fontVariantNumeric: 'tabular-nums' };

  return (
    <div>
      <div style={rowStyle}>
        <span style={labelStyle}>INPUT TOKENS</span>
        <span style={valueStyle}>{formatTokenCount(totalInput)}</span>
      </div>
      <div style={rowStyle}>
        <span style={labelStyle}>OUTPUT TOKENS</span>
        <span style={valueStyle}>{formatTokenCount(totalOutput)}</span>
      </div>
      {totalCacheRead > 0 && (
        <div style={rowStyle}>
          <span style={labelStyle}>CACHE HITS</span>
          <span style={{ ...valueStyle, color: 'rgb(74 222 128 / 0.8)' }}>{formatTokenCount(totalCacheRead)}</span>
        </div>
      )}
      {hasCost && (
        <div style={{ ...rowStyle, borderBottom: 'none' }}>
          <span style={labelStyle}>EST. COST</span>
          <span style={{ ...valueStyle, color: 'rgb(251 191 36)' }}>${totalCost.toFixed(4)}</span>
        </div>
      )}
      {records.length > 1 && (
        <div style={{ marginTop: '6px', color: 'var(--ws-fg-dim)', fontSize: '0.6rem' }}>
          {records.length} run{records.length !== 1 ? 's' : ''} · models: {[...new Set(records.map((r) => r.model))].join(', ')}
        </div>
      )}
    </div>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{
        fontFamily: 'monospace',
        fontSize: 'var(--ws-font-xs)',
        color: 'var(--ws-fg-muted)',
        letterSpacing: '0.15em',
        textTransform: 'uppercase',
      }}
    >
      {children}
    </span>
  );
}

// ---------------------------------------------------------------------------
// SubordinateDeliverableCard — collapsible accordion card showing one child
// SWO's reviewResponse inline. First child defaults expanded, the rest
// collapsed so long delegation chains are scannable. A drill-in chevron
// button jumps the modal to the child's full detail view.
// ---------------------------------------------------------------------------

function SubordinateDeliverableCard({
  node,
  artifacts = [],
  bus,
  onDrillIn,
  defaultExpanded = false,
  hasSubChildren = false,
}: {
  node: DelegationNode;
  artifacts?: OutboxArtifact[];
  bus?: TauriBus;
  onDrillIn: () => void;
  defaultExpanded?: boolean;
  hasSubChildren?: boolean;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [copied, setCopied] = useState(false);
  const response = node.swo.reviewResponse ?? '';
  // Show first ~120 chars of the deliverable text in the collapsed header
  // so the user can scan content without opening each card.
  const previewText = response.trim().length > 0
    ? (response.length > 120 ? `${response.slice(0, 117).replace(/\s+/g, ' ')}…` : response.replace(/\s+/g, ' '))
    : null;
  const hasFiles = artifacts.length > 0;
  const hasText = response.trim().length > 0;

  // Decision badge: DELEGATE if this child spawned sub-agents, ANSWER if leaf.
  const decisionBadge = hasSubChildren
    ? { label: 'DELEGATE', color: 'rgb(96 165 250)' }
    : { label: 'ANSWER', color: 'rgb(74 222 128)' };

  return (
    <div
      style={{
        backgroundColor: 'rgb(34 197 94 / 0.03)',
        border: '1px solid rgb(34 197 94 / 0.15)',
        borderRadius: 'var(--ws-radius-sm)',
        overflow: 'hidden',
      }}
    >
      {/* Accordion header — click anywhere to expand/collapse */}
      <div
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          padding: '9px 12px',
          cursor: 'pointer',
          userSelect: 'none',
        }}
        onClick={() => setExpanded((v) => !v)}
        onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setExpanded((v) => !v); } }}
      >
        {/* Expand/collapse chevron */}
        <ChevronRight
          size={12}
          style={{
            color: 'rgb(74 222 128)',
            transform: expanded ? 'rotate(90deg)' : 'rotate(0deg)',
            transition: 'transform 0.15s ease',
            flexShrink: 0,
          }}
        />
        {/* Agent name */}
        <span
          style={{
            fontFamily: 'monospace',
            fontSize: 'var(--ws-font-sm)',
            fontWeight: 600,
            color: 'rgb(74 222 128)',
            flexShrink: 0,
          }}
        >
          {node.agentName}
        </span>
        {/* Decision badge */}
        <span
          style={{
            fontFamily: 'monospace',
            fontSize: '0.55rem',
            letterSpacing: '0.06em',
            color: decisionBadge.color,
            backgroundColor: decisionBadge.color.replace(')', ' / 0.1)'),
            border: `1px solid ${decisionBadge.color.replace(')', ' / 0.3)')}`,
            padding: '1px 5px',
            borderRadius: '3px',
            flexShrink: 0,
          }}
        >
          {decisionBadge.label}
        </span>
        {/* Collapsed: show task title + first ~120 chars of deliverable preview.
            Expanded: show task title only. */}
        <span
          style={{
            fontFamily: 'monospace',
            fontSize: 'var(--ws-font-xs)',
            color: expanded ? 'var(--ws-fg-muted)' : 'var(--ws-fg-primary)',
            flex: 1,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {expanded
            ? node.swo.title
            : (previewText ?? node.swo.title)}
        </span>
        {/* Drill-in button — navigate to child's full view */}
        <button
          onClick={(e) => { e.stopPropagation(); onDrillIn(); }}
          title="Open full detail for this subtask"
          style={{
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            color: 'rgb(34 197 94 / 0.5)',
            padding: '2px',
            display: 'flex',
            alignItems: 'center',
            flexShrink: 0,
          }}
          onMouseEnter={(e) => { e.currentTarget.style.color = 'rgb(74 222 128)'; }}
          onMouseLeave={(e) => { e.currentTarget.style.color = 'rgb(34 197 94 / 0.5)'; }}
        >
          <ChevronRight size={14} />
        </button>
      </div>
      {expanded && (
        <div
          style={{
            borderTop: '1px solid rgb(34 197 94 / 0.12)',
            padding: '12px 14px',
            backgroundColor: 'rgb(0 0 0 / 0.18)',
            display: 'flex',
            flexDirection: 'column',
            gap: '12px',
          }}
        >
          {hasText && (
            <div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: '6px' }}>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    void navigator.clipboard.writeText(response).then(() => {
                      setCopied(true);
                      setTimeout(() => setCopied(false), 1500);
                    });
                  }}
                  style={{
                    fontFamily: 'monospace',
                    fontSize: '0.55rem',
                    letterSpacing: '0.06em',
                    color: copied ? 'rgb(74 222 128)' : 'var(--ws-fg-dim)',
                    backgroundColor: 'transparent',
                    border: '1px solid rgb(255 255 255 / 0.08)',
                    padding: '2px 6px',
                    cursor: 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    gap: '4px',
                    borderRadius: '3px',
                  }}
                >
                  {copied ? <Check size={9} /> : <Copy size={9} />}
                  {copied ? 'COPIED' : 'COPY'}
                </button>
              </div>
              <div
                className="ws-markdown"
                style={{
                  fontSize: 'var(--ws-font-sm)',
                  color: 'var(--ws-fg-primary)',
                  lineHeight: 1.55,
                  wordBreak: 'break-word',
                  maxHeight: '320px',
                  overflowY: 'auto',
                  scrollbarWidth: 'thin',
                  scrollbarColor: 'rgb(34 197 94 / 0.2) transparent',
                }}
              >
                <ReactMarkdown remarkPlugins={[remarkGfm]}>{response}</ReactMarkdown>
              </div>
            </div>
          )}
          {hasFiles && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
              {!hasText && (
                <div style={{ fontSize: '0.55rem', color: 'var(--ws-fg-dim)', letterSpacing: '0.06em', textTransform: 'uppercase' }}>
                  Deliverable file{artifacts.length > 1 ? 's' : ''}
                </div>
              )}
              {artifacts.map((a) => (
                <InlineArtifactCard
                  key={a.id}
                  artifact={a}
                  bus={bus}
                  defaultExpanded={!hasText && artifacts.length === 1}
                />
              ))}
            </div>
          )}
          {!hasText && !hasFiles && (
            <div
              style={{
                fontSize: 'var(--ws-font-xs)',
                color: 'var(--ws-fg-dim)',
                fontStyle: 'italic',
                padding: '6px 0',
              }}
            >
              No inline preview available. Click the chevron on the right to open this subtask's full detail view.
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// InlineArtifactCard — expandable artifact file preview
// ---------------------------------------------------------------------------

type PreviewState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'loaded'; preview: ArtifactPreview }
  | { status: 'error'; message: string };

function formatBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${n} B`;
}

function InlineArtifactCard({
  artifact,
  bus,
  defaultExpanded = false,
}: {
  artifact: OutboxArtifact;
  bus?: TauriBus;
  defaultExpanded?: boolean;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [previewState, setPreviewState] = useState<PreviewState>({ status: 'idle' });
  const [copied, setCopied] = useState(false);

  // If defaultExpanded, fetch the preview on mount.
  useEffect(() => {
    if (defaultExpanded && previewState.status === 'idle' && bus) {
      setPreviewState({ status: 'loading' });
      bus.previewArtifact(artifact.id)
        .then((preview) => setPreviewState({ status: 'loaded', preview }))
        .catch((err) => setPreviewState({
          status: 'error',
          message: err instanceof Error ? err.message : 'Failed to load preview.',
        }));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleToggle = useCallback(async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!expanded) {
      setExpanded(true);
      if (previewState.status === 'idle') {
        if (!bus) {
          setPreviewState({ status: 'error', message: 'Preview requires a Tauri kernel connection.' });
          return;
        }
        setPreviewState({ status: 'loading' });
        try {
          const preview = await bus.previewArtifact(artifact.id);
          setPreviewState({ status: 'loaded', preview });
        } catch (err) {
          setPreviewState({
            status: 'error',
            message: err instanceof Error ? err.message : 'Failed to load preview.',
          });
        }
      }
    } else {
      setExpanded(false);
    }
  }, [expanded, previewState.status, bus, artifact.id]);

  const handleCopy = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    if (previewState.status !== 'loaded') return;
    void navigator.clipboard.writeText(previewState.preview.content).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }, [previewState]);

  return (
    <div
      style={{
        border: '1px solid var(--ws-border)',
        borderRadius: 'var(--ws-radius-sm)',
        backgroundColor: 'rgb(34 197 94 / 0.04)',
        overflow: 'hidden',
      }}
    >
      {/* Header row */}
      <button
        type="button"
        onClick={(e) => { void handleToggle(e); }}
        style={{
          width: '100%',
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          padding: '8px 12px',
          background: 'none',
          border: 'none',
          cursor: 'pointer',
          fontFamily: 'monospace',
          textAlign: 'left',
        }}
      >
        <FileText size={12} style={{ color: 'rgb(74 222 128 / 0.7)', flexShrink: 0 }} />
        <span style={{ flex: 1, fontSize: 'var(--ws-font-sm)', color: 'var(--ws-fg-primary)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {artifact.filename}
        </span>
        {previewState.status === 'loaded' && (
          <span style={{ fontSize: '0.6rem', color: 'var(--ws-fg-dim)', flexShrink: 0 }}>
            {formatBytes(previewState.preview.sizeBytes)}
            {previewState.preview.truncated ? ' (truncated)' : ''}
          </span>
        )}
        {previewState.status === 'loading' ? (
          <Loader2 size={12} style={{ color: 'var(--ws-fg-dim)', animation: 'spin 1s linear infinite', flexShrink: 0 }} />
        ) : (
          expanded
            ? <ChevronDown size={12} style={{ color: 'var(--ws-fg-dim)', flexShrink: 0 }} />
            : <ChevronDown size={12} style={{ color: 'var(--ws-fg-dim)', flexShrink: 0, transform: 'rotate(-90deg)' }} />
        )}
      </button>

      {/* Expanded content */}
      {expanded && (
        <div style={{ borderTop: '1px solid var(--ws-border)' }}>
          {previewState.status === 'loading' && (
            <div style={{ padding: '12px 16px', color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-xs)' }}>
              Loading preview…
            </div>
          )}
          {previewState.status === 'error' && (
            <div style={{ padding: '12px 16px', color: 'rgb(248 113 113)', fontSize: 'var(--ws-font-xs)' }}>
              {previewState.message}
            </div>
          )}
          {previewState.status === 'loaded' && (
            <>
              <div style={{ display: 'flex', justifyContent: 'flex-end', padding: '4px 8px', borderBottom: '1px solid rgb(255 255 255 / 0.05)' }}>
                <button
                  onClick={handleCopy}
                  style={{
                    fontFamily: 'monospace',
                    fontSize: '0.6rem',
                    letterSpacing: '0.06em',
                    color: copied ? 'rgb(74 222 128)' : 'var(--ws-fg-dim)',
                    backgroundColor: 'transparent',
                    border: '1px solid rgb(255 255 255 / 0.1)',
                    padding: '2px 7px',
                    cursor: 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    gap: '4px',
                    borderRadius: 'var(--ws-radius-sm)',
                  }}
                >
                  {copied ? <Check size={10} /> : <Copy size={10} />}
                  {copied ? 'COPIED' : 'COPY'}
                </button>
              </div>
              {previewState.preview.renderMode === 'markdown' ? (
                <div
                  className="ws-markdown"
                  style={{
                    padding: '12px 16px',
                    fontSize: 'var(--ws-font-sm)',
                    color: 'var(--ws-fg-primary)',
                    lineHeight: '1.6',
                    wordBreak: 'break-word',
                    maxHeight: '480px',
                    overflowY: 'auto',
                    scrollbarWidth: 'thin',
                    scrollbarColor: 'rgb(34 197 94 / 0.2) transparent',
                  }}
                >
                  <ReactMarkdown remarkPlugins={[remarkGfm]}>{previewState.preview.content}</ReactMarkdown>
                </div>
              ) : previewState.preview.renderMode === 'binary' ? (
                <div style={{ padding: '12px 16px', color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-xs)' }}>
                  Binary file — no preview available.
                </div>
              ) : (
                <pre
                  style={{
                    margin: 0,
                    padding: '12px 16px',
                    fontFamily: 'monospace',
                    fontSize: 'var(--ws-font-xs)',
                    color: 'var(--ws-fg-primary)',
                    lineHeight: '1.5',
                    overflowX: 'auto',
                    overflowY: 'auto',
                    maxHeight: '480px',
                    whiteSpace: 'pre-wrap',
                    wordBreak: 'break-all',
                    scrollbarWidth: 'thin',
                    scrollbarColor: 'rgb(34 197 94 / 0.2) transparent',
                  }}
                >
                  {previewState.preview.content}
                </pre>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}
