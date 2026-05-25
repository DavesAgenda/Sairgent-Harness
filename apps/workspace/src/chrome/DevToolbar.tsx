import { useRef, useState } from 'react';
import type { Bus } from '../types';
import { runHappyPath, runBlockedPath, runParallelBurst, resetIdCounter } from '../sim/mockScenarios';
import { SignalRecorder } from '../sim/signalRecorder';
import { ReplayBus } from '../sim/replayBus';

interface DevToolbarProps {
  bus: Bus;
  onReset: () => void;
}

type ScenarioButton = {
  label: string;
  shortLabel: string;
  action: () => void;
  color: string;
};

export function DevToolbar({ bus, onReset }: DevToolbarProps) {
  const [isRecording, setIsRecording] = useState(false);
  const recorderRef = useRef<SignalRecorder | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const buttons: ScenarioButton[] = [
    {
      label: 'HAPPY PATH',
      shortLabel: '✓ HAPPY',
      action: () => runHappyPath(bus),
      color: 'rgb(74 222 128)',
    },
    {
      label: 'BLOCKED PATH',
      shortLabel: '⊘ BLOCKED',
      action: () => runBlockedPath(bus),
      color: 'rgb(250 204 21)',
    },
    {
      label: 'PARALLEL BURST',
      shortLabel: '⇶ PARALLEL',
      action: () => runParallelBurst(bus),
      color: 'rgb(96 165 250)',
    },
  ];

  function handleReset() {
    resetIdCounter();
    onReset();
  }

  function handleToggleRecord() {
    if (isRecording) {
      // Stop recording and download
      recorderRef.current?.stop();
      recorderRef.current?.download();
      recorderRef.current = null;
      setIsRecording(false);
    } else {
      // Start recording
      const recorder = new SignalRecorder();
      recorder.wrap(bus);
      recorderRef.current = recorder;
      setIsRecording(true);
    }
  }

  function handleLoadReplay() {
    fileInputRef.current?.click();
  }

  async function handleFileSelected(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const recorded = await SignalRecorder.fromFile(file);
      const replayBus = new ReplayBus();
      // Forward replayed signals to the active bus
      replayBus.subscribe((signal) => bus.emit(signal));
      replayBus.replay(recorded);
    } catch (err) {
      console.error('[DevToolbar] Failed to load replay:', err);
    }
    // Reset file input
    e.target.value = '';
  }

  return (
    <div
      style={{
        position: 'fixed',
        bottom: '12px',
        left: '12px',
        zIndex: 60,
        display: 'flex',
        alignItems: 'center',
        gap: '6px',
        fontFamily: 'monospace',
      }}
    >
      {/* Dev label */}
      <span
        style={{
          fontSize: 'var(--ws-font-xs)',
          color: 'var(--ws-fg-dim)',
          letterSpacing: '0.1em',
          textTransform: 'uppercase',
          marginRight: '4px',
        }}
      >
        DEV▸
      </span>

      {/* Scenario buttons */}
      {buttons.map(({ label, action, color }) => (
        <ToolButton
          key={label}
          label={label}
          color={color}
          onClick={action}
        />
      ))}

      {/* Divider */}
      <span style={{ color: 'rgb(34 197 94 / 0.2)', fontSize: '0.75rem' }}>│</span>

      {/* Record/Replay controls */}
      <ToolButton
        label={isRecording ? '⏹ STOP REC' : '⏺ RECORD'}
        color={isRecording ? 'rgb(248 113 113)' : 'rgb(251 146 60)'}
        onClick={handleToggleRecord}
      />
      <ToolButton
        label="▶ REPLAY"
        color="rgb(168 85 247)"
        onClick={handleLoadReplay}
      />

      {/* Hidden file input for replay load */}
      <input
        ref={fileInputRef}
        type="file"
        accept=".json"
        style={{ display: 'none' }}
        onChange={handleFileSelected}
      />

      {/* Divider */}
      <span style={{ color: 'rgb(34 197 94 / 0.2)', fontSize: '0.75rem' }}>│</span>

      {/* Reset button */}
      <ToolButton
        label="RESET"
        color="rgb(248 113 113)"
        onClick={handleReset}
        prefix="⟳ "
      />
    </div>
  );
}

interface ToolButtonProps {
  label: string;
  color: string;
  onClick: () => void;
  prefix?: string;
}

function ToolButton({ label, color, onClick, prefix = '' }: ToolButtonProps) {
  return (
    <button
      onClick={onClick}
      style={{
        fontFamily: 'monospace',
        fontSize: 'var(--ws-font-xs)',
        letterSpacing: '0.07em',
        color: color,
        backgroundColor: 'rgb(9 9 11 / 0.95)',
        border: `1px solid ${color.replace(')', ' / 0.4)')}`,
        padding: '4px 10px',
        cursor: 'pointer',
        textTransform: 'uppercase',
        transition: 'all 0.12s ease',
        whiteSpace: 'nowrap',
      }}
      onMouseEnter={(e) => {
        const btn = e.currentTarget;
        btn.style.backgroundColor = color.replace('rgb(', 'rgb(').replace(')', ' / 0.12)');
        btn.style.borderColor = color;
        btn.style.boxShadow = `0 0 8px ${color.replace(')', ' / 0.35)')}`;
        btn.style.color = color;
      }}
      onMouseLeave={(e) => {
        const btn = e.currentTarget;
        btn.style.backgroundColor = 'rgb(9 9 11 / 0.95)';
        btn.style.borderColor = color.replace(')', ' / 0.4)');
        btn.style.boxShadow = 'none';
        btn.style.color = color;
      }}
    >
      {prefix}{label}
    </button>
  );
}
