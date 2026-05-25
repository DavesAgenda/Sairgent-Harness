import { render, screen, fireEvent } from '@testing-library/react';
import { AgentDesk } from './AgentDesk';
import type { DeskState } from '../types';

function makeDesk(overrides: Partial<DeskState> = {}): DeskState {
  return {
    agentId: 'perry',
    name: 'Perry',
    role: 'COO',
    icon: '⌘',
    presence: 'READY',
    currentTask: null,
    statusText: null,
    progress: 0,
    isDelegating: false,
    gridRow: 0,
    gridCol: 0,
    ...overrides,
  };
}

describe('AgentDesk', () => {
  it('renders agent name', () => {
    render(<AgentDesk desk={makeDesk({ name: 'Lois' })} onDeskClick={vi.fn()} />);
    expect(screen.getByText('Lois')).toBeInTheDocument();
  });

  it('renders agent role', () => {
    render(<AgentDesk desk={makeDesk({ role: 'CIO' })} onDeskClick={vi.fn()} />);
    expect(screen.getByText('CIO')).toBeInTheDocument();
  });

  it('has data-testid="agent-desk"', () => {
    render(<AgentDesk desk={makeDesk()} onDeskClick={vi.fn()} />);
    expect(screen.getByTestId('agent-desk')).toBeInTheDocument();
  });

  it('click calls onDeskClick with correct agentId', () => {
    const handler = vi.fn();
    render(<AgentDesk desk={makeDesk({ agentId: 'lois' })} onDeskClick={handler} />);
    fireEvent.click(screen.getByTestId('agent-desk'));
    expect(handler).toHaveBeenCalledOnce();
    expect(handler).toHaveBeenCalledWith('lois');
  });

  it('shows progress bar when COMPUTING with progress > 0', () => {
    const { container } = render(
      <AgentDesk
        desk={makeDesk({ presence: 'COMPUTING', progress: 0.5, currentTask: 'Analyzing' })}
        onDeskClick={vi.fn()}
      />,
    );
    // The smooth progress bar is a div with animated width
    const progressBar = container.querySelector('.h-1.rounded-sm.overflow-hidden');
    expect(progressBar).toBeInTheDocument();
  });

  it('shows progress bar when READY with progress > 0 (smooth interpolation)', () => {
    const { container } = render(
      <AgentDesk
        desk={makeDesk({ presence: 'READY', progress: 0.5 })}
        onDeskClick={vi.fn()}
      />,
    );
    // Progress bar is visible even for non-computing agents with progress > 0
    const progressBar = container.querySelector('.h-1.rounded-sm.overflow-hidden');
    expect(progressBar).toBeInTheDocument();
  });

  it('does not show progress bar when IDLE and progress is 0', () => {
    const { container } = render(
      <AgentDesk
        desk={makeDesk({ presence: 'IDLE', progress: 0 })}
        onDeskClick={vi.fn()}
      />,
    );
    const progressBar = container.querySelector('.h-1.rounded-sm.overflow-hidden');
    expect(progressBar).not.toBeInTheDocument();
  });

  it('shows current task text when present', () => {
    render(
      <AgentDesk
        desk={makeDesk({ presence: 'COMPUTING', currentTask: 'Market research' })}
        onDeskClick={vi.fn()}
      />,
    );
    expect(screen.getByText('Market research')).toBeInTheDocument();
  });

  it('shows "Ready" when presence is READY and no current task', () => {
    render(<AgentDesk desk={makeDesk({ presence: 'READY', currentTask: null })} onDeskClick={vi.fn()} />);
    expect(screen.getByText('Ready')).toBeInTheDocument();
  });

  it('shows "Idle" when presence is IDLE', () => {
    render(<AgentDesk desk={makeDesk({ presence: 'IDLE' })} onDeskClick={vi.fn()} />);
    expect(screen.getByText('Idle')).toBeInTheDocument();
  });
});
