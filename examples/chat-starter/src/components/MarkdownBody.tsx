import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

interface MarkdownBodyProps {
  children: string;
  className?: string;
}

/** URL allow-list for assistant-rendered links. Assistant bodies are
 *  LLM-generated and can also echo back tool-result content the user
 *  controls — so we must not blindly trust `href`. react-markdown calls
 *  this for every link / image URL; returning an empty string makes the
 *  anchor inert. */
function safeUrl(url?: string): string {
  if (!url) return "";
  const trimmed = url.trim();
  // Relative URLs (no scheme) and the http(s)/mailto schemes are safe.
  // Block javascript:, data:, vbscript:, file:, anything else exotic.
  if (/^(https?:|mailto:)/i.test(trimmed)) return trimmed;
  if (/^[a-z][a-z0-9+.-]*:/i.test(trimmed)) return ""; // any other scheme → drop
  return trimmed; // bare / relative paths
}

// Renders assistant bodies as markdown. Streaming bodies arrive as
// concatenated token chunks (e.g. "**bo" then "ld**"); react-markdown
// re-parses on every render and emits what is currently parseable, so the
// in-flight string just looks like it has unrendered punctuation for a
// moment — exactly the behavior every modern AI chat shows.
//
// Tailwind v4 doesn't ship @tailwindcss/typography, so we style the
// element selectors directly via the `prose-*` className conventions below.
export function MarkdownBody({ children, className }: MarkdownBodyProps) {
  return (
    <div className={cn("md-body", className)}>
      <Markdown
        remarkPlugins={[remarkGfm]}
        urlTransform={safeUrl}
        components={{
          a: ({ href, children: linkChildren, ...rest }) => (
            <a
              {...rest}
              href={href}
              target="_blank"
              rel="noreferrer noopener"
              className="text-[var(--accent)] underline decoration-[var(--accent)]/40 hover:decoration-[var(--accent)]"
            >
              {linkChildren}
            </a>
          ),
          code: ({ className: cls, children: codeChildren, ...rest }) => {
            const inline = !cls;
            if (inline) {
              return (
                <code
                  {...rest}
                  className="font-mono text-[12.5px] rounded px-1 py-0.5 bg-[var(--background-alt)]/80 border border-[var(--surface-border)]"
                >
                  {codeChildren}
                </code>
              );
            }
            return (
              <code {...rest} className={cn("font-mono text-[12.5px] leading-relaxed block", cls)}>
                {codeChildren}
              </code>
            );
          },
          pre: ({ children: preChildren, ...rest }) => (
            <pre
              {...rest}
              className="my-2 rounded-lg bg-[var(--background-alt)]/80 border border-[var(--surface-border)] p-3 overflow-x-auto"
            >
              {preChildren}
            </pre>
          ),
        }}
      >
        {children}
      </Markdown>
    </div>
  );
}
