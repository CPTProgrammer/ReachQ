//! Shared helpers for the agent panel components (design 01).

import {
	findModel,
	getIdentitySelection,
	resolveSelection
} from '$lib/state/agent-settings.svelte';
import { getThreads } from '$lib/state/agent-threads.svelte';
import type { AgentSendOpts, DiffPreview, PathMessage, ToolCallView } from '$lib/ipc/agent';

export function safeParse(json: string): unknown {
	try {
		return JSON.parse(json);
	} catch {
		return undefined;
	}
}

/** Pretty-print a JSON string; returns the input unchanged when invalid
 *  (e.g. still streaming). */
export function prettyJson(json: string): string {
	try {
		return JSON.stringify(JSON.parse(json), null, 2);
	} catch {
		return json;
	}
}

/** First non-empty string arg among the given keys of a tool call's args. */
export function argString(args: unknown, keys: string[]): string {
	if (!args || typeof args !== 'object') return '';
	const o = args as Record<string, unknown>;
	for (const k of keys) {
		const v = o[k];
		if (typeof v === 'string' && v) return v;
	}
	return '';
}

/** Joined text of a message's text blocks (user messages are text-only). */
export function messageText(msg: PathMessage): string {
	return msg.content
		.filter((b) => b.type === 'text')
		.map((b) => (b as { type: 'text'; text: string }).text)
		.join('\n');
}

/** `131072` -> `128K`, `1048576` -> `1M`. */
export function formatContextLength(n: number): string {
	if (n >= 1 << 20) return `${Math.round(n / (1 << 20))}M`;
	if (n >= 1 << 10) return `${Math.round(n / (1 << 10))}K`;
	return String(n);
}

export function formatTokens(n: number): string {
	return n.toLocaleString('en-US');
}

/**
 * `320` -> `"0.32s"`, `4200` -> `"4.2s"`, `42000` -> `"42s"`,
 * `125000` -> `"2m 5s"`, `3780000` -> `"1h 3m"`.
 */
export function formatDuration(ms: number): string {
	if (ms < 1000) return `${(ms / 1000).toFixed(2)}s`;
	if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
	if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
	const totalSec = Math.round(ms / 1000);
	if (totalSec < 3600) return `${Math.floor(totalSec / 60)}m ${totalSec % 60}s`;
	return `${Math.floor(totalSec / 3600)}h ${Math.floor((totalSec % 3600) / 60)}m`;
}

/**
 * Output throughput, excluding time-to-first-token when known:
 * `340` tokens in `4200`ms with `300`ms TTFT -> `"87"` (`"45.3"` below 100).
 */
export function formatSpeed(completionTokens: number, durationMs: number, ttftMs?: number): string {
	const decodeMs = ttftMs != null && ttftMs < durationMs ? durationMs - ttftMs : durationMs;
	if (decodeMs <= 0) return '0';
	const tps = completionTokens / (decodeMs / 1000);
	return tps >= 100 ? String(Math.round(tps)) : tps.toFixed(1);
}

/**
 * Extract the structured diff for write_file / edit_file cards. The backend
 * delivers a DiffPreview via `result.uiPayload` once the call finishes and
 * via the approval payload while approval is pending; both use the envelope
 * `{ diff: DiffPreview, path, isNewFile? }`. Only that structured shape is
 * accepted — anything else leaves the card to its raw/llmText fallback.
 */
export function extractDiff(call: ToolCallView, approvalPayload?: unknown): DiffPreview | null {
	return asDiffPreview(call.result?.uiPayload) ?? asDiffPreview(approvalPayload);
}

function asDiffPreview(payload: unknown): DiffPreview | null {
	if (!payload || typeof payload !== 'object') return null;
	const diff = (payload as Record<string, unknown>).diff;
	if (!diff || typeof diff !== 'object') return null;
	const d = diff as Record<string, unknown>;
	if (typeof d.path !== 'string' || !Array.isArray(d.hunks)) return null;
	return diff as unknown as DiffPreview;
}

/**
 * Effective send options for a thread (design 01 §3.6 / 05 §5):
 * thread snapshot -> identity last-used -> global default, with the
 * thinking flag coerced against model capabilities.
 */
export function resolveSendOpts(identity: string, threadId: string | null): AgentSendOpts | null {
	const summary = threadId ? getThreads().find((t) => t.id === threadId) : undefined;
	const sel =
		resolveSelection(identity, summary?.model ?? null) ??
		getIdentitySelection(identity) ??
		(summary?.model
			? { model: summary.model.model, thinking: summary.model.thinking, effort: summary.model.effort ?? null }
			: null);
	if (!sel) return null;
	const meta = findModel(sel.model)?.model;
	const thinking = meta
		? meta.thinkingMandatory || (meta.supportsThinking && sel.thinking)
		: sel.thinking;
	return { model: sel.model, thinking, effort: thinking ? (sel.effort ?? undefined) : undefined };
}

export function decodeBase64(b64: string): Uint8Array {
	const bin = atob(b64);
	const bytes = new Uint8Array(bin.length);
	for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
	return bytes;
}

export async function copyText(text: string): Promise<boolean> {
	try {
		await navigator.clipboard.writeText(text);
		return true;
	} catch {
		return false;
	}
}
