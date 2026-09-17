//! Agent panel + conversation state (design 01 §4).
//!
//! Owns: panel geometry per owner scope (localStorage), the event-stream
//! subscription per scope, and the live per-thread message state
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
	DiffPreview,
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
// Panel state per owner scope (localStorage `reach-agent-panel:{scope}`)
// Scope: "session:<uuid>" (saved session) | "link:<identity>" (quick connect).
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

function panelKey(scope: string): string {
	return `${PANEL_KEY_PREFIX}${scope}`;
}

/**
 * Pure read — safe inside `$derived`/template expressions. Missing entries
 * fall back to defaults merged with localStorage without writing to `panels`;
 * the entry is only materialized by `updatePanelState`.
 */
export function getPanelState(scope: string): AgentPanelState {
	const existing = panels[scope];
	if (existing) return existing;
	let stored: Partial<AgentPanelState> = {};
	try {
		stored = JSON.parse(localStorage.getItem(panelKey(scope)) ?? '{}');
	} catch {
		stored = {};
	}
	return { ...PANEL_DEFAULTS, ...stored };
}

export function updatePanelState(scope: string, patch: Partial<AgentPanelState>): void {
	const current = getPanelState(scope);
	panels[scope] = { ...current, ...patch };
	try {
		localStorage.setItem(panelKey(scope), JSON.stringify(panels[scope]));
	} catch {
		/* quota errors are non-fatal */
	}
}

export function togglePanel(scope: string): void {
	const p = getPanelState(scope);
	updatePanelState(scope, { open: !p.open });
}

// ---------------------------------------------------------------------------
// Detached pop-out window (design 01 §1.2)
// ---------------------------------------------------------------------------

/**
 * Window label for a scope's detached panel. Tauri labels allow only
 * alphanumerics plus `- / : _`, so percent-encode the scope and flatten
 * anything still unsafe (`%`, `.`, …) to `_`. Keyed by scope (not identity)
 * so the pop-out survives session connection-detail edits.
 */
export function agentWindowLabel(scope: string): string {
	return `agent-${encodeURIComponent(scope).replace(/[^A-Za-z0-9_-]/g, '_')}`;
}

/**
 * Open the panel as a detached window. If a window for this scope already
 * exists, focus it instead of creating a duplicate.
 */
export async function popOutPanel(scope: string): Promise<void> {
	const panel = getPanelState(scope);
	updatePanelState(scope, { detached: true, open: true });
	const label = agentWindowLabel(scope);
	try {
		const existing = await WebviewWindow.getByLabel(label);
		if (existing) {
			await existing.setFocus();
			return;
		}
		const win = new WebviewWindow(label, {
			url: `/?agent=${encodeURIComponent(scope)}`,
			title: `Agent — ${scope.startsWith('link:') ? scope.slice(5) : scope}`,
			width: panel.panelWidth,
			height: Math.max(window.innerHeight, 480),
			minWidth: 360,
			minHeight: 400,
			// Undecorated: AgentWindow renders a custom title bar styled after
			// the main window's (design 01 §1.2).
			decorations: false
		});
		void win.once('tauri://error', (e) => {
			console.error('Failed to create detached agent window:', e);
			updatePanelState(scope, { detached: false });
		});
	} catch (err) {
		// Non-Tauri env (browser preview) or permission failure: stay docked.
		console.error('Failed to open detached agent window:', err);
		updatePanelState(scope, { detached: false });
	}
}

/** Latched by `dockBackPanel` so the window-close hook keeps the panel open. */
let dockBackRequested = false;

/**
 * Dock the detached panel back into the main window, keeping it open. The
 * latch makes `initDetachedWindowCloseHook` persist `{ detached: false,
 * open: true }` instead of the plain-close semantics (design 01 §1.2).
 */
export async function dockBackPanel(scope: string): Promise<void> {
	updatePanelState(scope, { detached: false, open: true });
	dockBackRequested = true;
	try {
		const win = await WebviewWindow.getByLabel(agentWindowLabel(scope));
		await win?.close();
	} catch (err) {
		dockBackRequested = false;
		console.error('Failed to close detached agent window:', err);
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
		const scope = e.key.slice(PANEL_KEY_PREFIX.length);
		if (!scope) return;
		if (e.newValue === null) {
			delete panels[scope];
			return;
		}
		let stored: Partial<AgentPanelState>;
		try {
			stored = JSON.parse(e.newValue);
		} catch {
			return;
		}
		panels[scope] = { ...PANEL_DEFAULTS, ...stored };
	});
}

/**
 * Detached-window close hook (design 01 §1.2): closing the pop-out window
 * docks the panel back in the closed state — persists
 * `{detached: false, open: false}` to localStorage, which the main window
 * picks up via `initPanelStorageSync`. When the close comes from
 * `dockBackPanel`, the latch keeps the panel open instead. Returns a
 * cleanup fn.
 */
export function initDetachedWindowCloseHook(scope: string): () => void {
	const persistClosed = () => {
		try {
			const key = panelKey(scope);
			const stored = JSON.parse(localStorage.getItem(key) ?? '{}') as Partial<AgentPanelState>;
			localStorage.setItem(key, JSON.stringify({ ...stored, detached: false, open: dockBackRequested }));
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
// Connection tracking: connectionId -> owner scope (from ssh_connect's
// ConnectionInfo.agent_scope)
// ---------------------------------------------------------------------------

let connections = $state<Record<string, string>>({});

export function registerConnectionScope(connectionId: string, scope: string): void {
	connections[connectionId] = scope;
}

/** Owner scope of a connection ("session:<uuid>" | "link:<identity>"). */
export function scopeForConnection(connectionId: string | undefined): string | null {
	return connectionId ? (connections[connectionId] ?? null) : null;
}

export function forgetConnection(connectionId: string): void {
	delete connections[connectionId];
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
	/** Structured diff previews for streaming write/edit calls, keyed by tool_call id. */
	previews: Record<string, DiffPreview>;
	/** Latest valid-JSON patch of a streaming call's arguments, keyed by tool_call id. */
	argsPatched: Record<string, string>;
	/** Payloads captured from approval_needed, keyed by tool_call id. Kept at
	 *  terminal status so rejected write/edit cards keep their diff. */
	approvalPayloads: Record<string, unknown>;
	/** Last usage seen (drives the context ring). */
	lastUsage: Usage | null;
	/** Last error, cleared on next send. */
	error: string | null;
	/** Whether the runtime has been seeded from a backend snapshot. */
	loaded: boolean;
	/** The stream epoch at the last snapshot application; see streamEpoch. */
	loadedEpoch: number;
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
			previews: {},
			argsPatched: {},
			approvalPayloads: {},
			lastUsage: null,
			error: null,
			loaded: false,
			loadedEpoch: 0
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

// Tool-call ids are globally unique, so the per-call lookups below simply
// scan the loaded runtimes for the owning thread.

/** Live structured diff preview of a streaming write/edit call. */
export function getToolPreview(toolCallId: string): DiffPreview | null {
	for (const rt of Object.values(runtimes)) {
		const preview = rt.previews[toolCallId];
		if (preview) return preview;
	}
	return null;
}

/** Latest valid-JSON patch of a streaming call's args — preferred over the
 *  raw, possibly unterminated `argsJson` accumulation. */
export function getPatchedArgs(toolCallId: string): string | null {
	for (const rt of Object.values(runtimes)) {
		const patched = rt.argsPatched[toolCallId];
		if (patched) return patched;
	}
	return null;
}

/** Payload captured from the call's approval_needed event, if any. */
export function getApprovalPayload(toolCallId: string): unknown {
	for (const rt of Object.values(runtimes)) {
		if (toolCallId in rt.approvalPayloads) return rt.approvalPayloads[toolCallId];
	}
	return undefined;
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

function isTerminalCall(status: ToolCallStatus): boolean {
	return STATUS_RANK[status] >= STATUS_RANK.success;
}

/** Seed from a backend snapshot (panel open / thread switch). */
export function applySnapshot(
	threadId: string,
	snapshot: ThreadSnapshot & { previews?: Record<string, DiffPreview> },
	running: boolean
) {
	const rt = ensureThreadRuntime(threadId);
	rt.messages = snapshot.messages;
	rt.loadedEpoch = streamEpoch;
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
	// Merge live previews carried by the snapshot (streaming calls only), then
	// drop streaming artifacts for calls that are terminal or no longer shown.
	if (snapshot.previews) {
		for (const [id, preview] of Object.entries(snapshot.previews)) {
			rt.previews[id] = preview;
		}
	}
	for (const id of Object.keys(rt.previews)) {
		const view = rt.toolCalls[id];
		if (!view || isTerminalCall(view.status)) delete rt.previews[id];
	}
	for (const id of Object.keys(rt.argsPatched)) {
		const view = rt.toolCalls[id];
		if (!view || isTerminalCall(view.status)) delete rt.argsPatched[id];
	}
	if (snapshot.thread.lastUsage) rt.lastUsage = snapshot.thread.lastUsage;
}

// ---------------------------------------------------------------------------
// Event stream subscription (per scope; design 01 §5)
// ---------------------------------------------------------------------------

const subscriptions: Record<string, () => void> = {};

/**
 * Bumped on every (re)subscription to a scope's event stream. While
 * subscribed, every runtime of the scope stays live-synced by the stream
 * (handleEvent routes by thread id), so re-applying a persisted snapshot over
 * such a runtime is redundant churn (it remounts tool cards) and can even
 * overwrite newer live state. A runtime whose loadedEpoch lags the current
 * epoch may have missed events (panel closed / resubscribed) and must reload.
 */
let streamEpoch = 0;

export function subscribeScope(scope: string): void {
	if (subscriptions[scope]) return;
	streamEpoch++;
	let cancelled = false;
	let unlisten: (() => void) | null = null;
	const wrapper = () => {
		cancelled = true;
		unlisten?.();
	};
	subscriptions[scope] = wrapper;
	const res = backend.onEvent(scope, handleEvent);
	if (res instanceof Promise) {
		res.then((u) => {
			if (cancelled) u();
			else unlisten = u;
		});
	} else {
		unlisten = res;
	}
}

export function unsubscribeScope(scope: string): void {
	subscriptions[scope]?.();
	delete subscriptions[scope];
}

/// Create (or find) the streaming placeholder for one assistant round. The
/// backend announces every round with `message_start`; content events keep
/// calling this lazily as a safety net so no block can strand without a
/// host message.
function ensureStreamingMessage(rt: ThreadRuntime, threadId: string, messageId: string): PathMessage {
	let msg = rt.messages.find((m) => m.id === messageId);
	if (!msg) {
		msg = {
			id: messageId,
			threadId,
			branchIndex: 0,
			seq: Number.MAX_SAFE_INTEGER,
			role: 'assistant',
			content: [],
			createdAt: Date.now(),
			branch: { index: 1, count: 1 }
		} as PathMessage;
		rt.messages.push(msg);
	}
	return msg;
}

/// Remove the round's streaming placeholder if it never gained content
/// (empty round, provider error, or cancel before the first delta) —
/// otherwise it would linger as a permanent typing indicator.
function dropEmptyStreamingMessage(rt: ThreadRuntime, messageId: string | null): void {
	if (!messageId) return;
	const msg = rt.messages.find((m) => m.id === messageId);
	if (msg && msg.content.length === 0) {
		rt.messages = rt.messages.filter((m) => m.id !== messageId);
	}
}

/// Drop tool calls that never reached a terminal state when a run dies
/// abnormally: the backend persists no record of un-executed calls, so the
/// live view converges to what a reload will show — no phantom streaming
/// cards. Execute-phase cancels arrive as terminal tool_call events before
/// the terminal run event (channel ordering), so they survive the sweep.
function dropUnfinishedToolCalls(rt: ThreadRuntime): void {
	for (const [id, view] of Object.entries(rt.toolCalls)) {
		if (isTerminalCall(view.status)) continue;
		const msg = rt.messages.find((m) => m.id === view.messageId);
		if (msg) {
			msg.content = msg.content.filter((b) => !(b.type === 'tool_call' && b.id === id));
		}
		delete rt.toolCalls[id];
		delete rt.previews[id];
		delete rt.argsPatched[id];
	}
}

function handleEvent(e: AgentEvent): void {
	switch (e.kind) {
		case 'message_start': {
			const rt = ensureThreadRuntime(e.threadId);
			ensureStreamingMessage(rt, e.threadId, e.messageId);
			rt.streamingMessageId = e.messageId;
			rt.running = true;
			break;
		}
		case 'text_delta':
		case 'thinking_delta': {
			const rt = ensureThreadRuntime(e.threadId);
			const msg = ensureStreamingMessage(rt, e.threadId, e.messageId);
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
			// Streaming artifacts are obsolete once the call settles.
			if (isTerminalCall(e.toolCall.status)) {
				delete rt.previews[e.toolCall.id];
				delete rt.argsPatched[e.toolCall.id];
			}
			rt.running = true;
			// Attach to the round's assistant message. message_start created it
			// up front; ensureStreamingMessage is the safety net so the card
			// (and its approval UI) always has a host message.
			const msg = ensureStreamingMessage(rt, e.threadId, e.toolCall.messageId);
			if (!msg.content.some((b) => b.type === 'tool_call' && b.id === e.toolCall.id)) {
				msg.content.push({
					type: 'tool_call',
					id: e.toolCall.id,
					name: e.toolCall.name,
					args: safeParse(e.toolCall.argsJson),
					status: e.toolCall.status,
					result: e.toolCall.result
				});
			} else {
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
		case 'tool_call_args_patched': {
			const rt = ensureThreadRuntime(e.threadId);
			rt.argsPatched[e.toolCallId] = e.argsJson;
			break;
		}
		case 'tool_call_preview': {
			const rt = ensureThreadRuntime(e.threadId);
			rt.previews[e.toolCallId] = e.preview;
			break;
		}
		case 'approval_needed': {
			const rt = ensureThreadRuntime(e.threadId);
			const view = rt.toolCalls[e.approval.toolCallId];
			if (view) {
				view.status = 'pending_approval';
				view.warnings = e.approval.warnings;
			}
			if (e.approval.payload !== undefined) {
				rt.approvalPayloads[e.approval.toolCallId] = e.approval.payload;
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
			dropEmptyStreamingMessage(rt, e.messageId);
			if (e.metadata) {
				const msg = rt.messages.find((m) => m.id === e.messageId);
				if (msg) msg.metadata = e.metadata;
			}
			if (rt.streamingMessageId === e.messageId) rt.streamingMessageId = null;
			break;
		}
		case 'run_end': {
			// Normal run exit (no more rounds, queue empty); settles the
			// composer back to the send button. Abnormal exits use
			// error/cancelled instead.
			const rt = ensureThreadRuntime(e.threadId);
			rt.running = false;
			rt.streamingMessageId = null;
			break;
		}
		case 'error': {
			const rt = ensureThreadRuntime(e.threadId);
			rt.running = false;
			rt.error = e.message;
			dropUnfinishedToolCalls(rt);
			dropEmptyStreamingMessage(rt, rt.streamingMessageId);
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
			dropUnfinishedToolCalls(rt);
			dropEmptyStreamingMessage(rt, rt.streamingMessageId);
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

/**
 * Cap on the number of live buffers (hard ceiling ≈ 1000 × 341 KB of base64).
 * Past it the oldest buffers are evicted; a settled call then falls back to
 * its persisted ui_payload projection on remount.
 */
const TERMINAL_BUFFER_MAX_COUNT = 1000;

// Map (not Object) so iteration order is guaranteed for every key shape;
// eviction relies on it. Buffers are the replay source for (re)mounted xterm
// views; a running evicted call silently re-creates its buffer on the next
// chunk. Live streaming goes through `terminalListeners`, which eviction
// must not touch.
const terminalBuffers = new Map<string, TerminalBuffer>();
const terminalListeners: Record<string, (dataB64: string) => void> = {};

/** Drop oldest-inserted buffers past the count cap, skipping calls that
 * currently have a mounted viewer (their replay source is in active use). */
function evictTerminalBuffersIfNeeded(): void {
	while (terminalBuffers.size > TERMINAL_BUFFER_MAX_COUNT) {
		let deleted = false;
		for (const key of terminalBuffers.keys()) {
			if (terminalListeners[key] !== undefined) continue;
			terminalBuffers.delete(key);
			deleted = true;
			break;
		}
		if (!deleted) return; // everything left is being watched
	}
}

function appendTerminalOutput(toolCallId: string, dataB64: string): void {
	let buf = terminalBuffers.get(toolCallId);
	if (!buf) {
		buf = { chunks: [], bytes: 0 };
		terminalBuffers.set(toolCallId, buf);
		evictTerminalBuffersIfNeeded();
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
	const buf = terminalBuffers.get(toolCallId);
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
	scope: string,
	threadId: string,
	text: string,
	opts: AgentSendOpts
): Promise<void> {
	const rt = ensureThreadRuntime(threadId);
	rt.error = null;
	const status = await backend.sendMessage(scope, threadId, text, opts);
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
	scope: string,
	threadId: string,
	text: string,
	opts: AgentSendOpts
): Promise<void> {
	const rt = ensureThreadRuntime(threadId);
	rt.error = null;
	rt.queued = null;
	try {
		await backend.sendNow(scope, threadId, text, opts);
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
	scope: string,
	threadId: string,
	messageId: string,
	newContent: string,
	opts: AgentSendOpts
): Promise<void> {
	const rt = ensureThreadRuntime(threadId);
	rt.error = null;
	const snapshot = await backend.threadEditMessage(scope, threadId, messageId, newContent, opts);
	// The fork switched the active branch backend-side; the snapshot carries
	// the new path with correct branch info. Run events that already arrived
	// only added placeholders, which the next delta lazily re-creates.
	applySnapshot(threadId, snapshot, true);
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
export async function loadThread(threadId: string): Promise<ThreadSnapshot | null> {
	const rt = getThreadRuntime(threadId);
	// In-sync runtime: the event stream has kept it current since load, so
	// applying a persisted snapshot over it would only churn the view
	// (remounting xterm cards) and could even overwrite newer live state.
	if (rt?.loaded && rt.loadedEpoch === streamEpoch) return null;
	const state = await backend.threadState(threadId);
	applySnapshot(threadId, state, state.running);
	return state;
}
