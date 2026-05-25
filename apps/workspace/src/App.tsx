import { useState, useCallback, useEffect } from 'react';
import { MockBus } from './sim/mockBus';
import { TauriBus } from './sim/tauriBus';
import { isTauriRuntime } from './sim/platform';
import { runHappyPath } from './sim/mockScenarios';
import { useWorkspaceState } from './world/useWorkspaceState';
import type { Agent, Bus } from './types';
import { Header } from './chrome/Header';
import { SubmitJobDialog } from './chrome/SubmitJobDialog';
import { AgentCardModal, type AgentSavedFields } from './chrome/AgentCardModal';
import { ArtifactViewer } from './chrome/ArtifactViewer';
import { ActivityLog } from './chrome/ActivityLog';
import { JobHistoryPanel } from './chrome/JobHistoryPanel';
import { JobDetailModal } from './chrome/JobDetailModal';
import { DevToolbar } from './chrome/DevToolbar';
import { Settings } from './chrome/Settings';
import { Onboarding } from './chrome/Onboarding';
import { useSkin } from './render/useSkin';
import { agents as mockAgents } from './sim/mockRoster';

// Create a single bus instance per runtime mode
const mockBus = new MockBus();
const tauriBus = new TauriBus();

function getActiveBus(): { bus: Bus; isTauri: boolean; tauriBusInstance?: TauriBus } {
  if (isTauriRuntime()) {
    return { bus: tauriBus, isTauri: true, tauriBusInstance: tauriBus };
  }
  return { bus: mockBus, isTauri: false };
}

const ONBOARDING_KEY = 'sairgent_onboarding_complete';

function hasCompletedOnboarding(): boolean {
  if (isTauriRuntime()) {
    // In Tauri mode, onboarding status is checked via secrets_status
    // but we also track locally for immediate UI state
    return localStorage.getItem(ONBOARDING_KEY) === 'true';
  }
  return localStorage.getItem(ONBOARDING_KEY) === 'true';
}

export default function App() {
  const [worldKey, setWorldKey] = useState(0);

  return (
    <div className="h-screen flex flex-col font-mono" style={{ backgroundColor: 'var(--ws-bg)', color: 'var(--ws-fg-primary)' }}>
      <WorkspaceShell key={worldKey} onReset={() => setWorldKey((k) => k + 1)} />
    </div>
  );
}

function WorkspaceShell({ onReset }: { onReset: () => void }) {
  const { bus, isTauri, tauriBusInstance } = getActiveBus();
  const [kernelAgents, setKernelAgents] = useState<Agent[] | undefined>(undefined);
  const [tauriReady, setTauriReady] = useState(!isTauri); // Mock mode is ready immediately
  const [onboardingComplete, setOnboardingComplete] = useState(hasCompletedOnboarding());
  const [checkingSecrets, setCheckingSecrets] = useState(isTauri);
  const [bootAttempt, setBootAttempt] = useState(0);

  // Check if we have stored credentials on mount (Tauri mode)
  useEffect(() => {
    if (!isTauri) {
      setCheckingSecrets(false);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const hasSecrets = await invoke<boolean>('secrets_status');
        if (!cancelled) {
          if (hasSecrets) {
            setOnboardingComplete(true);
            localStorage.setItem(ONBOARDING_KEY, 'true');
          }
          setCheckingSecrets(false);
        }
      } catch {
        if (!cancelled) setCheckingSecrets(false);
      }
    })();
    return () => { cancelled = true; };
  }, [isTauri]);

  // Initialize Tauri bus -- re-runs when bootAttempt increments (after saving a new key)
  useEffect(() => {
    if (!tauriBusInstance) return;
    if (!onboardingComplete) return;
    let cancelled = false;
    setTauriReady(false);
    tauriBusInstance.init().then((bootstrap) => {
      if (cancelled) return;
      setKernelAgents(bootstrap.agents);
      setTauriReady(true);
    }).catch((err) => {
      console.error('[workspace] Tauri bootstrap failed, falling back to mock:', err);
      setTauriReady(true); // Fall through to mock roster
    });
    return () => {
      cancelled = true;
    };
  }, [tauriBusInstance, onboardingComplete, bootAttempt]);

  // Called after saving a new API key in Settings -- triggers kernel re-boot
  const retryKernelBoot = useCallback(() => {
    if (tauriBusInstance) {
      tauriBusInstance.dispose(); // Reset the bus so init() can run fresh
    }
    setBootAttempt((n) => n + 1);
  }, [tauriBusInstance]);

  const world = useWorkspaceState(bus, kernelAgents);
  const { skin, loading: skinLoading, setSkin, availableSkins } = useSkin();
  const [selectedAgent, setSelectedAgent] = useState<string | null>(null);
  const [selectedInboxItemId, setSelectedInboxItemId] = useState<string | null>(null);
  const [submitOpen, setSubmitOpen] = useState(false);
  const [activityLogOpen, setActivityLogOpen] = useState(false);
  const [jobHistoryOpen, setJobHistoryOpen] = useState(false);
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Track which completed jobs have been viewed (for unread dots)
  // Persisted to localStorage so state survives restarts
  const [viewedJobIds, setViewedJobIds] = useState<Set<string>>(() => {
    try {
      const stored = localStorage.getItem('sairgent_viewed_jobs');
      return stored ? new Set(JSON.parse(stored) as string[]) : new Set();
    } catch { return new Set(); }
  });
  // Persist viewed job IDs to localStorage when they change
  useEffect(() => {
    try { localStorage.setItem('sairgent_viewed_jobs', JSON.stringify([...viewedJobIds])); }
    catch { /* quota exceeded — ignore */ }
  }, [viewedJobIds]);
  const unreadCompletedCount = world.jobs.filter(
    (j) => j.status === 'COMPLETED' && !viewedJobIds.has(j.id),
  ).length;

  // Resolve agents for the job detail modal
  const resolvedAgents = kernelAgents ?? mockAgents;

  // Keyboard shortcut: Cmd/Ctrl+, to open settings
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        setSettingsOpen((prev) => !prev);
      }
    }
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  const handleSubmitJob = useCallback(
    (title: string, description: string) => {
      console.log('[workspace] handleSubmitJob called:', { title, description, isTauri, hasBus: !!tauriBusInstance });
      if (isTauri && tauriBusInstance) {
        tauriBusInstance.submitJob(title, description).catch((err) => {
          console.error('[workspace] Job submission failed:', err);
        });
      } else {
        console.log('[workspace] Using mock mode');
        runHappyPath(bus as MockBus);
      }
    },
    [bus, isTauri, tauriBusInstance],
  );

  const handleRerun = useCallback(
    (title: string) => {
      handleSubmitJob(title, '');
      setSelectedJobId(null);
      setJobHistoryOpen(false);
    },
    [handleSubmitJob],
  );

  const handleCancelJob = useCallback(
    (jobId: string) => {
      if (isTauri && tauriBusInstance) {
        tauriBusInstance.cancelJob(jobId).catch((err) => {
          console.error('[workspace] Job cancellation failed:', err);
        });
      }
    },
    [isTauri, tauriBusInstance],
  );

  const handleRequestRevision = useCallback(
    async (jobId: string, feedback: string) => {
      if (!isTauri || !tauriBusInstance) {
        console.warn('[workspace] Revision requests require a real kernel bus');
        return;
      }
      try {
        await tauriBusInstance.requestRevision(jobId, feedback);
      } catch (err) {
        console.error('[workspace] Revision request failed:', err);
        throw err;
      }
    },
    [isTauri, tauriBusInstance],
  );

  const handleInboxClick = useCallback(() => {
    setJobHistoryOpen(true);
  }, []);

  const handleOnboardingComplete = useCallback(() => {
    localStorage.setItem(ONBOARDING_KEY, 'true');
    setOnboardingComplete(true);
  }, []);

  const openJobDetail = useCallback((jobId: string) => {
    setSelectedJobId(jobId);
    setViewedJobIds((prev) => new Set(prev).add(jobId));
  }, []);

  // Navigate from activity log entry to job detail
  const handleActivityEntryClick = useCallback((swoId: string) => {
    openJobDetail(swoId);
    setActivityLogOpen(false);
  }, [openJobDetail]);

  // Optimistic local update after agent card save
  const handleAgentSaved = useCallback((fields: AgentSavedFields) => {
    setKernelAgents((prev) => {
      if (!prev) return prev;
      return prev.map((a) =>
        a.id === fields.agentId
          ? {
              ...a,
              ...(fields.role !== undefined && { role: fields.role }),
              ...(fields.provider !== undefined && { provider: fields.provider }),
              ...(fields.model !== undefined && { model: fields.model }),
              ...(fields.raisonDetre !== undefined && { raisonDetre: fields.raisonDetre }),
              ...(fields.personaPrompt !== undefined && { personaPrompt: fields.personaPrompt }),
            }
          : a,
      );
    });
  }, []);

  const selectedInboxItem = world.inbox.find((i) => i.id === selectedInboxItemId);

  // Show loading during secrets check
  if (checkingSecrets) {
    return (
      <div className="flex items-center justify-center h-full">
        <span className="text-sm animate-pulse" style={{ color: 'var(--ws-fg-muted)' }}>
          Checking connections...
        </span>
      </div>
    );
  }

  // Show onboarding if no connection exists
  if (!onboardingComplete) {
    return <Onboarding onComplete={handleOnboardingComplete} />;
  }

  if (!tauriReady || skinLoading || !skin) {
    return (
      <div className="flex items-center justify-center h-full">
        <span className="text-sm animate-pulse" style={{ color: 'var(--ws-fg-muted)' }}>
          {!tauriReady ? 'Connecting to engine...' : 'Loading skin...'}
        </span>
      </div>
    );
  }

  const SkinCanvas = skin.WorkspaceCanvas;

  return (
    <>
      <Header
        inboxCount={unreadCompletedCount}
        activityCount={world.activityLog.length}
        jobCount={world.jobs.length}
        onSubmitClick={() => setSubmitOpen(true)}
        onInboxClick={handleInboxClick}
        onActivityClick={() => setActivityLogOpen((o) => !o)}
        onJobHistoryClick={() => setJobHistoryOpen((o) => !o)}
        onSettingsClick={() => setSettingsOpen(true)}
      />

      <main className="flex-1 overflow-auto pt-14">
        <SkinCanvas world={world} onDeskClick={setSelectedAgent} />
      </main>

      {/* Dev toolbar only in mock mode */}
      {!isTauri && <DevToolbar bus={bus} onReset={onReset} />}

      <SubmitJobDialog
        open={submitOpen}
        onOpenChange={setSubmitOpen}
        onSubmit={handleSubmitJob}
      />

      <ActivityLog
        entries={world.activityLog}
        open={activityLogOpen}
        onClose={() => setActivityLogOpen(false)}
        onEntryClick={handleActivityEntryClick}
      />

      <JobHistoryPanel
        jobs={world.jobs}
        open={jobHistoryOpen}
        onClose={() => setJobHistoryOpen(false)}
        onJobClick={(id) => {
          openJobDetail(id);
          setJobHistoryOpen(false);
        }}
        viewedJobIds={viewedJobIds}
        onMarkAllRead={() => {
          setViewedJobIds(new Set(world.jobs.filter((j) => j.status === 'COMPLETED').map((j) => j.id)));
        }}
        onRerun={handleRerun}
        onCancel={handleCancelJob}
      />

      {selectedJobId && (
        <JobDetailModal
          jobId={selectedJobId}
          swoMap={world.swoMap}
          agents={resolvedAgents}
          activityLog={world.activityLog}
          onClose={() => setSelectedJobId(null)}
          onRerun={handleRerun}
          onRequestRevision={handleRequestRevision}
          bus={tauriBusInstance}
          artifacts={world.artifactsBySwo?.[selectedJobId]}
        />
      )}

      <Settings
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        activeSkinId={skin.id}
        availableSkins={availableSkins}
        onSkinSelect={setSkin}
        onConnectionSaved={retryKernelBoot}
        bus={tauriBusInstance}
      />

      {selectedAgent && (
        <AgentCardModal
          agentId={selectedAgent}
          agents={resolvedAgents}
          desks={[...world.desks, ...world.bench]}
          jobs={world.jobs}
          swoMap={world.swoMap}
          activityLog={world.activityLog}
          onClose={() => setSelectedAgent(null)}
          onSaved={handleAgentSaved}
          onJobClick={(id) => {
            setSelectedAgent(null);
            openJobDetail(id);
          }}
        />
      )}

      {selectedInboxItem && (
        <ArtifactViewer
          title={selectedInboxItem.title}
          agentName={selectedInboxItem.agentName}
          content={selectedInboxItem.content}
          onClose={() => setSelectedInboxItemId(null)}
        />
      )}
    </>
  );
}
