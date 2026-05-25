import { useState, useEffect } from 'react';
import { isTauriRuntime } from '../../sim/platform';

interface AgentRow {
  id: string;
  name: string;
  role: string;
  presence: string;
  orgClass: string;
  directReportCount: number;
}

export function TeamSection() {
  const [agents, setAgents] = useState<AgentRow[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadRoster();
  }, []);

  async function loadRoster() {
    setLoading(true);
    if (isTauriRuntime()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const tree = await invoke<
          {
            id: string;
            name: string;
            role: string;
            presence: string;
            orgProfile: { orgClass: string };
            directReportCount: number;
            children: unknown[];
          }[]
        >('roster_tree');
        // Flatten tree into rows
        const rows: AgentRow[] = [];
        function walk(nodes: typeof tree) {
          for (const node of nodes) {
            rows.push({
              id: node.id,
              name: node.name,
              role: node.role,
              presence: node.presence,
              orgClass: node.orgProfile.orgClass,
              directReportCount: node.directReportCount,
            });
            if (Array.isArray(node.children)) {
              walk(node.children as typeof tree);
            }
          }
        }
        walk(tree);
        setAgents(rows);
      } catch (err) {
        console.error('[settings] Failed to load roster:', err);
        setAgents(mockAgents());
      }
    } else {
      setAgents(mockAgents());
    }
    setLoading(false);
  }

  if (loading) {
    return (
      <div style={{ color: 'rgb(34 197 94 / 0.5)', fontFamily: 'monospace', fontSize: '0.75rem' }}>
        Loading your team...
      </div>
    );
  }

  return (
    <div>
      <h2 style={headingStyle}>Your Team</h2>
      <p style={subTextStyle}>
        {agents.length} team member{agents.length === 1 ? '' : 's'} in your organization.
      </p>

      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
        {agents.map((agent) => (
          <div
            key={agent.id}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '12px',
              padding: '10px 14px',
              border: '1px solid rgb(34 197 94 / 0.1)',
              borderRadius: '4px',
            }}
          >
            {/* Presence dot */}
            <div
              style={{
                width: '6px',
                height: '6px',
                borderRadius: '50%',
                backgroundColor: presenceColor(agent.presence),
                flexShrink: 0,
              }}
            />
            {/* Name + role */}
            <div style={{ flex: 1, minWidth: 0 }}>
              <div
                style={{
                  fontFamily: 'monospace',
                  fontSize: '0.75rem',
                  fontWeight: 700,
                  color: 'rgb(74 222 128)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {agent.name}
              </div>
              <div
                style={{
                  fontFamily: 'monospace',
                  fontSize: '0.6rem',
                  color: 'rgb(34 197 94 / 0.5)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {agent.role}
              </div>
            </div>
            {/* Org class badge */}
            <span
              style={{
                fontFamily: 'monospace',
                fontSize: '0.55rem',
                letterSpacing: '0.08em',
                textTransform: 'uppercase',
                color: 'rgb(34 197 94 / 0.4)',
                padding: '2px 6px',
                border: '1px solid rgb(34 197 94 / 0.15)',
                borderRadius: '2px',
                flexShrink: 0,
              }}
            >
              {formatOrgClass(agent.orgClass)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function presenceColor(presence: string): string {
  switch (presence) {
    case 'READY':
    case 'COMPUTING':
      return 'rgb(74 222 128)';
    case 'IDLE':
      return 'rgb(34 197 94 / 0.4)';
    case 'STALE':
      return '#eab308';
    default:
      return 'rgb(34 197 94 / 0.15)';
  }
}

function formatOrgClass(orgClass: string): string {
  switch (orgClass) {
    case 'Manager':
      return 'Manager';
    case 'LeadIc':
      return 'Lead';
    case 'Specialist':
      return 'Specialist';
    default:
      return orgClass;
  }
}

function mockAgents(): AgentRow[] {
  return [
    { id: '1', name: 'Perry', role: 'Chief Operating Officer', presence: 'READY', orgClass: 'Manager', directReportCount: 5 },
    { id: '2', name: 'Oracle', role: 'Product Lead', presence: 'READY', orgClass: 'Manager', directReportCount: 3 },
    { id: '3', name: 'Felicity', role: 'CTO', presence: 'IDLE', orgClass: 'LeadIc', directReportCount: 0 },
    { id: '4', name: 'Jimmy', role: 'UX Designer', presence: 'IDLE', orgClass: 'Specialist', directReportCount: 0 },
    { id: '5', name: 'Lex', role: 'Writer', presence: 'OFFLINE', orgClass: 'Specialist', directReportCount: 0 },
  ];
}

const headingStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.85rem',
  fontWeight: 700,
  color: 'rgb(74 222 128)',
  letterSpacing: '0.1em',
  textTransform: 'uppercase',
  marginBottom: '8px',
};

const subTextStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  color: 'rgb(34 197 94 / 0.6)',
  marginBottom: '20px',
  lineHeight: 1.5,
};
