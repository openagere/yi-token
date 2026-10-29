import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Markdown preview of a record's text. Only react-markdown and GFM run here: raw HTML in the source is not parsed
 * (no rehype-raw), so a script tag shows up as text. Links always open in a new tab without a referrer.
 */
const REMARK_PLUGINS = [remarkGfm];

const COMPONENTS: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noreferrer">
      {children}
    </a>
  ),
};

export function MarkdownBlock({ source }: { source: string }) {
  return (
    <div className="markdown-block">
      <Markdown remarkPlugins={REMARK_PLUGINS} components={COMPONENTS}>
        {source}
      </Markdown>
    </div>
  );
}
