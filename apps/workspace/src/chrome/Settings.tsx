import { useState, useEffect } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { X } from 'lucide-react';
import { ConnectionsSection } from './settings/ConnectionsSection';
import { TeamSection } from './settings/TeamSection';
import { AppearanceSection } from './settings/AppearanceSection';
import { NotificationsSection } from './settings/NotificationsSection';
import { AdvancedSection } from './settings/AdvancedSection';
import { UsageCostSection } from './settings/UsageCostSection';
import { ToolInventorySection } from './settings/ToolInventorySection';
import { SchedulesSection } from './settings/SchedulesSection';
import { McpSection } from './settings/McpSection';
import type { TauriBus } from '../sim/tauriBus';

export type SettingsSection =
  | 'connections'
  | 'team'
  | 'appearance'
  | 'notifications'
  | 'advanced'
  | 'usage'
  | 'tools'
  | 'schedules'
  | 'mcp';

interface SettingsProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Current skin id for Appearance section */
  activeSkinId: string;
  availableSkins: { id: string; name: string }[];
  onSkinSelect: (id: string) => void;
  /** Called after saving a new API key — triggers kernel re-boot */
  onConnectionSaved?: () => void;
  /** Initial section to open to (e.g. from onboarding) */
  initialSection?: SettingsSection;
  /** TauriBus instance for fetching usage data (Tauri only) */
  bus?: TauriBus;
}

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: 'connections', label: 'Connections' },
  { id: 'team', label: 'Your Team' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'tools', label: 'Tool Inventory' },
  { id: 'schedules', label: 'Schedules' },
  { id: 'mcp', label: 'MCP Connectors' },
  { id: 'advanced', label: 'Advanced' },
  { id: 'usage', label: 'Usage & Cost' },
];

export function Settings({
  open,
  onOpenChange,
  activeSkinId,
  availableSkins,
  onSkinSelect,
  onConnectionSaved,
  initialSection = 'connections',
  bus,
}: SettingsProps) {
  const [activeSection, setActiveSection] = useState<SettingsSection>(initialSection);

  // Reset to initial section when opened
  useEffect(() => {
    if (open) {
      setActiveSection(initialSection);
    }
  }, [open, initialSection]);

  // Keyboard shortcut: Escape to close (handled by Radix Dialog)
  // Keyboard shortcut: Cmd/Ctrl+, to open (handled in App.tsx)

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay
          style={{
            position: 'fixed',
            inset: 0,
            backgroundColor: 'rgba(0, 0, 0, 0.7)',
            zIndex: 100,
          }}
        />
        <Dialog.Content
          style={{
            position: 'fixed',
            inset: '40px',
            zIndex: 101,
            display: 'flex',
            backgroundColor: 'var(--ws-bg)',
            border: '1px solid var(--ws-border)',
            borderRadius: 'var(--ws-radius-md)',
            overflow: 'hidden',
            outline: 'none',
          }}
          aria-describedby={undefined}
        >
          {/* Sidebar */}
          <nav
            style={{
              width: '200px',
              borderRight: '1px solid var(--ws-border-subtle)',
              padding: 'var(--ws-space-xl) 0',
              flexShrink: 0,
              display: 'flex',
              flexDirection: 'column',
            }}
          >
            <Dialog.Title
              style={{
                fontFamily: 'monospace',
                fontSize: 'var(--ws-font-base)',
                fontWeight: 700,
                color: 'var(--ws-fg-primary)',
                letterSpacing: '0.15em',
                textTransform: 'uppercase',
                padding: '0 var(--ws-space-lg) var(--ws-space-lg)',
                borderBottom: '1px solid var(--ws-border-subtle)',
                marginBottom: 'var(--ws-space-sm)',
              }}
            >
              Settings
            </Dialog.Title>
            {SECTIONS.map((section) => (
              <button
                key={section.id}
                onClick={() => setActiveSection(section.id)}
                style={{
                  display: 'block',
                  width: '100%',
                  padding: 'var(--ws-space-sm) var(--ws-space-lg)',
                  textAlign: 'left',
                  fontFamily: 'monospace',
                  fontSize: 'var(--ws-font-base)',
                  letterSpacing: '0.05em',
                  color:
                    activeSection === section.id
                      ? 'var(--ws-fg-primary)'
                      : 'var(--ws-fg-secondary)',
                  backgroundColor:
                    activeSection === section.id
                      ? 'var(--ws-accent-soft)'
                      : 'transparent',
                  border: 'none',
                  cursor: 'pointer',
                  transition: 'all 0.1s ease',
                  borderLeft:
                    activeSection === section.id
                      ? '2px solid var(--ws-fg-primary)'
                      : '2px solid transparent',
                }}
              >
                {section.label}
              </button>
            ))}
          </nav>

          {/* Content pane */}
          <div
            style={{
              flex: 1,
              overflow: 'auto',
              padding: '24px 32px',
              position: 'relative',
            }}
          >
            {/* Close button */}
            <Dialog.Close asChild>
              <button
                style={{
                  position: 'absolute',
                  top: '16px',
                  right: '16px',
                  background: 'none',
                  border: 'none',
                  color: 'rgb(34 197 94 / 0.5)',
                  cursor: 'pointer',
                  padding: '4px',
                }}
                aria-label="Close settings"
              >
                <X size={18} />
              </button>
            </Dialog.Close>

            {activeSection === 'connections' && <ConnectionsSection onConnectionSaved={onConnectionSaved} />}
            {activeSection === 'team' && <TeamSection />}
            {activeSection === 'appearance' && (
              <AppearanceSection
                activeSkinId={activeSkinId}
                availableSkins={availableSkins}
                onSkinSelect={onSkinSelect}
              />
            )}
            {activeSection === 'notifications' && <NotificationsSection />}
            {activeSection === 'tools' && <ToolInventorySection />}
            {activeSection === 'schedules' && <SchedulesSection />}
            {activeSection === 'mcp' && <McpSection />}
            {activeSection === 'advanced' && <AdvancedSection />}
            {activeSection === 'usage' && <UsageCostSection bus={bus} />}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
