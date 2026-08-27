//! Agent panel + conversation state (design 01 §4).
//!
//! Owns: panel geometry per identity (localStorage), the event-stream
//! subscription per identity, and the live per-thread message state
//! reconciled from backend events. Tool card state lives in `toolCalls`.

import {
	tauriBackend,
	type AgentBackend
} from '$lib/ipc/agent-backend';
import { appendDraft } from '$lib/components/agent/composer-draft.svelte';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { applyThreadTitle } from './agent-threads.svelte';
import type {
	AgentEvent,
	AgentSendOpts,
	PathMessage,
	StoredMessage,
	ThreadSnapshot,
	ToolCallStatus,
	ToolCallView,
	Usage
} from '$lib/ipc/agent';

// ---------------------------------------------------------------------------
// Backend seam (mock only under /preview, design 01 §4.1)
// ---------------------------------------------------------------------------

let backend: AgentBackend = tauriBackend;

export function getAgentBackend(): AgentBackend {
	return backend;
}

/** Swap the backend (preview pages only; restore on destroy). */
export function setAgentBackend(b: AgentBackend): void {
	backend = b;
}

// ---------------------------------------------------------------------------
// Panel state per identity (localStorage `reach-agent-panel:{identity}`)
// ---------------------------------------------------------------------------

export interface AgentPanelState {
	open: boolean;
	panelWidth: number;
	threadsWidth: number;
	threadsCollapsed: boolean;
	detached: boolean;
}

const PANEL_DEFAULTS: AgentPanelState = {
	open: false,
	panelWidth: 480,
	threadsWidth: 200,
	threadsCollapsed: false,
	detached: false
};

const PANEL_KEY_PREFIX = 'reach-agent-panel:';

let panels = $state<Record<string, AgentPanelState>>({});

function panelKey(identity: string): string {
	return `${PANEL_KEY_PREFIX}${identity}`;
}

/**
 * Pure read — safe inside `$derived`/template expressions. Missing entries
 * fall back to defaults merged with localStorage without writing to `panels`;
 * the entry is only materialized by `updatePanelState`.
 */
export function getPanelState(identity: string): AgentPanelState {
	const existing = panels[identity];
	if (existing) return existing;
	let stored: Partial<AgentPanelState> = {};
	try {
		stored = JSON.parse(localStorage.getItem(panelKey(identity)) ?? '{}');
	} catch {
		stored = {};
	}
	return { ...PANEL_DEFAULTS, ...stored };
}

export function updatePanelState(identity: string, patch: Partial<AgentPanelState>): void {
	const current = getPanelState(identity);
	panels[identity] = { ...current, ...patch };
	try {
		localStorage.setItem(panelKey(identity), JSON.stringify(panels[identity]));
	} catch {
		/* quota errors are non-fatal */
	}
}

export function togglePanel(identity: string): void {
	const p = getPanelState(identity);
	updatePanelState(identity, { open: !p.open });
}

// ---------------------------------------------------------------------------
// Detached pop-out window (design 01 §1.2)
// ---------------------------------------------------------------------------

/**
 * Window label for an identity's detached panel. Tauri labels allow only
 * alphanumerics plus `- / : _`, so percent-encode the identity and flatten
 * anything still unsafe (`%`, `.`, …) to `_`.
 */
export function agentWindowLabel(identity: string): string {
	return `agent-${encodeURIComponent(identity).replace(/[^A-Za-z0-9_-]/g, '_')}`;
}

/**
 * Open the panel as a detached window. If a window for this identity already
 * exists, focus it instead of creating a duplicate.
 */
export async function popOutPanel(identity: string): Promise<void> {
	const panel = getPanelState(identity);
	updatePanelState(identity, { detached: true, open: true });
	const label = agentWindowLabel(identity);
	try {
		const existing = await WebviewWindow.getByLabel(label);
		if (existing) {
			await existing.setFocus();
			return;
		}
		const win = new WebviewWindow(label, {
			url: `/?agent=${encodeURIComponent(identity)}`,
			title: `Agent — ${identity}`,
			width: panel.panelWidth,
			height: Math.max(window.innerHeight, 480),
			minWidth: 360,
			minHeight: 400
		});
		void win.once('tauri://error', (e) => {
			console.error('Failed to create detached agent window:', e);
			updatePanelState(identity, { detached: false });
		});
	} catch (err) {
		// Non-Tauri env (browser preview) or permission failure: stay docked.
		console.error('Failed to open detached agent window:', err);
		updatePanelState(identity, { detached: false });
	}
}

let storageSyncInitialized = false;

/**
 * Cross-window panel-state sync (design 01 §1.2): writes to
 * `reach-agent-panel:*` in one window are mirrored into this window's
 * reactive map via the `storage` event (which only fires in *other* windows
 * of the same origin). Call once per window.
 */
export function initPanelStorageSync(): void {
	if (storageSyncInitialized || typeof window === 'undefined') return;
	storageSyncInitialized = true;
	window.addEventListener('storage', (e) => {
		if (!e.key?.startsWith(PANEL_KEY_PREFIX)) return;
		const identity = e.key.slice(PANEL_KEY_PREFIX.length);
		if (!identity) return;
		if (e.newValue === null) {
			delete panels[identity];
			return;
		}
		let stored: Partial<AgentPanelState>;
		try {
			stored = JSON.parse(e.newValue);
		} catch {
			return;
		}
		panels[identity] = { ...PANEL_DEFAULTS, ...stored };
	});
}

/**
 * Detached-window close hook (design 01 §1.2): closing the pop-out window
 * docks the panel back in the closed state — persists
 * `{detached: false, open: false}` to localStorage, which the main window
 * picks up via `initPanelStorageSync`. Returns a cleanup fn.
 */
export function initDetachedWindowCloseHook(identity: string): () => void {
	const persistClosed = () => {
		try {
			const key = panelKey(identity);
			const stored = JSON.parse(localStorage.getItem(key) ?? '{}') as Partial<AgentPanelState>;
			localStorage.setItem(key, JSON.stringify({ ...stored, detached: false, open: false }));
		} catch {
			/* non-fatal */
		}
	};
	window.addEventListener('beforeunload', persistClosed);
	let unlisten: (() => void) | null = null;
	let cancelled = false;
	try {
		getCurrentWindow()
			.onCloseRequested(persistClosed)
			.then((u) => {
				if (cancelled) u();
				else unlisten = u;
			})
			.catch(() => {});
	} catch {
		/* non-Tauri env */
	}
	return () => {
		cancelled = true;
		window.removeEventListener('beforeunload', persistClosed);
		unlisten?.();
	};
}

// ---------------------------------------------------------------------------
// Identity tracking: connectionId -> identity (from ssh_connect results)
// ---------------------------------------------------------------------------

let identities = $state<Record<string, string>>({});

export function registerConnectionIdentity(connectionId: string, identity: string): void {
	identities[connectionId] = identity;
}

export function identityForConnection(connectionId: string | undefined): string | null {
	return connectionId ? (identities[connectionId] ?? null) : null;
}

export function forgetConnection(connectionId: string): void {
	delete identities[connectionId];
}

// ---------------------------------------------------------------------------
// Live thread state
// ---------------------------------------------------------------------------

export interface QueuedMessage {
	text: string;
}

export interface ThreadRuntime {
	messages: PathMessage[];
	running: boolean;
	/** Message id currently streaming (assistant). */
	streamingMessageId: string | null;
	/** Queued message waiting for the round boundary (max 1). */
	queued: QueuedMessage | null;
	/** Live tool call views, keyed by tool_call id. */
	toolCalls: Record<string, ToolCallView>;
	/** Last usage seen (drives the context ring). */
	lastUsage: Usage | null;
	/** Last error, cleared on next send. */
	error: string | null;
	/** Whether the runtime has been seeded from a backend snapshot. */
	loaded: boolean;
}

let runtimes = $state<Record<string, ThreadRuntime>>({});

/**
 * Get-or-create the runtime entry — the single materialization point for the
 * write path (event handlers, actions, snapshot seeding). Writes to `$state`,
 * so it must NEVER be called from `$derived`/template expressions; readers
 * use `getThreadRuntime` instead.
 */
function ensureThreadRuntime(threadId: string): ThreadRuntime {
	if (!runtimes[threadId]) {
		runtimes[threadId] = {
			messages: [],
			running: false,
			streamingMessageId: null,
			queued: null,
			toolCalls: {},
			lastUsage: null,
			error: null,
			loaded: false
		};
	}
	return runtimes[threadId];
}

/**
 * Pure read for render code (`$derived`/template) — never creates the entry.
 * `null` means no runtime exists yet (thread not loaded, no events seen);
 * the entry materializes via `ensureThreadRuntime` once data arrives.
 */
export function getThreadRuntime(threadId: string): ThreadRuntime | null {
	return runtimes[threadId] ?? null;
}

/**
 * Monotonic status order for snapshot merging: a snapshot is a point-in-time
 * DB read, so a live view that has already moved further (e.g. an approval
 * resolved to running/success between the read and application) must not be
 * downgraded back. Terminal states all share rank 3.
 */
const STATUS_RANK: Record<ToolCallStatus, number> = {
	streaming: 0,
	pending_approval: 1,
	running: 2,
	success: 3,
	failed: 3,
	rejected: 3,
	cancelled: 3
};

/** Seed from a backend snapshot (panel open / thread switch). */
export function applySnapshot(threadId: string, snapshot: ThreadSnapshot, running: boolean) {
	const rt = ensureThreadRuntime(threadId);
	rt.messages = snapshot.messages;
	rt.running = running;
	rt.loaded = true;
	rt.error = null;
	rt.streamingMessageId = null;
	// Rebuild live tool call views from persisted content.
	const prev = rt.toolCalls;
	rt.toolCalls = {};
	for (const msg of snapshot.messages) {
		for (const block of msg.content) {
			if (block.type === 'tool_call') {
				const live = prev[block.id];
				if (live && STATUS_RANK[live.status] > STATUS_RANK[block.status]) {
					rt.toolCalls[block.id] = live;
					continue;
				}
				rt.toolCalls[block.id] = {
					id: block.id,
					messageId: msg.id,
					name: block.name,
					argsJson: JSON.stringify(block.args ?? {}, null, 2),
					status: block.status,
					result: block.result,
					warnings: block.warnings
				};
			}
		}
	}
	if (snapshot.thread.lastUsage) rt.lastUsage = snapshot.thread.lastUsage;
}

// ---------------------------------------------------------------------------
// Event stream subscription (per identity; design 01 §5)
// ---------------------------------------------------------------------------

const subscriptions: Record<string, () => void> = {};

export function subscribeIdentity(identity: string): void {
	if (subscriptions[identity]) return;
	let cancelled = false;
	let unlisten: (() => void) | null = null;
	const wrapper = () => {
		cancelled = true;
		unlisten?.();
	};
	subscriptions[identity] = wrapper;
	const res = backend.onEvent(identity, handleEvent);
	if (res instanceof Promise) {
		res.then((u) => {
			if (cancelled) u();
			else unlisten = u;
		});
	} else {
		unlisten = res;
	}
}

export function unsubscribeIdentity(identity: string): void {
	subscriptions[identity]?.();
	delete subscriptions[identity];
}

function handleEvent(e: AgentEvent): void {
	switch (e.kind) {
		case 'text_delta':
		case 'thinking_delta': {
			const rt = ensureThreadRuntime(e.threadId);
			// Ensure a streaming placeholder message exists.
			let msg = rt.messages.find((m) => m.id === e.messageId);
			if (!msg) {
				msg = {
					id: e.messageId,
					threadId: e.threadId,
					branchIndex: 0,
					seq: Number.MAX_SAFE_INTEGER,
					role: 'assistant',
					content: [],
					createdAt: Date.now(),
					branch: { index: 1, count: 1 }
				} as PathMessage;
				rt.messages.push(msg);
			}
			rt.streamingMessageId = e.messageId;
			rt.running = true;
			const blockType = e.kind === 'text_delta' ? 'text' : 'thinking';
			let block = msg.content.find((b) => b.type === blockType);
			if (!block) {
				block =
					blockType === 'text'
						? { type: 'text', text: '' }
						: { type: 'thinking', text: '' };
				msg.content.push(block);
			}
			(block as { text: string }).text += e.delta;
			break;
		}
		case 'tool_call': {
			const rt = ensureThreadRuntime(e.threadId);
			const existing = rt.toolCalls[e.toolCall.id];
			if (existing) {
				// Mutate in place: replacing the view object bumps the record's
				// source, which propagates down the bare-getter prop chain
				// (Svelte 5 read-only props) into ToolTerminal's xterm effect and
				// recreates the terminal, wiping live output on every status change.
				Object.assign(existing, e.toolCall);
			} else {
				rt.toolCalls[e.toolCall.id] = e.toolCall;
			}
			rt.running = true;
			// Attach to the assistant message if it exists yet.
			const msg = rt.messages.find((m) => m.id === e.toolCall.messageId);
			if (msg && !msg.content.some((b) => b.type === 'tool_call' && b.id === e.toolCall.id)) {
				msg.content.push({
					type: 'tool_call',
					id: e.toolCall.id,
					name: e.toolCall.name,
					args: safeParse(e.toolCall.argsJson),
					status: e.toolCall.status,
					result: e.toolCall.result
				});
			} else if (msg) {
				for (const b of msg.content) {
					if (b.type === 'tool_call' && b.id === e.toolCall.id) {
						b.status = e.toolCall.status;
						b.result = e.toolCall.result;
					}
				}
			}
			break;
		}
		case 'tool_call_args_delta': {
			const rt = ensureThreadRuntime(e.threadId);
			const view = rt.toolCalls[e.toolCallId];
			if (view) view.argsJson += e.argsJsonDelta;
			break;
		}
		case 'approval_needed': {
			const rt = ensureThreadRuntime(e.threadId);
			const view = rt.toolCalls[e.approval.toolCallId];
			if (view) {
				view.status = 'pending_approval';
				view.warnings = e.approval.warnings;
			}
			break;
		}
		case 'user_message': {
			const rt = ensureThreadRuntime(e.threadId);
			rt.queued = null;
			if (!rt.messages.some((m) => m.id === e.message.id)) {
				rt.messages.push({
					...e.message,
					branch: { index: 1, count: 1 }
				} as PathMessage);
			}
			rt.running = true;
			rt.error = null;
			break;
		}
		case 'usage': {
			ensureThreadRuntime(e.threadId).lastUsage = e.usage;
			break;
		}
		case 'message_done': {
			const rt = ensureThreadRuntime(e.threadId);
			if (rt.streamingMessageId === e.messageId) rt.streamingMessageId = null;
			break;
		}
		case 'error': {
			const rt = ensureThreadRuntime(e.threadId);
			rt.running = false;
			rt.error = e.message;
			rt.streamingMessageId = null;
			// The backend drops the queue on abnormal exit; return the text to
			// the composer so the user doesn't lose it.
			if (rt.queued) {
				appendDraft(e.threadId, rt.queued.text);
				rt.queued = null;
			}
			break;
		}
		case 'cancelled': {
			const rt = ensureThreadRuntime(e.threadId);
			rt.running = false;
			rt.streamingMessageId = null;
			break;
		}
		case 'terminal_output': {
			appendTerminalOutput(e.toolCallId, e.dataB64);
			break;
		}
		case 'title_updated': {
			applyThreadTitle(e.threadId, e.title);
			break;
		}
		case 'thread_updated': {
			// Threads lists refresh on demand; nothing to do here.
			break;
		}
	}
}

function safeParse(json: string): unknown {
	try {
		return JSON.parse(json);
	} catch {
		return json;
	}
}

// ---------------------------------------------------------------------------
// Terminal output buffer + fan-out (card xterm views subscribe per tool_call_id)
// ---------------------------------------------------------------------------

/** Decoded-byte cap per tool call; oldest chunks are dropped past the cap. */
const TERMINAL_BUFFER_LIMIT = 256 * 1024;

interface TerminalBuffer {
	chunks: string[];
	bytes: number;
}

const terminalBuffers: Record<string, TerminalBuffer> = {};
const terminalListeners: Record<string, (dataB64: string) => void> = {};

function appendTerminalOutput(toolCallId: string, dataB64: string): void {
	let buf = terminalBuffers[toolCallId];
	if (!buf) {
		buf = { chunks: [], bytes: 0 };
		terminalBuffers[toolCallId] = buf;
	}
	buf.chunks.push(dataB64);
	buf.bytes += (dataB64.length * 3) / 4; // base64 -> decoded bytes (approx.)
	while (buf.bytes > TERMINAL_BUFFER_LIMIT && buf.chunks.length > 1) {
		const dropped = buf.chunks.shift();
		if (dropped) buf.bytes -= (dropped.length * 3) / 4;
	}
	terminalListeners[toolCallId]?.(dataB64);
}

export function onTerminalOutput(
	toolCallId: string,
	cb: (dataB64: string) => void
): () => void {
	// Replay buffered output so a (re)mounted view catches up with everything
	// emitted while it was gone (thread switch, remount, pre-mount race), then
	// go live. Synchronous, so no chunk can interleave between the two phases.
	const buf = terminalBuffers[toolCallId];
	if (buf) {
		for (const chunk of buf.chunks) cb(chunk);
	}
	terminalListeners[toolCallId] = cb;
	return () => {
		if (terminalListeners[toolCallId] === cb) delete terminalListeners[toolCallId];
	};
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

export async function agentSendMessage(
	identity: string,
	threadId: string,
	text: string,
	opts: AgentSendOpts
): Promise<void> {
	const rt = ensureThreadRuntime(threadId);
	rt.error = null;
	const status = await backend.sendMessage(identity, threadId, text, opts);
	if (status === 'queued') {
		rt.queued = { text };
	} else if (status === 'queued_full') {
		// Queue was full: the send had no effect (design 01 §3.5).
	} else {
		rt.running = true;
	}
}

export async function agentCancelRun(threadId: string): Promise<void> {
	const rt = ensureThreadRuntime(threadId);
	// A queued message goes back to the composer (like the queued bar's edit
	// action) instead of being discarded.
	if (rt.queued) appendDraft(threadId, rt.queued.text);
	rt.queued = null;
	await backend.dequeue(threadId).catch(() => {});
	await backend.cancel(threadId);
	rt.running = false;
}

/** Force-send the queued message (design 01 §3.5 "send now"). The backend
 * atomically supersedes the queue, cancels the run, waits for it to exit,
 * then starts a fresh run. On timeout the message is re-queued backend-side;
 * restore the bar so the UI matches. */
export async function agentSendNow(
	identity: string,
	threadId: string,
	text: string,
	opts: AgentSendOpts
): Promise<void> {
	const rt = ensureThreadRuntime(threadId);
	rt.error = null;
	rt.queued = null;
	try {
		await backend.sendNow(identity, threadId, text, opts);
		rt.running = true;
	} catch (e) {
		rt.queued = { text };
		rt.error = String(e);
	}
}

/** Discard the queued message (backend queue + UI bar). */
export async function agentDequeueMessage(threadId: string): Promise<void> {
	ensureThreadRuntime(threadId).queued = null;
	await backend.dequeue(threadId).catch(() => {});
}

export async function agentApproveCall(toolCallId: string, approved: boolean): Promise<void> {
	await backend.approve(toolCallId, approved);
}

export async function agentStopTerminal(toolCallId: string): Promise<void> {
	await backend.terminalStop(toolCallId);
}

export async function agentResizeTerminal(
	toolCallId: string,
	cols: number,
	rows: number
): Promise<void> {
	await backend.terminalResize(toolCallId, cols, rows);
}

/** Edit a user message and fork a new branch (design 01 §2.4). */
export async function agentEditAndFork(
	identity: string,
	threadId: string,
	messageId: string,
	newContent: string,
	opts: AgentSendOpts
): Promise<void> {
	const rt = ensureThreadRuntime(threadId);
	rt.error = null;
	await backend.threadEditMessage(identity, threadId, messageId, newContent, opts);
	rt.running = true;
}

/** Switch the displayed branch and replace the visible path. */
export async function agentSwitchBranch(
	threadId: string,
	atMessageId: string,
	direction: 'prev' | 'next'
): Promise<void> {
	const snapshot = await backend.threadSetActiveBranch(threadId, atMessageId, direction);
	const rt = ensureThreadRuntime(threadId);
	applySnapshot(threadId, snapshot, rt.running);
}

/** Load a thread's state (panel open / thread switch). */
export async function loadThread(threadId: string): Promise<ThreadSnapshot> {
	const state = await backend.threadState(threadId);
	applySnapshot(threadId, state, state.running);
	return state;
}
