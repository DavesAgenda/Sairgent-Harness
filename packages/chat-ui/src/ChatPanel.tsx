import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { FeedMessage } from '@sairgent/chat-core';

interface ChatPanelProps {
  messages: FeedMessage[];
  status: string;
  onSend: (msg: string) => void;
}

function bubbleClass(message: FeedMessage) {
  if (message.role === 'user') return 'bubble bubble--user';
  if (message.role === 'system') return 'bubble bubble--system';
  return 'bubble bubble--agent';
}

export const ChatPanel: React.FC<ChatPanelProps> = ({ messages, status, onSend }) => {
  const [input, setInput] = React.useState('');
  const bottomRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  const handleSend = () => {
    const text = input.trim();
    if (!text) return;
    onSend(text);
    setInput('');
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const busy = status !== 'ready';

  return (
    <>
      <div className="chat-messages">
        {messages.map((message) => (
          <div key={message.id} className={bubbleClass(message)}>
            <span className="bubble__sender">{message.sender}</span>
            <div className="bubble__content">
              <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml>
                {message.content}
              </ReactMarkdown>
            </div>
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

      <div className="chat-input-row">
        <textarea
          className="chat-textarea"
          rows={2}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Message Perry (COO)... (Enter to send)"
          disabled={busy}
        />
        <button className="btn-send" onClick={handleSend} disabled={busy || !input.trim()}>
          Send
        </button>
      </div>
    </>
  );
};

export default ChatPanel;
