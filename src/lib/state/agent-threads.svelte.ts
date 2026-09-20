//! Threads list state (design 01 §2.1, §4): CRUD per owner scope, active
//! thread pointer, archive flows, and the settings-page "View all threads"
//! listing across scopes.

import { getThreadRuntime } from './agent.svelte';
import { getAgentBackend } from './agent-backend.svelte';
import type { ContentBlock, ThreadSummary } from '$lib/ipc/agent';
import { t } from '$lib/state/i18n.svelte';
import {
	discardDraft,
	flushDrafts,
	getDraft,
	hydrateDrafts
} from './agent-drafts.svelte';

/** Threads of the currently displayed scope (unarchived, recent first). */
let threads = $state<ThreadSummary[]>([]);
/** All threads across scopes including archived (settings view). */
let allThreads = $state<ThreadSummary[]>([]);
/** The thread currently open in the panel. */
let activeThreadId = $state<string | null>(null);
let loading = $state(false);
/** Scope the panel list was last loaded with. */
let currentScope: string | null = null;

export function getThreads(): ThreadSummary[] {
	return threads;
}

export function getAllThreads(): ThreadSummary[] {
	return allThreads;
}

export function getActiveThreadId(): string | null {
	return activeThreadId;
}

export function threadsLoading(): boolean {
	return loading;
}

export async function loadThreads(scope: string): Promise<void> {
	loading = true;
	try {
		threads = await getAgentBackend().threadsList(scope);
		currentScope = scope;
		hydrateDrafts(threads);
	} finally {
		loading = false;
	}
}

export async function loadAllThreads(): Promise<void> {
	allThreads = await getAgentBackend().threadsListAll();
}

export async function createThread(scope: string): Promise<ThreadSummary> {
	const prevId = activeThreadId;
	const thread = await getAgentBackend().threadCreate(scope);
	threads = [thread, ...threads];
	activeThreadId = thread.id;
	if (prevId && prevId !== thread.id) void pruneIfEmpty(prevId);
	return thread;
}

/** Re-anchor a thread to a different owner scope (settings "all threads"). */
export async function reassignThread(threadId: string, ownerKey: string): Promise<void> {
	await getAgentBackend().threadReassign(threadId, ownerKey);
	allThreads = allThreads.map((t) => (t.id === threadId ? { ...t, ownerKey } : t));
	if (ownerKey === currentScope) {
		// Moved INTO the open panel's scope: reload for correct order/draft.
		if (currentScope) await loadThreads(currentScope);
	} else if (threads.some((t) => t.id === threadId)) {
		// Moved OUT of the open panel's scope.
		threads = threads.filter((t) => t.id !== threadId);
		if (activeThreadId === threadId) activeThreadId = threads[0]?.id ?? null;
	}
}

/** Re-anchor every thread of one owner scope to another (settings batch move). */
export async function reassignScope(fromKey: string, toKey: string): Promise<void> {
	await getAgentBackend().threadReassignScope(fromKey, toKey);
	allThreads = allThreads.map((t) => (t.ownerKey === fromKey ? { ...t, ownerKey: toKey } : t));
	if (currentScope === fromKey || currentScope === toKey) {
		// The open panel is the source (now empty) or the target (grew): reload.
		if (currentScope) await loadThreads(currentScope);
		if (activeThreadId && !threads.some((t) => t.id === activeThreadId)) {
			activeThreadId = threads[0]?.id ?? null;
		}
	}
}

export function selectThread(threadId: string): void {
	const prevId = activeThreadId;
	if (prevId === threadId) return;
	activeThreadId = threadId;
	if (prevId) void pruneIfEmpty(prevId);
}

/** Auto-delete the thread switched away from when it is an untouched shell:
 * no messages, no draft, no title (a user-set title is explicit intent to
 * keep). Drafts are flushed first so the check sees the latest text and the
 * surviving thread's draft is on disk. */
async function pruneIfEmpty(threadId: string): Promise<void> {
	await flushDrafts().catch(() => {});
	// The user may have switched back while the flush was in flight.
	if (activeThreadId === threadId) return;
	const summary = threads.find((x) => x.id === threadId);
	if (!summary) return; // archived / deleted / another identity's list
	if (summary.title) return;
	const rt = getThreadRuntime(threadId);
	const hasMessages = rt?.loaded ? rt.messages.length > 0 : summary.messageCount > 0;
	if (hasMessages) return;
	if (getDraft(threadId).trim().length > 0) return;
	await deleteThread(threadId).catch(() => {});
}

export async function renameThread(threadId: string, title: string): Promise<void> {
	await getAgentBackend().threadRename(threadId, title);
	threads = threads.map((t) => (t.id === threadId ? { ...t, title } : t));
	allThreads = allThreads.map((t) => (t.id === threadId ? { ...t, title } : t));
}

export async function archiveThread(threadId: string): Promise<void> {
	await getAgentBackend().threadArchive(threadId, true);
	threads = threads.filter((t) => t.id !== threadId);
	allThreads = allThreads.map((t) => (t.id === threadId ? { ...t, archived: true } : t));
	if (activeThreadId === threadId) activeThreadId = threads[0]?.id ?? null;
}

export async function unarchiveThread(threadId: string): Promise<void> {
	await getAgentBackend().threadArchive(threadId, false);
	allThreads = allThreads.map((t) => (t.id === threadId ? { ...t, archived: false } : t));
}

export async function deleteThread(threadId: string): Promise<void> {
	await getAgentBackend().threadDelete(threadId);
	discardDraft(threadId);
	threads = threads.filter((t) => t.id !== threadId);
	allThreads = allThreads.filter((t) => t.id !== threadId);
	if (activeThreadId === threadId) activeThreadId = threads[0]?.id ?? null;
}

/** Apply a title change pushed by the backend (auto-generated titles). */
export function applyThreadTitle(threadId: string, title: string): void {
	threads = threads.map((t) => (t.id === threadId ? { ...t, title } : t));
	allThreads = allThreads.map((t) => (t.id === threadId ? { ...t, title } : t));
}

const PREVIEW_LENGTH = 30;

/** Collapse whitespace/newlines so previews stay single-line. */
function normalizePreview(text: string): string {
	return text.replace(/\s+/g, ' ').trim();
}

function truncatePreview(text: string): string {
	return [...text].slice(0, PREVIEW_LENGTH).join('');
}

type TextBlock = Extract<ContentBlock, { type: 'text' }>;

/** First user message preview from the live runtime (loaded threads only).
 * Reactive and always current — unlike the summary's `preview`, which is a
 * list-time snapshot. */
function liveFirstUserPreview(threadId: string): string | null {
	const rt = getThreadRuntime(threadId);
	if (!rt?.loaded) return null;
	for (const p of rt.messages) {
		if (p.role !== 'user') continue;
		const block = p.content.find((b): b is TextBlock => b.type === 'text');
		const normalized = block ? normalizePreview(block.text) : '';
		if (normalized) return truncatePreview(normalized);
	}
	return null;
}

/** Display title for an untitled thread: real title → first-user-message
 * preview (live runtime, then list-time snapshot) → in-progress draft →
 * localized "New Thread". Drafts/runtimes are `$state`, so the label
 * updates while typing and right after the first send. */
export function threadDisplayTitle(thread: ThreadSummary): string {
	if (thread.title) return thread.title;
	const live = liveFirstUserPreview(thread.id);
	if (live) return live;
	if (thread.preview) return truncatePreview(normalizePreview(thread.preview));
	const draft = normalizePreview(getDraft(thread.id));
	if (draft) return truncatePreview(draft);
	return t('agent.untitled_thread');
}

/** Relative "last active" label (design 01 §2.1: 1m / 1h / 1d / 1w / 1mo). */
export function relativeTime(updatedAt: number): string {
	const seconds = Math.max(0, (Date.now() - updatedAt) / 1000);
	if (seconds < 90) return `${Math.max(1, Math.round(seconds / 60))}m`;
	const minutes = seconds / 60;
	if (minutes < 60) return `${Math.floor(minutes)}m`;
	const hours = minutes / 60;
	if (hours < 36) return `${Math.floor(hours)}h`;
	const days = hours / 24;
	if (days < 7) return `${Math.floor(days)}d`;
	if (days < 45) return `${Math.floor(days / 7)}w`;
	return `${Math.floor(days / 30)}mo`;
}
