import type { TrajectoryStep } from "@/generated";

/** Parses text as JSON when the result is an object or an array; anything else is undefined. */
export function parseJsonContainer(text: string | null | undefined): unknown {
  if (text === null || text === undefined) return undefined;
  try {
    const value: unknown = JSON.parse(text);
    return value !== null && typeof value === "object" ? value : undefined;
  } catch {
    return undefined;
  }
}

/** Re-indents JSON text so that diffs compare line by line; text that is not JSON is returned unchanged. */
export function prettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

/** The tool definition with the given name from a step's `tools` JSON array, or undefined. */
export function findTool(tools: string | null, name: string | null): unknown {
  if (tools === null || name === null) return undefined;
  const list = parseJsonContainer(tools);
  if (!Array.isArray(list)) return undefined;
  return list.find((item: unknown) => {
    if (item === null || typeof item !== "object") return false;
    return (item as { name?: unknown }).name === name;
  });
}

/** Original size of a truncated body, in KiB with one decimal, as the inspector shows it. */
export function formatKiB(bytes: number): string {
  return `${(bytes / 1024).toFixed(1)} KiB`;
}

export function formatMs(milliseconds: number, language: string): string {
  return `${new Intl.NumberFormat(language).format(Math.round(milliseconds))} ms`;
}

export function formatCount(value: number, language: string): string {
  return new Intl.NumberFormat(language).format(value);
}

/** Local date and time to the second, as the inspector shows start times. */
export function formatClock(timestamp: number, language: string): string {
  const date = new Date(timestamp);
  const two = (value: number) => String(value).padStart(2, "0");
  const time = date.toLocaleTimeString(language, {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  return `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())} ${time}`;
}

/** Request timing as the inspector shows it: TTFT, generation and total, each null when a timestamp is missing. */
export interface StepTiming {
  ttftMs: number | null;
  generationMs: number | null;
  totalMs: number | null;
  /** Output tokens per second over the generation phase, when both are known and the phase is not empty. */
  tokensPerSecond: number | null;
}

export function stepTiming(step: TrajectoryStep): StepTiming {
  const total = step.finished_at === null ? null : step.finished_at - step.requested_at;
  const ttft = step.first_token_at === null ? null : step.first_token_at - step.requested_at;
  const generation =
    step.first_token_at === null || step.finished_at === null ? null : step.finished_at - step.first_token_at;
  const tokensPerSecond =
    generation !== null && generation > 0 && step.output_tokens > 0
      ? step.output_tokens / (generation / 1000)
      : null;
  return { ttftMs: ttft, generationMs: generation, totalMs: total, tokensPerSecond };
}
