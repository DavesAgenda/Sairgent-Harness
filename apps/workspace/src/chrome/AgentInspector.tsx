import { motion, AnimatePresence } from 'motion/react';
import * as ScrollArea from '@radix-ui/react-scroll-area';
import { X } from 'lucide-react';
import { getAgentById } from '../sim/mockRoster';

interface AgentInspectorProps {
  agentId: string;
  currentTask?: string | null;
  onClose: () => void;
}

export function AgentInspector({ agentId, currentTask, onClose }: AgentInspectorProps) {
  const agent = getAgentById(agentId);

  if (!agent) return null;

  return (
    <AnimatePresence>
      <div
        style={{
          position: 'fixed',
          inset: 0,
          zIndex: 80,
          pointerEvents: 'auto',
        }}
      >
        {/* Backdrop */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          onClick={onClose}
          style={{
            position: 'absolute',
            inset: 0,
            backgroundColor: 'rgb(0 0 0 / 0.5)',
          }}
        />

        {/* Drawer panel */}
        <motion.div
          initial={{ x: '100%' }}
          animate={{ x: 0 }}
          exit={{ x: '100%' }}
          transition={{ type: 'tween', duration: 0.25, ease: 'easeOut' }}
          style={{
            position: 'absolute',
            top: 0,
            right: 0,
            bottom: 0,
            width: 'min(380px, 100vw)',
            backgroundColor: 'var(--ws-bg)',
            borderLeft: '1px solid var(--ws-border)',
            boxShadow: '-8px 0 40px rgb(0 0 0 / 0.6)',
            display: 'flex',
            flexDirection: 'column',
            fontFamily: 'monospace',
          }}
        >
          {/* Top bar */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: 'var(--ws-space-md) var(--ws-space-lg)',
              borderBottom: '1px solid var(--ws-border-subtle)',
              backgroundColor: 'var(--ws-bg-elevated)',
              flexShrink: 0,
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)' }}>╔═</span>
              <span
                style={{
                  fontSize: 'var(--ws-font-sm)',
                  color: 'var(--ws-fg-secondary)',
                  letterSpacing: '0.12em',
                  textTransform: 'uppercase',
                }}
              >
                AGENT PROFILE
              </span>
              <span style={{ color: 'var(--ws-fg-dim)', fontSize: 'var(--ws-font-sm)' }}>═╗</span>
            </div>
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

          {/* Scrollable content */}
          <ScrollArea.Root style={{ flex: 1, overflow: 'hidden' }}>
            <ScrollArea.Viewport style={{ height: '100%', width: '100%' }}>
              <div style={{ padding: '24px 20px', display: 'flex', flexDirection: 'column', gap: '24px' }}>

                {/* Agent identity */}
                <div style={{ display: 'flex', alignItems: 'center', gap: '16px' }}>
                  <div
                    style={{
                      width: '56px',
                      height: '56px',
                      border: '1px solid rgb(34 197 94 / 0.5)',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      fontSize: '1.8rem',
                      color: 'rgb(74 222 128)',
                      backgroundColor: 'rgb(34 197 94 / 0.06)',
                      flexShrink: 0,
                    }}
                  >
                    {agent.icon}
                  </div>
                  <div>
                    <div
                      style={{
                        fontSize: 'var(--ws-font-xl)',
                        fontWeight: 700,
                        color: 'var(--ws-fg-primary)',
                        letterSpacing: '0.08em',
                        textTransform: 'uppercase',
                      }}
                    >
                      {agent.name}
                    </div>
                    <div
                      style={{
                        fontSize: 'var(--ws-font-sm)',
                        color: 'var(--ws-fg-secondary)',
                        letterSpacing: '0.06em',
                        marginTop: '2px',
                      }}
                    >
                      {agent.role}
                    </div>
                  </div>
                </div>

                {/* Title */}
                <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                  <SectionLabel>TITLE</SectionLabel>
                  <p
                    style={{
                      fontSize: 'var(--ws-font-md)',
                      color: 'var(--ws-fg-primary)',
                      margin: 0,
                      letterSpacing: '0.03em',
                    }}
                  >
                    {agent.title}
                  </p>
                </div>

                {/* Current task */}
                {currentTask && (
                  <div
                    style={{
                      border: '1px solid rgb(34 197 94 / 0.35)',
                      padding: '10px 12px',
                      backgroundColor: 'rgb(34 197 94 / 0.04)',
                    }}
                  >
                    <SectionLabel>CURRENT TASK</SectionLabel>
                    <p
                      style={{
                        fontSize: 'var(--ws-font-base)',
                        color: 'var(--ws-fg-primary)',
                        margin: '6px 0 0',
                        fontStyle: 'italic',
                      }}
                    >
                      {currentTask}
                    </p>
                    <div
                      style={{
                        marginTop: '8px',
                        height: '2px',
                        backgroundColor: 'rgb(34 197 94 / 0.15)',
                        position: 'relative',
                        overflow: 'hidden',
                      }}
                    >
                      <motion.div
                        animate={{ x: ['-100%', '200%'] }}
                        transition={{ duration: 2, repeat: Infinity, ease: 'linear' }}
                        style={{
                          position: 'absolute',
                          top: 0,
                          left: 0,
                          width: '50%',
                          height: '100%',
                          backgroundColor: 'rgb(74 222 128)',
                        }}
                      />
                    </div>
                  </div>
                )}

                {/* Skills */}
                <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
                  <SectionLabel>SKILLS</SectionLabel>
                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px' }}>
                    {(agent.skills ?? []).map((skill) => (
                      <span
                        key={skill}
                        style={{
                          fontFamily: 'monospace',
                          fontSize: 'var(--ws-font-sm)',
                          color: 'var(--ws-fg-primary)',
                          border: '1px solid var(--ws-border)',
                          borderRadius: 'var(--ws-radius-sm)',
                          padding: '3px 8px',
                          letterSpacing: '0.05em',
                          backgroundColor: 'var(--ws-bg-elevated)',
                        }}
                      >
                        {skill}
                      </span>
                    ))}
                  </div>
                </div>

                {/* Tools */}
                <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
                  <SectionLabel>TOOLS</SectionLabel>
                  <div style={{ display: 'flex', flexWrap: 'wrap', gap: '6px' }}>
                    {(agent.tools ?? []).map((tool) => (
                      <span
                        key={tool}
                        style={{
                          fontFamily: 'monospace',
                          fontSize: 'var(--ws-font-sm)',
                          color: 'var(--ws-fg-secondary)',
                          border: '1px solid var(--ws-border-subtle)',
                          borderRadius: 'var(--ws-radius-sm)',
                          padding: '3px 8px',
                          letterSpacing: '0.05em',
                          backgroundColor: 'transparent',
                        }}
                      >
                        [{tool}]
                      </span>
                    ))}
                  </div>
                </div>

                {/* Footer decoration */}
                <div
                  style={{
                    fontSize: '0.6rem',
                    color: 'rgb(34 197 94 / 0.25)',
                    letterSpacing: '0.05em',
                    marginTop: '8px',
                  }}
                >
                  ╚══ {agent.id.toUpperCase()} ══╝
                </div>
              </div>
            </ScrollArea.Viewport>

            <ScrollArea.Scrollbar
              orientation="vertical"
              style={{
                display: 'flex',
                padding: '2px',
                width: '8px',
                backgroundColor: 'transparent',
              }}
            >
              <ScrollArea.Thumb
                style={{
                  flex: 1,
                  backgroundColor: 'rgb(34 197 94 / 0.25)',
                  borderRadius: '0',
                  position: 'relative',
                }}
              />
            </ScrollArea.Scrollbar>
          </ScrollArea.Root>
        </motion.div>
      </div>
    </AnimatePresence>
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
      ▸ {children}
    </span>
  );
}
