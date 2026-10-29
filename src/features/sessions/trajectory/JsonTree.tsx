import { useState } from "react";

/** Levels expanded when the tree first renders; deeper containers start collapsed. */
const DEFAULT_OPEN_DEPTH = 2;

interface JsonTreeProps {
  value: unknown;
  openDepth?: number;
}

/** A collapsible view of a parsed JSON value. Objects and arrays fold; primitives show as they are. */
export function JsonTree({ value, openDepth = DEFAULT_OPEN_DEPTH }: JsonTreeProps) {
  return (
    <div className="json-tree">
      <JsonNode name={null} value={value} depth={0} openDepth={openDepth} />
    </div>
  );
}

interface JsonNodeProps {
  name: string | null;
  value: unknown;
  depth: number;
  openDepth: number;
}

function JsonNode({ name, value, depth, openDepth }: JsonNodeProps) {
  const [open, setOpen] = useState(depth < openDepth);
  const label = name === null ? null : <span className="json-key">{name}: </span>;

  if (value === null || typeof value !== "object") {
    return (
      <div className="json-leaf">
        {label}
        <span className={`json-value json-${value === null ? "null" : typeof value}`}>
          {JSON.stringify(value) ?? String(value)}
        </span>
      </div>
    );
  }

  const isArray = Array.isArray(value);
  const entries: [string, unknown][] = Object.entries(value);
  const [openBracket, closeBracket] = isArray ? ["[", "]"] : ["{", "}"];
  return (
    <div className="json-node">
      <button
        type="button"
        className="json-toggle"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        {label}
        {open ? openBracket : `${openBracket} … ${entries.length} ${closeBracket}`}
      </button>
      {open && (
        <div className="json-children">
          {entries.map(([key, child]) => (
            <JsonNode key={key} name={key} value={child} depth={depth + 1} openDepth={openDepth} />
          ))}
          <span className="json-bracket">{closeBracket}</span>
        </div>
      )}
    </div>
  );
}
