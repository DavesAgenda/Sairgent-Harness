import { render, screen } from '@testing-library/react';
import { WorkspaceCanvas } from './WorkspaceCanvas';
import type { DeskState, WorkspaceWorld } from '../types';

function makeDesk(id: string, overrides: Partial<DeskState> = {}): DeskState {
  return {
    agentId: id,
    name: id.charAt(0).toUpperCase() + id.slice(1),
    role: 'Agent',
    icon: '◈',
    presence: 'IDLE',
    currentTask: null,
    statusText: null,
    progress: 0,
    isDelegating: false,
    gridRow: 0,
    gridCol: 0,
    ...overrides,
  };
}

function makeWorld(overrides: Partial<WorkspaceWorld> = {}): WorkspaceWorld {
  return {
    desks: [],
    tubes: [],
    bench: [],
    inbox: [],
    jobs: [],
    swoMap: new Map(),
    ...overrides,
  };
}

describe('WorkspaceCanvas', () => {
  it('renders correct number of desks', () => {
    const world = makeWorld({
      desks: [
        makeDesk('perry', { gridRow: 0, gridCol: 0 }),
        makeDesk('lois', { gridRow: 0, gridCol: 1 }),
        makeDesk('lex', { gridRow: 0, gridCol: 2 }),
      ],
    });
    render(<WorkspaceCanvas world={world} onDeskClick={vi.fn()} />);
    const deskEls = screen.getAllByTestId('agent-desk');
    expect(deskEls).toHaveLength(3);
  });

  it('shows empty state when no active desks', () => {
    render(<WorkspaceCanvas world={makeWorld()} onDeskClick={vi.fn()} />);
    expect(screen.getByText(/workspace idle/i)).toBeInTheDocument();
  });

  it('does not show empty state when there are active desks', () => {
    const world = makeWorld({
      desks: [makeDesk('perry', { gridRow: 0, gridCol: 0 })],
    });
    render(<WorkspaceCanvas world={world} onDeskClick={vi.fn()} />);
    expect(screen.queryByText(/workspace idle/i)).not.toBeInTheDocument();
  });

  it('shows bench row with idle agents', () => {
    const world = makeWorld({
      bench: [makeDesk('lois'), makeDesk('lex')],
    });
    render(<WorkspaceCanvas world={world} onDeskClick={vi.fn()} />);
    expect(screen.getByText('Lois')).toBeInTheDocument();
    expect(screen.getByText('Lex')).toBeInTheDocument();
  });

  it('renders both active desks and bench agents simultaneously', () => {
    const world = makeWorld({
      desks: [makeDesk('perry', { gridRow: 0, gridCol: 2 })],
      bench: [makeDesk('lois'), makeDesk('lex')],
    });
    render(<WorkspaceCanvas world={world} onDeskClick={vi.fn()} />);

    expect(screen.getAllByTestId('agent-desk')).toHaveLength(1);
    expect(screen.getByText('Lois')).toBeInTheDocument();
    expect(screen.getByText('Lex')).toBeInTheDocument();
  });
});
