import { useState } from 'react';

export function NotificationsSection() {
  const [soundEnabled, setSoundEnabled] = useState(false);
  const [desktopNotifications, setDesktopNotifications] = useState(false);
  const [notifyJobComplete, setNotifyJobComplete] = useState(true);
  const [notifyBlocked, setNotifyBlocked] = useState(true);
  const [notifyNeedsReview, setNotifyNeedsReview] = useState(true);

  return (
    <div>
      <h2 style={headingStyle}>Notifications</h2>
      <p style={subTextStyle}>Choose how you want to be alerted when your team needs attention.</p>

      <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
        <ToggleRow
          label="Sound effects"
          description="Play sounds when tasks complete or need attention"
          enabled={soundEnabled}
          onChange={setSoundEnabled}
        />
        <ToggleRow
          label="Desktop notifications"
          description="Show system notifications for important events"
          enabled={desktopNotifications}
          onChange={(v) => {
            if (v && 'Notification' in window && Notification.permission !== 'granted') {
              Notification.requestPermission().then((perm) => {
                setDesktopNotifications(perm === 'granted');
              });
            } else {
              setDesktopNotifications(v);
            }
          }}
        />

        <div style={{ borderTop: '1px solid rgb(34 197 94 / 0.1)', paddingTop: '16px', marginTop: '4px' }}>
          <label style={labelStyle}>Event Types</label>
          <div style={{ display: 'flex', flexDirection: 'column', gap: '12px' }}>
            <ToggleRow
              label="Task complete"
              description="When a task finishes successfully"
              enabled={notifyJobComplete}
              onChange={setNotifyJobComplete}
            />
            <ToggleRow
              label="Blocked"
              description="When a task is blocked and needs your help"
              enabled={notifyBlocked}
              onChange={setNotifyBlocked}
            />
            <ToggleRow
              label="Needs review"
              description="When output is ready for your approval"
              enabled={notifyNeedsReview}
              onChange={setNotifyNeedsReview}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

function ToggleRow({
  label,
  description,
  enabled,
  onChange,
}: {
  label: string;
  description: string;
  enabled: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
      <div style={{ flex: 1 }}>
        <div
          style={{
            fontFamily: 'monospace',
            fontSize: '0.75rem',
            fontWeight: 600,
            color: 'rgb(74 222 128)',
            marginBottom: '2px',
          }}
        >
          {label}
        </div>
        <div
          style={{
            fontFamily: 'monospace',
            fontSize: '0.6rem',
            color: 'rgb(34 197 94 / 0.5)',
          }}
        >
          {description}
        </div>
      </div>
      <button
        onClick={() => onChange(!enabled)}
        role="switch"
        aria-checked={enabled}
        style={{
          width: '36px',
          height: '20px',
          borderRadius: '10px',
          border: 'none',
          backgroundColor: enabled ? 'rgb(34 197 94 / 0.6)' : 'rgb(34 197 94 / 0.15)',
          cursor: 'pointer',
          position: 'relative',
          transition: 'background-color 0.15s ease',
          flexShrink: 0,
        }}
      >
        <div
          style={{
            position: 'absolute',
            top: '2px',
            left: enabled ? '18px' : '2px',
            width: '16px',
            height: '16px',
            borderRadius: '50%',
            backgroundColor: enabled ? 'rgb(74 222 128)' : 'rgb(34 197 94 / 0.3)',
            transition: 'left 0.15s ease, background-color 0.15s ease',
          }}
        />
      </button>
    </div>
  );
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
  marginBottom: '24px',
  lineHeight: 1.5,
};

const labelStyle: React.CSSProperties = {
  display: 'block',
  fontFamily: 'monospace',
  fontSize: '0.7rem',
  fontWeight: 700,
  color: 'rgb(34 197 94 / 0.7)',
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  marginBottom: '12px',
};
