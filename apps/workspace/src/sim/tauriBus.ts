/**
 * TauriBus — Implements the workspace Bus interface by connecting to the
 * real Sairgent kernel via Tauri IPC commands and events.
 *
 * Bootstrap: invoke runtime_bootstrap → adapt roster + queue → emit as workspace signals
 * Live: listen runtime-signal → adapt each kernel signal → emit workspace signals
 *
 * Falls back cleanly: if Tauri APIs aren't available, init() is a no-op.
 */
import type { Agent, AgentTokenTotals, ArtifactPreview, Bus, BusBootstrap, CliTool, CliToolUpsertRequest, OutboxArtifact, RuntimeSignal, SwoRecord, TokenUsageRecord } from '../types';
import {
  adaptBootstrap,
  adaptSignal,
  type AgentNameMap,
  type KernelBootstrap,
  type KernelSignal,
} from './signalAdapter';

export class TauriBus implements Bus {
  private listeners = new Set<(signal: RuntimeSignal) => void>();
  private unlistenFn: (() => void) | null = null;
  private swoCache = new Map<string, SwoRecord>();
  private nameMap: AgentNameMap = new Map();
  private _agents: Agent[] = [];
  private _initialized = false;

  subscribe(callback: (signal: RuntimeSignal) => void): () => void {
    this.listeners.add(callback);
    return () => {
      this.listeners.delete(callback);
    };
  }

  emit(signal: RuntimeSignal): void {
    // Track SWO state for the adapter (it needs existing SWOs to
    // distinguish swo.created vs swo.updated)
    if (signal.type === 'swo.created' || signal.type === 'swo.updated') {
      const swo = signal.payload.swo as SwoRecord | undefined;
      if (swo) this.swoCache.set(swo.id, swo);
    }
    if (signal.type === 'swo.completed') {
      const partial = signal.payload.swo as { id: string } | undefined;
      if (partial) {
        const existing = this.swoCache.get(partial.id);
        if (existing) {
          this.swoCache.set(partial.id, { ...existing, status: 'COMPLETED', progress: 1 });
        }
      }
    }

    for (const listener of this.listeners) {
      listener(signal);
    }
  }

  get agents(): Agent[] {
    return this._agents;
  }

  get initialized(): boolean {
    return this._initialized;
  }

  /**
   * Connect to the kernel: bootstrap + subscribe to live signals.
   * Returns the bootstrap data for the workspace to use as initial state.
   */
  async init(): Promise<BusBootstrap> {
    // Dynamic import — these modules only exist in a Tauri build
    const { invoke } = await import('@tauri-apps/api/core');
    const { listen } = await import('@tauri-apps/api/event');

    // 0. Boot the kernel if it hasn't been booted yet
    try {
      console.log('[tauriBus] calling kernel_boot_from_keychain...');
      await invoke('kernel_boot_from_keychain');
      console.log('[tauriBus] kernel booted successfully');
    } catch (err) {
      console.error('[tauriBus] kernel_boot_from_keychain FAILED:', err);
      // If boot fails, bootstrap will also fail — let it propagate
      throw new Error(`Kernel boot failed: ${err}`);
    }

    // 1. Bootstrap: get initial state from kernel
    const bootstrap = await invoke<KernelBootstrap>('runtime_bootstrap');
    const { agents, swos, signals, nameMap } = adaptBootstrap(bootstrap);
    this._agents = agents;
    this.nameMap = nameMap;

    // Pre-populate SWO cache
    for (const swo of swos) {
      this.swoCache.set(swo.id, swo);
    }

    // Emit bootstrap signals to hydrate workspace state
    for (const signal of signals) {
      this.emit(signal);
    }

    // 2. Subscribe to live kernel signals
    const cursor = bootstrap.cursor.value;
    // Replay any signals we missed between bootstrap and subscription
    try {
      const { invoke: replayInvoke } = await import('@tauri-apps/api/core');
      const missed = await replayInvoke<KernelSignal[]>('runtime_replay', {
        request: { cursor, limit: 200 },
      });
      if (Array.isArray(missed)) {
        for (const kernelSignal of missed) {
          const adapted = adaptSignal(kernelSignal, this.swoCache, this.nameMap);
          for (const ws of adapted) {
            this.emit(ws);
          }
        }
      }
    } catch {
      // Replay is best-effort — if it fails, we still have bootstrap data
    }

    // 3. Listen for live signals
    const unlisten = await listen<KernelSignal>('runtime-signal', (event) => {
      console.log('[tauriBus] signal:', event.payload.kind, JSON.stringify(event.payload.payload).slice(0, 200));
      const adapted = adaptSignal(event.payload, this.swoCache, this.nameMap);
      console.log('[tauriBus] adapted:', adapted.length, 'signals →', adapted.map(s => s.type));
      for (const ws of adapted) {
        this.emit(ws);
      }
    });

    this.unlistenFn = unlisten;
    this._initialized = true;

    // Collect inbox items that were emitted via signals during bootstrap
    const inboxItems = signals
      .filter((s) => s.type === 'inbox.item.added')
      .map((s) => s.payload.item as import('../types').InboxItem);

    return {
      agents,
      swos,
      inbox: inboxItems,
    };
  }

  /** Submit a work order to the kernel. */
  async submitJob(title: string, description: string): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core');
    console.log('[tauriBus] submitting job:', title);
    await invoke('submit_work_order', {
      request: {
        title,
        outcome: description || title,
        priority: 'NORMAL',
      },
    });
    console.log('[tauriBus] job submitted successfully');
  }

  /** Cancel an active work order. */
  async cancelJob(swoId: string): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('cancel_work_order', { swoId: Number(swoId) });
  }

  /** Request a revision on a completed work order, preserving lineage and delegation tree. */
  async requestRevision(swoId: string, feedback: string): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core');
    console.log('[tauriBus] requesting revision for SWO', swoId);
    await invoke('queue_request_revision', {
      request: {
        swoId: Number(swoId),
        feedback,
      },
    });
    console.log('[tauriBus] revision requested successfully');
  }

  /** Load token usage records for a specific SWO. */
  async loadTokenUsageForSwo(swoId: string): Promise<TokenUsageRecord[]> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<TokenUsageRecord[]>('token_usage_for_swo', { swoId: Number(swoId) });
  }

  /** Load per-agent token usage totals across all runs. */
  async loadTokenUsageTotals(): Promise<AgentTokenTotals[]> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<AgentTokenTotals[]>('token_usage_totals');
  }

  /** Load all CLI tools from the kernel. */
  async loadCliTools(): Promise<CliTool[]> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<CliTool[]>('cli_tools_list');
  }

  /** Create or update a CLI tool in the kernel. */
  async upsertCliTool(request: CliToolUpsertRequest): Promise<CliTool> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<CliTool>('cli_tool_upsert', { request });
  }

  /** Delete a CLI tool by ID. */
  async deleteCliTool(toolId: string): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<void>('cli_tool_delete', { toolId });
  }

  /** List artifacts for a specific SWO. */
  async artifactsForSwo(swoId: string): Promise<OutboxArtifact[]> {
    const { invoke } = await import('@tauri-apps/api/core');
    const raw = await invoke<Array<{
      id: number;
      swoId: number | null;
      agentId: string | null;
      filename: string;
      absolutePath: string;
      createdAt: string;
    }>>('artifacts_for_swo', { swoId: Number(swoId) });
    return raw.map((r) => ({
      id: r.id,
      swoId: r.swoId,
      agentId: r.agentId,
      filename: r.filename,
      absolutePath: r.absolutePath,
      createdAt: new Date(r.createdAt).getTime(),
    }));
  }

  /** Fetch the preview content for an artifact. */
  async previewArtifact(artifactId: number): Promise<ArtifactPreview> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<ArtifactPreview>('preview_generated_artifact', { artifactId });
  }

  dispose(): void {
    if (this.unlistenFn) {
      this.unlistenFn();
      this.unlistenFn = null;
    }
    this.listeners.clear();
    this.swoCache.clear();
    this._initialized = false;
  }
}
