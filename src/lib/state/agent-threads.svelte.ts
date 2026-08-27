//! Threads list state (design 01 §2.1, §4): CRUD per identity, active
//! thread pointer, archive flows, and the settings-page "View all threads"
//! listing across identities.

import { getAgentBackend } from './agent.svelte';
import type { ThreadSummary } from '$lib/ipc/agent';

/** Threads of the currently displayed identity (unarchived, recent first). */
let threads = $state<ThreadSummary[]>([]);
/** All threads across identities including archived (settings view). */
let allThreads = $state<ThreadSummary[]>([]);
/** The thread currently open in the panel. */
let activeThreadId = $state<string | null>(null);
let loading = $state(false);

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

export async function loadThreads(identity: string): Promise<void> {
	loading = true;
	try {
		threads = await getAgentBackend().threadsList(identity);
	} finally {
		loading = false;
	}
}

export async function loadAllThreads(): Promise<void> {
	allThreads = await getAgentBackend().threadsListAll();
}

export async function createThread(identity: string): Promise<ThreadSummary> {
	const thread = await getAgentBackend().threadCreate(identity);
	threads = [thread, ...threads];
	activeThreadId = thread.id;
	return thread;
}

export function selectThread(threadId: string): void {
	activeThreadId = threadId;
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
	threads = threads.filter((t) => t.id !== threadId);
	allThreads = allThreads.filter((t) => t.id !== threadId);
	if (activeThreadId === threadId) activeThreadId = threads[0]?.id ?? null;
}

/** Apply a title change pushed by the backend (auto-generated titles). */
export function applyThreadTitle(threadId: string, title: string): void {
	threads = threads.map((t) => (t.id === threadId ? { ...t, title } : t));
	allThreads = allThreads.map((t) => (t.id === threadId ? { ...t, title } : t));
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
