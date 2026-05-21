import { Fragment, useMemo } from "react";

type MarkdownBlock =
  | { type: "heading"; depth: number; text: string }
  | { type: "paragraph"; text: string }
  | { type: "quote"; text: string }
  | { type: "list"; items: string[] }
  | { type: "code"; text: string };

type InlineMarkdownSegment =
  | { type: "text"; text: string }
  | { type: "code"; text: string }
  | { type: "link"; label: string; target: string };

export function MarkdownContent({ text }: { text: string }) {
  const blocks = useMemo(() => parseMarkdownBlocks(text), [text]);

  return (
    <div className="markdown-content">
      {blocks.map((block, index) => {
        if (block.type === "heading") {
          const Tag = block.depth === 1 ? "h1" : block.depth === 2 ? "h2" : "h3";
          return <Tag key={`${block.type}:${index}`}>{renderInlineMarkdown(block.text)}</Tag>;
        }

        if (block.type === "code") {
          return (
            <pre key={`${block.type}:${index}`} className="markdown-code-block">
              <code>{block.text}</code>
            </pre>
          );
        }

        if (block.type === "quote") {
          return (
            <blockquote key={`${block.type}:${index}`}>
              {renderInlineMarkdown(block.text)}
            </blockquote>
          );
        }

        if (block.type === "list") {
          return (
            <ul key={`${block.type}:${index}`}>
              {block.items.map((item, itemIndex) => (
                <li key={`${block.type}:${index}:${itemIndex}`}>{renderInlineMarkdown(item)}</li>
              ))}
            </ul>
          );
        }

        return <p key={`${block.type}:${index}`}>{renderInlineMarkdown(block.text)}</p>;
      })}
    </div>
  );
}

function parseMarkdownBlocks(text: string): MarkdownBlock[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const blocks: MarkdownBlock[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    const trimmed = line.trim();

    if (!trimmed) {
      index += 1;
      continue;
    }

    if (trimmed.startsWith("```")) {
      const codeLines: string[] = [];
      index += 1;
      while (index < lines.length && !lines[index].trim().startsWith("```")) {
        codeLines.push(lines[index]);
        index += 1;
      }
      if (index < lines.length) {
        index += 1;
      }
      blocks.push({ type: "code", text: codeLines.join("\n") });
      continue;
    }

    const headingMatch = trimmed.match(/^(#{1,3})\s+(.*)$/);
    if (headingMatch) {
      blocks.push({
        type: "heading",
        depth: headingMatch[1].length,
        text: headingMatch[2],
      });
      index += 1;
      continue;
    }

    if (trimmed.startsWith(">")) {
      const quoteLines: string[] = [];
      while (index < lines.length) {
        const quoteLine = lines[index].trim();
        if (!quoteLine.startsWith(">")) {
          break;
        }
        quoteLines.push(quoteLine.replace(/^>\s?/, ""));
        index += 1;
      }
      blocks.push({ type: "quote", text: quoteLines.join("\n") });
      continue;
    }

    if (/^[-*]\s+/.test(trimmed)) {
      const items: string[] = [];
      while (index < lines.length) {
        const listLine = lines[index].trim();
        const match = listLine.match(/^[-*]\s+(.*)$/);
        if (!match) {
          break;
        }
        items.push(match[1]);
        index += 1;
      }
      blocks.push({ type: "list", items });
      continue;
    }

    const paragraphLines: string[] = [];
    while (index < lines.length) {
      const paragraphLine = lines[index];
      const paragraphTrimmed = paragraphLine.trim();
      if (
        !paragraphTrimmed ||
        paragraphTrimmed.startsWith("```") ||
        /^(#{1,3})\s+/.test(paragraphTrimmed) ||
        paragraphTrimmed.startsWith(">") ||
        /^[-*]\s+/.test(paragraphTrimmed)
      ) {
        break;
      }
      paragraphLines.push(paragraphLine);
      index += 1;
    }
    blocks.push({
      type: "paragraph",
      text: paragraphLines.join("\n"),
    });
  }

  return blocks;
}

function renderInlineMarkdown(text: string) {
  const segments = parseInlineMarkdownSegments(text);

  return segments.map((segment, index) => {
    if (segment.type === "code") {
      return <code key={`code:${index}`}>{segment.text}</code>;
    }

    if (segment.type === "link") {
      const key = `link:${index}:${segment.target}`;
      const href = isLocalMarkdownLinkTarget(segment.target) ? "#" : segment.target;
      return (
        <a
          key={key}
          href={href}
          onClick={(event) => {
            event.preventDefault();
            void openMarkdownLinkTarget(segment.target);
          }}
        >
          {segment.label}
        </a>
      );
    }

    return <Fragment key={`text:${index}`}>{segment.text}</Fragment>;
  });
}

function parseInlineMarkdownSegments(text: string): InlineMarkdownSegment[] {
  const segments: InlineMarkdownSegment[] = [];
  let cursor = 0;

  while (cursor < text.length) {
    const codeSegment = tryParseInlineCode(text, cursor);
    if (codeSegment) {
      if (codeSegment.start > cursor) {
        segments.push({ type: "text", text: text.slice(cursor, codeSegment.start) });
      }
      segments.push({ type: "code", text: codeSegment.text });
      cursor = codeSegment.end;
      continue;
    }

    const linkSegment = tryParseInlineLink(text, cursor);
    if (linkSegment) {
      if (linkSegment.start > cursor) {
        segments.push({ type: "text", text: text.slice(cursor, linkSegment.start) });
      }
      segments.push({
        type: "link",
        label: linkSegment.label,
        target: linkSegment.target,
      });
      cursor = linkSegment.end;
      continue;
    }

    const nextSpecialIndex = findNextInlineSpecialIndex(text, cursor);
    const nextCursor =
      nextSpecialIndex === -1
        ? text.length
        : nextSpecialIndex === cursor
          ? cursor + 1
          : nextSpecialIndex;
    segments.push({
      type: "text",
      text: text.slice(cursor, nextCursor),
    });
    cursor = nextCursor;
  }

  return segments;
}

function tryParseInlineCode(text: string, start: number) {
  if (text[start] !== "`") {
    return null;
  }

  const end = text.indexOf("`", start + 1);
  if (end === -1) {
    return null;
  }

  return {
    start,
    end: end + 1,
    text: text.slice(start + 1, end),
  };
}

function tryParseInlineLink(text: string, start: number) {
  if (text[start] !== "[") {
    return null;
  }

  const labelEnd = text.indexOf("](", start);
  if (labelEnd === -1) {
    return null;
  }

  const targetStart = labelEnd + 2;
  let depth = 1;
  let cursor = targetStart;
  while (cursor < text.length) {
    const char = text[cursor];
    if (char === "(") {
      depth += 1;
    } else if (char === ")") {
      depth -= 1;
      if (depth === 0) {
        return {
          start,
          end: cursor + 1,
          label: text.slice(start + 1, labelEnd),
          target: text.slice(targetStart, cursor),
        };
      }
    }
    cursor += 1;
  }

  return null;
}

function findNextInlineSpecialIndex(text: string, start: number) {
  const nextCode = text.indexOf("`", start);
  const nextLink = text.indexOf("[", start);

  if (nextCode === -1) {
    return nextLink;
  }
  if (nextLink === -1) {
    return nextCode;
  }
  return Math.min(nextCode, nextLink);
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
