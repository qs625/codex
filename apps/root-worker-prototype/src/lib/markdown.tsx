import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export function MarkdownContent({ text }: { text: string }) {
  return (
    <div className="markdown-content">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children, ...props }) => {
            const target = href ?? "";
            const fallbackHref = isLocalMarkdownLinkTarget(target) ? "#" : target;

            return (
              <a
                {...props}
                href={fallbackHref}
                onClick={(event) => {
                  event.preventDefault();
                  if (target) {
                    void openMarkdownLinkTarget(target);
                  }
                }}
              >
                {children}
              </a>
            );
          },
          pre: ({ children }) => <pre className="markdown-code-block">{children}</pre>,
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}

async function openMarkdownLinkTarget(target: string) {
  if (window.codexDesktop) {
    try {
      await window.codexDesktop.openLink(target);
      return;
    } catch (error) {
      console.error("Failed to open markdown link target", target, error);
    }
  }

  if (isLocalMarkdownLinkTarget(target)) {
    return;
  }

  window.open(target, "_blank", "noopener,noreferrer");
}

function isLocalMarkdownLinkTarget(target: string) {
  return (
    target.startsWith("file://") ||
    target.startsWith("/") ||
    target.startsWith("~/") ||
    target.startsWith("./") ||
    target.startsWith("../") ||
    target.startsWith("\\\\") ||
    /^[A-Za-z]:[\\/]/.test(target)
  );
}
