//! Per-thread composer drafts, persisted to agent.db via the backend.
//!
//! - The in-memory cache is the immediate source for the textarea; typing
//!   never waits on IPC.
//! - Writes go through a 400ms dirty-check interval: keys whose cached value
//!   differs from the last persisted one are flushed. Crash loss window:
//!   <=400ms. Graceful paths flush explicitly: send (AgentComposer), thread
//!   switch / auto-prune (agent-threads.svelte), window beforeunload.
//! - `hydrateDrafts` seeds the cache from thread summaries on load; keys
//!   already cached are never overwritten (they may hold un-flushed input).

import { getAgentBackend } from '$lib/state/agent.svelte';
import type { ThreadSummary } from '$lib/ipc/agent';

const FLUSH_INTERVAL_MS = 400;

let drafts = $state<Record<string, string>>({});
/** Values as last successfully persisted, for keys ever written this session. */
let persisted: Record<string, string> = {};
/** Keys changed since the last successful flush. */
const dirty = new Set<string>();
let timer: ReturnType<typeof setInterval> | null = null;

/** Identity-level keys (`__identity:<id>`, composer disabled without a
 * thread) have no backend row; never persist them. */
function isPersistable(key: string): boolean {
	return !key.startsWith('__identity:');
}

export function getDraft(key: string): string {
	return drafts[key] ?? '';
}

export function setDraft(key: string, text: string): void {
	drafts[key] = text;
	if (!isPersistable(key) || persisted[key] === text) return;
	if (persisted[key] === undefined && text === '') return; // nothing to clear
	dirty.add(key);
	ensureTimer();
}

/** Append to an existing draft (queued message -> back to editing). */
export function appendDraft(key: string, text: string): void {
	const cur = getDraft(key);
	setDraft(key, cur ? `${cur}\n${text}` : text);
}

/** Seed the cache from loaded thread summaries (drafts persisted in
 * agent.db). Existing cache entries win — they may be newer than the DB. */
export function hydrateDrafts(summaries: ThreadSummary[]): void {
	for (const s of summaries) {
		if (!s.draft) continue;
		if (s.id in drafts) continue;
		drafts[s.id] = s.draft;
		persisted[s.id] = s.draft;
	}
}

/** Drop all local state for a deleted thread (its DB row cascades). */
export function discardDraft(key: string): void {
	delete drafts[key];
	delete persisted[key];
	dirty.delete(key);
}

/** Flush one key immediately (send path). */
export async function flushDraft(key: string): Promise<void> {
	if (!dirty.has(key)) return;
	await flushKey(key);
	stopTimerIfClean();
}

/** Flush every dirty key (thread switch / beforeunload). */
export async function flushDrafts(): Promise<void> {
	if (dirty.size === 0) return;
	await Promise.all([...dirty].map((k) => flushKey(k)));
	stopTimerIfClean();
}

function ensureTimer(): void {
	if (timer !== null) return;
	timer = setInterval(() => void flushDrafts(), FLUSH_INTERVAL_MS);
}

function stopTimerIfClean(): void {
	if (timer !== null && dirty.size === 0) {
		clearInterval(timer);
		timer = null;
	}
}

async function flushKey(key: string): Promise<void> {
	const value = getDraft(key);
	try {
		await getAgentBackend().threadSetDraft(key, value);
		persisted[key] = value;
		dirty.delete(key);
	} catch {
		// Stay dirty; retried on the next tick or explicit flush.
	}
}

if (typeof window !== 'undefined') {
	window.addEventListener('beforeunload', () => void flushDrafts());
}
