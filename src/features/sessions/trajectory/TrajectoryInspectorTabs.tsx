import { useState, type ReactNode } from "react";
import type { ContextItem, StepContext } from "@/generated";
import { describeError } from "@/shared/api";
import { useI18n } from "@/shared/i18n";
import { useBlob, useStepContext } from "../hooks";
import { JsonTree } from "./JsonTree";
import { MarkdownBlock } from "./MarkdownBlock";
import {
  findTool,
  formatClock,
  formatCount,
  formatKiB,
  formatMs,
  parseJsonContainer,
  prettyJson,
  stepTiming,
} from "./inspectorFormat";
import { KIND_KEY, isCall } from "./records";
import { promptDiffLines, type PromptDiffLine } from "./promptDiff";
import type { InspectorTabId, InspectorTarget } from "./TrajectoryInspector";

const DASH = "—";

/** Tabs that read the step's reconstructed context (system, tools, schema, options, full context, diff). */
const CONTEXT_TABS: ReadonlySet<InspectorTabId> = new Set<InspectorTabId>([
  "system",
  "tools",
  "diff",
  "schema",
  "options",
  "context",
]);

interface TrajectoryInspectorTabsProps {
  tab: InspectorTabId;
  target: InspectorTarget;
  /** The step before this one in the same thread; the system diff compares against it. */
  previousStepId: string | null;
}

/** The content of the active inspector tab. Remounted per target, so the full-text toggles start collapsed. */
export function TrajectoryInspectorTabs({ tab, target, previousStepId }: TrajectoryInspectorTabsProps) {
  const { item } = target;
  const { record, step } = item;
  const isRequest = target.kind === "request";
  const context = useStepContext(CONTEXT_TABS.has(tab) ? step.id : null);
  const previous = useStepContext(tab === "diff" ? previousStepId : null);
  // A request marker has no record of its own; its body and result blobs are disabled.
  const bodyBlob = useBlob(isRequest ? null : record.body_blob);
  const resultBlob = useBlob(isRequest ? null : record.result_blob);
  const [fullBody, setFullBody] = useState(false);
  const [fullResult, setFullResult] = useState(false);

  if (isRequest) {
    switch (tab) {
      case "usage":
        return <UsageTab step={step} />;
      case "options":
        return <OptionsTab step={step} context={context} />;
      case "timing":
        return <RequestTimingTab step={step} />;
      default:
        return <ContextTab context={context} />;
    }
  }

  switch (tab) {
    case "overview":
      return (
        <OverviewTab
          item={item}
          bodyBlob={bodyBlob}
          fullBody={fullBody}
          onToggleBody={() => setFullBody((current) => !current)}
        />
      );
    case "preview": {
      const source =
        fullBody && bodyBlob.data !== undefined ? bodyBlob.data.json : (record.body ?? record.text);
      return (
        <>
          {record.body_blob !== null && (
            <TruncationNote
              blob={bodyBlob}
              expanded={fullBody}
              onToggle={() => setFullBody((current) => !current)}
            />
          )}
          <MarkdownBlock source={source} />
        </>
      );
    }
    case "raw": {
      const source =
        fullBody && bodyBlob.data !== undefined ? bodyBlob.data.json : (record.body ?? record.text);
      return (
        <>
          {record.body_blob !== null && (
            <TruncationNote
              blob={bodyBlob}
              expanded={fullBody}
              onToggle={() => setFullBody((current) => !current)}
            />
          )}
          <StructuredText text={source} />
          {record.thinking !== null && <ThinkingBlock text={record.thinking} />}
        </>
      );
    }
    case "arguments":
      return <StructuredText text={record.args} />;
    case "result": {
      const source =
        fullResult && resultBlob.data !== undefined
          ? resultBlob.data.json
          : (record.result_body ?? record.result);
      return (
        <>
          {record.result_blob !== null && (
            <TruncationNote
              blob={resultBlob}
              expanded={fullResult}
              onToggle={() => setFullResult((current) => !current)}
            />
          )}
          <StructuredText text={source} />
        </>
      );
    }
    case "schema":
      return <SchemaTab name={record.tool_name} context={context} />;
    case "timing":
      return <CallTimingTab record={record} />;
    case "system":
      return <SystemTab context={context} />;
    case "tools":
      return <StructuredText text={context.data?.tools ?? null} />;
    case "diff":
      return <DiffTab context={context} previous={previous} previousStepId={previousStepId} />;
    default:
      return null;
  }
}

/** The overview of a record: its facts, the one-line summary, and the body when there is one. */
function OverviewTab({
  item,
  bodyBlob,
  fullBody,
  onToggleBody,
}: {
  item: InspectorTarget["item"];
  bodyBlob: BlobQuery;
  fullBody: boolean;
  onToggleBody: () => void;
}) {
  const { t, language } = useI18n();
  const { record, step } = item;
  const call = isCall(record);
  // Only a tool call has a duration of its own. The request total belongs to the request marker's timing tab.
  const body = fullBody && bodyBlob.data !== undefined ? bodyBlob.data.json : record.body;
  return (
    <>
      <dl className="inspector-fields">
        <Field label={t("session.inspector.overview.kind")}>{t(KIND_KEY[record.kind])}</Field>
        <Field label={t("session.inspector.overview.request")}>
          {t("session.ledger.requestLabel", { number: step.number })}
        </Field>
        <Field label={t("session.inspector.overview.model")}>{step.model}</Field>
        <Field label={t("session.inspector.overview.started")}>
          {formatClock(call && record.started_at !== null ? record.started_at : step.requested_at, language)}
        </Field>
        {call && (
          <Field label={t("session.inspector.overview.duration")} estimated={record.duration_estimated}>
            {record.duration_ms === null ? DASH : formatMs(record.duration_ms, language)}
            {record.duration_estimated && <EstimateTag />}
          </Field>
        )}
      </dl>
      <p className="inspector-summary">{record.text}</p>
      {record.body_blob !== null && (
        <TruncationNote blob={bodyBlob} expanded={fullBody} onToggle={onToggleBody} />
      )}
      {body !== null && <pre className="inspector-pre">{body}</pre>}
    </>
  );
}

/** Notes that a body is cut, with its original size once the blob has arrived, and a toggle for the full text. */
function TruncationNote({
  blob,
  expanded,
  onToggle,
}: {
  blob: BlobQuery;
  expanded: boolean;
  onToggle: () => void;
}) {
  const { t } = useI18n();
  const label =
    blob.data === undefined
      ? t("session.inspector.truncatedNoSize")
      : t("session.inspector.truncated", { size: formatKiB(blob.data.raw_bytes) });
  return (
    <div className="inspector-truncation">
      <span>{label}</span>
      <button type="button" className="inspector-link" onClick={onToggle}>
        {expanded ? t("session.inspector.collapseFull") : t("session.inspector.loadFull")}
      </button>
      {expanded && blob.isError && (
        <p className="inspector-error" role="alert">
          {t("session.inspector.failedFull", { error: describeError(blob.error) })}
        </p>
      )}
      {expanded && blob.isPending && <p className="inspector-empty">{t("session.inspector.loadingFull")}</p>}
    </div>
  );
}

type BlobQuery = ReturnType<typeof useBlob>;

function UsageTab({ step }: { step: InspectorTarget["item"]["step"] }) {
  const { t, language } = useI18n();
  return (
    <dl className="inspector-fields">
      <Field label={t("session.inspector.usage.input")}>{formatCount(step.input_tokens, language)}</Field>
      <Field label={t("session.inspector.usage.inputCached")}>
        {formatCount(step.cached_tokens, language)}
      </Field>
      <Field label={t("session.inspector.usage.output")}>{formatCount(step.output_tokens, language)}</Field>
      <Field label={t("session.inspector.usage.outputReasoning")}>
        {formatCount(step.reasoning_tokens, language)}
      </Field>
      <Field label={t("session.inspector.usage.total")}>
        {formatCount(step.input_tokens + step.output_tokens, language)}
      </Field>
    </dl>
  );
}

function OptionsTab({ step, context }: { step: InspectorTarget["item"]["step"]; context: ContextQuery }) {
  const { t } = useI18n();
  const options = context.data?.options ?? {};
  return (
    <>
      <dl className="inspector-fields">
        <Field label={t("session.inspector.options.model")}>{step.model}</Field>
        <Field label={t("session.inspector.options.upstreamModel")}>{step.upstream_model}</Field>
        <Field label={t("session.inspector.options.provider")}>{step.provider_name}</Field>
        <Field label={t("session.inspector.options.protocol")}>
          {`${step.inbound_protocol} → ${step.upstream_protocol}`}
        </Field>
        <Field label={t("session.inspector.options.routeMode")}>{step.route_mode ?? DASH}</Field>
      </dl>
      <h4 className="inspector-heading">{t("session.inspector.options.optionsJson")}</h4>
      <ContextStatus context={context} />
      {context.data !== undefined && Object.keys(options).length === 0 && (
        <p className="inspector-empty">{t("session.inspector.options.none")}</p>
      )}
      {context.data !== undefined && Object.keys(options).length > 0 && (
        <dl className="inspector-fields">
          {Object.entries(options).map(([key, value]) => (
            <Field key={key} label={key}>
              {JSON.stringify(value) ?? DASH}
            </Field>
          ))}
        </dl>
      )}
    </>
  );
}

function RequestTimingTab({ step }: { step: InspectorTarget["item"]["step"] }) {
  const { t, language } = useI18n();
  const timing = stepTiming(step);
  const pending = step.finished_at === null;
  return (
    <dl className="inspector-fields">
      <Field label={t("session.inspector.timing.started")}>{formatClock(step.requested_at, language)}</Field>
      <Field label={t("session.inspector.timing.ttft")}>
        {timing.ttftMs === null ? DASH : formatMs(timing.ttftMs, language)}
      </Field>
      <Field label={t("session.inspector.timing.generation")}>
        {timing.generationMs === null ? DASH : formatMs(timing.generationMs, language)}
      </Field>
      <Field label={t("session.inspector.timing.total")}>
        {pending
          ? t("session.inspector.pending")
          : timing.totalMs === null
            ? DASH
            : formatMs(timing.totalMs, language)}
      </Field>
      <Field label={t("session.inspector.timing.throughput")}>
        {timing.tokensPerSecond === null
          ? DASH
          : t("session.inspector.timing.tokensPerSecond", {
              value: new Intl.NumberFormat(language, { maximumFractionDigits: 1 }).format(
                timing.tokensPerSecond,
              ),
            })}
      </Field>
    </dl>
  );
}

/** Timing of a tool call. The duration is an estimate: the proxy sees only the gap to the next request. */
function CallTimingTab({ record }: { record: InspectorTarget["item"]["record"] }) {
  const { t, language } = useI18n();
  const estimated = record.duration_estimated;
  return (
    <>
      <dl className="inspector-fields" data-estimated={estimated ? "true" : undefined}>
        <Field label={t("session.inspector.timing.started")}>
          {record.started_at === null ? DASH : formatClock(record.started_at, language)}
        </Field>
        <Field label={t("session.inspector.timing.duration")} estimated={estimated}>
          {record.duration_ms === null ? DASH : formatMs(record.duration_ms, language)}
          {estimated && <EstimateTag />}
        </Field>
      </dl>
      {estimated && <p className="inspector-note">{t("session.inspector.timing.estimated")}</p>}
    </>
  );
}

function SchemaTab({ name, context }: { name: string | null; context: ContextQuery }) {
  const { t } = useI18n();
  if (context.isPending) return <p className="inspector-empty">{t("session.inspector.loading")}</p>;
  if (context.isError) return <ContextStatus context={context} />;
  const tools = context.data?.tools ?? null;
  if (tools === null) return <p className="inspector-empty">{t("session.inspector.schema.noTools")}</p>;
  const definition = findTool(tools, name);
  if (definition === undefined)
    return <p className="inspector-empty">{t("session.inspector.schema.none")}</p>;
  return <JsonTree value={definition} />;
}

function SystemTab({ context }: { context: ContextQuery }) {
  const { t } = useI18n();
  if (context.isPending || context.isError) return <ContextStatus context={context} />;
  return (
    <>
      <h4 className="inspector-heading">{t("session.inspector.tab.system")}</h4>
      <StructuredText text={context.data?.system ?? null} />
    </>
  );
}

/**
 * The system prompt and the tool definitions of this step against the previous step of the thread. Each part is a
 * line diff with three lines of context, and the parts that did not change are left out.
 */
function DiffTab({
  context,
  previous,
  previousStepId,
}: {
  context: ContextQuery;
  previous: ContextQuery;
  previousStepId: string | null;
}) {
  const { t } = useI18n();
  if (previousStepId === null)
    return <p className="inspector-empty">{t("session.inspector.diff.noPrevious")}</p>;
  if (context.isPending || previous.isPending)
    return <p className="inspector-empty">{t("session.inspector.loading")}</p>;
  if (context.isError || previous.isError)
    return <ContextStatus context={context.isError ? context : previous} />;

  const before: Partial<StepContext> = previous.data ?? {};
  const after: Partial<StepContext> = context.data ?? {};
  const sections = [
    {
      title: t("session.inspector.tab.system"),
      lines: promptDiffLines(before.system ?? "", after.system ?? ""),
    },
    {
      title: t("session.inspector.tab.tools"),
      lines: promptDiffLines(prettyJson(before.tools ?? ""), prettyJson(after.tools ?? "")),
    },
  ].filter((section) => section.lines.length > 0);
  if (sections.length === 0) return <p className="inspector-empty">{t("session.inspector.diff.none")}</p>;
  return (
    <>
      {sections.map((section) => (
        <section key={section.title} className="prompt-diff-section">
          <h4 className="inspector-heading">{section.title}</h4>
          <pre className="prompt-diff">
            {section.lines.map((line, index) => (
              <DiffLine key={index} line={line} />
            ))}
          </pre>
        </section>
      ))}
    </>
  );
}

function DiffLine({ line }: { line: PromptDiffLine }) {
  return (
    <span className={`prompt-diff-line is-${line.kind}`}>
      {line.text || " "}
      {"\n"}
    </span>
  );
}

/** The full reconstructed context of a request: system prompt, tool definitions, and each input item as a tree. */
function ContextTab({ context }: { context: ContextQuery }) {
  const { t } = useI18n();
  if (context.isPending || context.isError) return <ContextStatus context={context} />;
  const data = context.data;
  if (data === undefined) return null;
  return (
    <>
      <p className="inspector-summary">
        {t("session.inspector.context.summary", { count: data.input.length, depth: data.chain_depth })}
      </p>
      <details className="inspector-fold">
        <summary>{t("session.inspector.context.system")}</summary>
        <pre className="inspector-pre">{data.system ?? DASH}</pre>
      </details>
      <details className="inspector-fold">
        <summary>{t("session.inspector.context.tools")}</summary>
        <StructuredText text={data.tools} />
      </details>
      <h4 className="inspector-heading">{t("session.inspector.context.input")}</h4>
      <ol className="inspector-context-items">
        {data.input.map((entry, index) => (
          <li key={index} data-context-item="">
            <details className="inspector-fold">
              <summary>{contextItemTitle(entry, t("session.inspector.context.truncated"))}</summary>
              <StructuredText text={entry.json} />
            </details>
          </li>
        ))}
      </ol>
    </>
  );
}

function contextItemTitle(entry: ContextItem, truncatedLabel: string): string {
  const parts = [entry.kind, entry.role ?? ""].filter((part) => part !== "");
  if (entry.truncated) parts.push(truncatedLabel);
  return parts.join(" · ");
}

type ContextQuery = ReturnType<typeof useStepContext>;

function ContextStatus({
  context,
}: {
  context: { isPending: boolean; isError: boolean; error: Error | null };
}) {
  const { t } = useI18n();
  if (context.isError) {
    return (
      <p className="inspector-error" role="alert">
        {t("session.inspector.failed", { error: describeError(context.error) })}
      </p>
    );
  }
  if (context.isPending) return <p className="inspector-empty">{t("session.inspector.loading")}</p>;
  return null;
}

/** A label and value pair of a definition list; `estimated` marks a timing that is not measured. */
function Field({ label, children, estimated }: { label: string; children: ReactNode; estimated?: boolean }) {
  return (
    <div className="inspector-field" data-estimated={estimated ? "true" : undefined}>
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

function EstimateTag() {
  const { t } = useI18n();
  return <span className="inspector-estimate">{t("session.timeline.estimated")}</span>;
}

/** Plain text in monospace, or a foldable tree when the text is a JSON object or array. Copies the text as stored. */
function StructuredText({ text }: { text: string | null }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  if (text === null || text === "")
    return <p className="inspector-empty">{t("session.inspector.noContent")}</p>;
  const tree = parseJsonContainer(text);
  return (
    <div className="inspector-structured">
      <button
        type="button"
        className="inspector-copy"
        onClick={() => {
          void navigator.clipboard?.writeText(text).then(
            () => setCopied(true),
            () => setCopied(false),
          );
        }}
      >
        {copied ? t("session.inspector.copied") : t("session.inspector.copy")}
      </button>
      {tree === undefined ? <pre className="inspector-pre">{text}</pre> : <JsonTree value={tree} />}
    </div>
  );
}

function ThinkingBlock({ text }: { text: string }) {
  const { t } = useI18n();
  return (
    <section>
      <h4 className="inspector-heading">{t("session.inspector.reasoning")}</h4>
      <pre className="inspector-pre">{text}</pre>
    </section>
  );
}
