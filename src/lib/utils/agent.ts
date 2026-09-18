//! Shared helpers for the agent panel components (design 01).

import type { DiffPreview, PathMessage, ToolCallView } from '$lib/ipc/agent';

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
