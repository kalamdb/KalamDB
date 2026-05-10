import { Paperclip, SendHorizontal, X } from 'lucide-react';
import { useRef, useState } from 'react';

export function ChatComposer({
  disabled,
  onSend,
}: {
  disabled: boolean;
  onSend: (body: string, attachment: File | null) => Promise<void>;
}) {
  const [body, setBody] = useState('');
  const [attachment, setAttachment] = useState<File | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const canSend = body.trim().length > 0 && !disabled;

  const submit = async () => {
    const trimmed = body.trim();
    if (!trimmed || disabled) {
      return;
    }

    await onSend(trimmed, attachment);
    setBody('');
    setAttachment(null);
    if (inputRef.current) {
      inputRef.current.value = '';
    }
  };

  return (
    <form
      className="composer"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <textarea
        value={body}
        disabled={disabled}
        placeholder="Message GPT-4o..."
        rows={2}
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault();
            void submit();
          }
        }}
      />
      <div className="composer-actions">
        <button
          type="button"
          className="icon-button"
          aria-label="Attach file"
          title="Attach file"
          disabled={disabled}
          onClick={() => inputRef.current?.click()}
        >
          <Paperclip size={17} />
        </button>
        <input
          ref={inputRef}
          type="file"
          className="visually-hidden"
          disabled={disabled}
          onChange={(event) => setAttachment(event.target.files?.[0] ?? null)}
        />
        {attachment ? (
          <span className="file-chip">
            {attachment.name}
            <button type="button" aria-label="Remove attachment" onClick={() => setAttachment(null)}>
              <X size={14} />
            </button>
          </span>
        ) : null}
        <button type="submit" className="send-button" aria-label="Send message" title="Send message" disabled={!canSend}>
          <SendHorizontal size={18} />
        </button>
      </div>
    </form>
  );
}