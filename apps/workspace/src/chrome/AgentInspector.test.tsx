import { render, screen, fireEvent } from '@testing-library/react';
import { AgentInspector } from './AgentInspector';

describe('AgentInspector', () => {
  it('renders agent name', () => {
    render(<AgentInspector agentId="perry" onClose={vi.fn()} />);
    expect(screen.getByText('Perry')).toBeInTheDocument();
  });

  it('renders agent role', () => {
    render(<AgentInspector agentId="perry" onClose={vi.fn()} />);
    expect(screen.getByText('COO')).toBeInTheDocument();
  });

  it('renders skills list', () => {
    render(<AgentInspector agentId="perry" onClose={vi.fn()} />);
    expect(screen.getByText('delegation')).toBeInTheDocument();
    expect(screen.getByText('sprint-planning')).toBeInTheDocument();
    expect(screen.getByText('cross-functional-synthesis')).toBeInTheDocument();
  });

  it('close button triggers onClose', () => {
    const handler = vi.fn();
    render(<AgentInspector agentId="perry" onClose={handler} />);

    // The X button rendered by lucide-react
    const buttons = screen.getAllByRole('button');
    fireEvent.click(buttons[0]!);
    expect(handler).toHaveBeenCalledOnce();
  });

  it('renders nothing when agentId is unknown', () => {
    const { container } = render(<AgentInspector agentId="unknown-agent" onClose={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders lois with correct role and skills', () => {
    render(<AgentInspector agentId="lois" onClose={vi.fn()} />);
    expect(screen.getByText('Lois')).toBeInTheDocument();
    expect(screen.getByText('CIO')).toBeInTheDocument();
    expect(screen.getByText('research')).toBeInTheDocument();
  });

  it('shows current task when provided', () => {
    render(
      <AgentInspector agentId="perry" currentTask="Analyzing market data" onClose={vi.fn()} />,
    );
    expect(screen.getByText('Analyzing market data')).toBeInTheDocument();
  });

  it('does not show current task section when currentTask is null', () => {
    render(<AgentInspector agentId="perry" currentTask={null} onClose={vi.fn()} />);
    expect(screen.queryByText('CURRENT TASK')).not.toBeInTheDocument();
  });

  it('renders agent title', () => {
    render(<AgentInspector agentId="perry" onClose={vi.fn()} />);
    expect(screen.getByText('Chief Operating Officer')).toBeInTheDocument();
  });
});
