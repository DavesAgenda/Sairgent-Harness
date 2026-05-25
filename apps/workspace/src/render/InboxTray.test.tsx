import { render, screen, fireEvent } from '@testing-library/react';
import { InboxTray } from './InboxTray';
import type { InboxItem } from '../types';

function makeItem(id: string, overrides: Partial<InboxItem> = {}): InboxItem {
  return {
    id,
    swoId: `swo-${id}`,
    agentName: 'Perry',
    title: `Item ${id}`,
    content: 'Content here',
    timestamp: Date.now(),
    ...overrides,
  };
}

describe('InboxTray', () => {
  it('shows empty state with no items', () => {
    render(<InboxTray items={[]} onItemClick={vi.fn()} />);
    expect(screen.getByText(/no deliverables yet/i)).toBeInTheDocument();
  });

  it('does not show empty state when items present', () => {
    render(<InboxTray items={[makeItem('1')]} onItemClick={vi.fn()} />);
    expect(screen.queryByText(/no deliverables yet/i)).not.toBeInTheDocument();
  });

  it('renders a card for each inbox item', () => {
    const items = [makeItem('1', { title: 'First Report' }), makeItem('2', { title: 'Second Report' })];
    render(<InboxTray items={items} onItemClick={vi.fn()} />);

    expect(screen.getByText('First Report')).toBeInTheDocument();
    expect(screen.getByText('Second Report')).toBeInTheDocument();
  });

  it('click triggers callback with correct item id', () => {
    const handler = vi.fn();
    const items = [makeItem('abc123', { title: 'My Report' })];
    render(<InboxTray items={items} onItemClick={handler} />);

    fireEvent.click(screen.getByText('My Report'));
    expect(handler).toHaveBeenCalledOnce();
    expect(handler).toHaveBeenCalledWith('abc123');
  });

  it('clicking second card calls handler with its id', () => {
    const handler = vi.fn();
    const items = [
      makeItem('first', { title: 'First Report' }),
      makeItem('second', { title: 'Second Report' }),
    ];
    render(<InboxTray items={items} onItemClick={handler} />);

    fireEvent.click(screen.getByText('Second Report'));
    expect(handler).toHaveBeenCalledWith('second');
  });

  it('renders DELIVERABLES header', () => {
    render(<InboxTray items={[]} onItemClick={vi.fn()} />);
    expect(screen.getByText('DELIVERABLES')).toBeInTheDocument();
  });

  it('shows item count badge when items present', () => {
    const items = [makeItem('1'), makeItem('2'), makeItem('3')];
    render(<InboxTray items={items} onItemClick={vi.fn()} />);
    expect(screen.getByText('[3]')).toBeInTheDocument();
  });

  it('renders agent name on each card', () => {
    const items = [
      makeItem('1', { agentName: 'Lois' }),
      makeItem('2', { agentName: 'Lex' }),
    ];
    render(<InboxTray items={items} onItemClick={vi.fn()} />);
    expect(screen.getByText('Lois')).toBeInTheDocument();
    expect(screen.getByText('Lex')).toBeInTheDocument();
  });
});
