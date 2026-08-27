//! Per-thread composer drafts, shared between AgentComposer (owns the
//! textarea) and AgentChat (the queued bar's "back to editing" action
//! appends the queued message into the draft). Design 01 §3.5.

let drafts = $state<Record<string, string>>({});

export function getDraft(key: string): string {
	return drafts[key] ?? '';
}

export function setDraft(key: string, text: string): void {
	drafts[key] = text;
}

/** Append to an existing draft (queued message -> back to editing). */
export function appendDraft(key: string, text: string): void {
	const cur = getDraft(key);
	setDraft(key, cur ? `${cur}\n${text}` : text);
}
