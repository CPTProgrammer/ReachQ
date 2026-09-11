//! In-memory AgentBackend for the dev-only `/preview/agent` route
//! (design 01 §4.1). Threads, messages, providers, and approvals all live in
//! the browser; a scenario player synthesizes the same `agent-event-*` stream
//! the Rust loop emits (see src-tauri/src/agent/agent_loop.rs), so the real
//! panel components render without Tauri.
//!
//! The real panel never touches this module — only `/preview/agent` imports
//! it, and the preview routes 404 outside dev.

import type { UnlistenFn } from '@tauri-apps/api/event';
import type { AgentBackend } from './agent-backend';
import type {
	AgentEvent,
	AgentSendOpts,
	ApprovalRequest,
	ApprovalWarning,
	ContentBlock,
	DiffPreview,
	DiffPreviewLine,
	InstanceModels,
	MessageMetadata,
	ModelMeta,
	PathMessage,
	ProviderInstance,
	StoredMessage,
	ThreadSnapshot,
	ThreadState,
	ThreadSummary,
	ToolCallStatus,
	ToolCallView,
	ToolConfig,
	ToolResult,
	ToolSettingsEntry,
	Usage
} from './agent';

/** Identity the preview page binds its AgentPanel to. */
export const MOCK_IDENTITY = 'mock@preview:22';

export interface MockScenarioMeta {
	name: string;
	label: string;
	description: string;
}

// ---------------------------------------------------------------------------
// Fake providers & models (drive the composer's model picker)
// ---------------------------------------------------------------------------

const PRESET_MODELS: Record<string, ModelMeta[]> = {
	openrouter: [
		{
			id: 'anthropic/claude-sonnet-4.5',
			displayName: 'Claude Sonnet 4.5',
			contextLength: 200_000,
			maxOutputTokens: 64_000,
			supportsThinking: true,
			thinkingMandatory: false,
			thinkingEfforts: ['low', 'medium', 'high'],
			defaultEffort: 'medium',
			supportsTools: true,
			pricing: [3, 15]
		},
		{
			id: 'openai/gpt-5.2',
			displayName: 'GPT-5.2',
			contextLength: 400_000,
			maxOutputTokens: 128_000,
			supportsThinking: true,
			thinkingMandatory: true,
			thinkingEfforts: ['low', 'medium', 'high', 'xhigh'],
			defaultEffort: 'medium',
			supportsTools: true,
			pricing: [1.75, 14]
		},
		{
			id: 'google/gemini-3-pro-preview',
			displayName: 'Gemini 3 Pro',
			contextLength: 1_048_576,
			maxOutputTokens: 65_536,
			supportsThinking: true,
			thinkingMandatory: false,
			thinkingEfforts: ['low', 'high'],
			defaultEffort: 'low',
			supportsTools: true,
			pricing: [2, 12]
		}
	],
	deepseek: [
		{
			id: 'deepseek-v4-pro',
			displayName: 'DeepSeek V4 Pro',
			contextLength: 131_072,
			maxOutputTokens: 32_768,
			supportsThinking: true,
			thinkingMandatory: false,
			thinkingEfforts: ['medium', 'high'],
			defaultEffort: 'medium',
			supportsTools: true,
			pricing: [0.55, 2.19]
		},
		{
			id: 'deepseek-chat',
			displayName: 'DeepSeek Chat',
			contextLength: 65_536,
			maxOutputTokens: 8_192,
			supportsThinking: false,
			thinkingMandatory: false,
			thinkingEfforts: [],
			supportsTools: true,
			pricing: [0.27, 1.1]
		}
	],
	kimi: [
		{
			id: 'kimi-k2-thinking',
			displayName: 'Kimi K2 Thinking',
			contextLength: 262_144,
			maxOutputTokens: 32_768,
			supportsThinking: true,
			thinkingMandatory: true,
			thinkingEfforts: ['high'],
			defaultEffort: 'high',
			supportsTools: true,
			pricing: [0.6, 2.5]
		}
	]
};

// ---------------------------------------------------------------------------
// Scenario context: the helpers scenario scripts use to emit events
// ---------------------------------------------------------------------------

const CANCELLED = Symbol('cancelled');

interface MockRun {
	cancelled: boolean;
	/** Rejects all pending waitApproval calls when the run is cancelled. */
	rejectApprovals: Set<() => void>;
}

interface MockThread {
	summary: ThreadSummary;
	/** All messages ever appended, by id. */
	messages: Map<string, StoredMessage>;
	/** parentId ('' = root) -> sibling message ids in branch order. */
	children: Map<string, string[]>;
	/** parentId -> active sibling index (default: last). */
	activeChild: Map<string, number>;
	queued: string | null;
	running: boolean;
	run: MockRun | null;
	scenario: string;
	seq: number;
}

interface Ctx {
	identity: string;
	threadId: string;
	/** Sleeps; rejects with the cancel sentinel when the run was cancelled. */
	sleep(ms: number): Promise<void>;
	/** Appends an empty assistant message to the store; returns its id. */
	beginAssistant(): string;
	streamThinking(messageId: string, text: string): Promise<void>;
	streamText(messageId: string, text: string): Promise<void>;
	/** Announces a tool call as `streaming` and drips its arguments JSON. */
	streamToolCall(messageId: string, name: string, args: unknown): Promise<ToolCallView>;
	/** Emits a tool_call state migration and persists it on the message. */
	setToolStatus(
		call: ToolCallView,
		status: ToolCallStatus,
		result?: ToolResult,
		warnings?: ApprovalWarning[]
	): void;
	/** pending_approval + approval_needed, then resolves with the decision. */
	requestApproval(
		call: ToolCallView,
		title: string,
		warnings: ApprovalWarning[],
		payload?: unknown
	): Promise<boolean>;
	/** Emits one terminal_output chunk (base64-encoded). */
	emitTerminal(toolCallId: string, text: string): void;
	terminalStopped(toolCallId: string): boolean;
	/** Emits a usage event with a growing prompt size. */
	addUsage(completionTokens: number, cachedTokens?: number): void;
	/**
	 * Persists final metadata and emits message_done. Tool-call rounds should
	 * pass `emitId`: the real backend emits the persisted message id there,
	 * which never matches the streaming placeholder id the panel knows (the
	 * frontend falls back to the streaming message).
	 */
	finishMessage(messageId: string, toolCallCount: number, startedAt: number, emitId?: string): void;
	emitError(message: string): void;
}

interface MockScenario {
	label: string;
	description: string;
	seed?: (thread: MockThread) => void;
	run: (ctx: Ctx) => Promise<void>;
	/** Reply text for the queued-message follow-up round. */
	queuedFollowUp?: (text: string) => string;
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

function b64(text: string): string {
	const bytes = new TextEncoder().encode(text);
	let bin = '';
	for (const b of bytes) bin += String.fromCharCode(b);
	return btoa(bin);
}

function stripAnsi(s: string): string {
	// eslint-disable-next-line no-control-regex
	return s.replace(/\x1b\[[0-9;]*m/g, '');
}

/** Word-sized chunks (whitespace kept) for natural-looking streaming. */
function wordChunks(text: string): string[] {
	return text.match(/\S+\s*|\s+/g) ?? [];
}

function jsonChunks(json: string): string[] {
	const out: string[] = [];
	let i = 0;
	while (i < json.length) {
		const n = 8 + Math.floor(Math.random() * 8);
		out.push(json.slice(i, i + n));
		i += n;
	}
	return out;
}

function safeParse(json: string): unknown {
	try {
		return JSON.parse(json);
	} catch {
		return json;
	}
}

/** Simple-escape lookup for parsePartialJson. */
const JSON_ESCAPES: Record<string, string> = {
	'"': '"',
	'\\': '\\',
	'/': '/',
	b: '\b',
	f: '\f',
	n: '\n',
	r: '\r',
	t: '\t'
};

/**
 * Best-effort parse of a *prefix* of a JSON document — the payload the real
 * backend puts in `tool_call_args_patched`. Never throws: a string cut
 * mid-way keeps its completed prefix (a dangling `\` or `\u12` escape tail
 * is dropped); an unclosed object/array yields the entries completed so far;
 * a number/true/false/null literal only counts once a delimiter proves it
 * complete (half literals are dropped — this mock never needs one). Returns
 * undefined until at least one value has arrived.
 */
function parsePartialJson(src: string): unknown {
	let i = 0;
	const ws = (): void => {
		while (i < src.length && ' \t\n\r'.includes(src.charAt(i))) i++;
	};
	const str = (): string => {
		let out = '';
		i++; // opening quote
		while (i < src.length) {
			const c = src.charAt(i++);
			if (c === '"') return out;
			if (c !== '\\') {
				out += c;
				continue;
			}
			if (i >= src.length) break; // dangling backslash: drop it
			const e = src.charAt(i);
			if (e !== 'u') {
				const rep: string | undefined = JSON_ESCAPES[e];
				if (rep === undefined) break; // unknown escape tail: drop it
				out += rep;
				i++;
				continue;
			}
			const hex = src.slice(i + 1, i + 5);
			if (!/^[0-9a-fA-F]{4}$/.test(hex)) break; // dangling \u tail: drop it
			out += String.fromCharCode(parseInt(hex, 16));
			i += 5;
		}
		return out;
	};
	// A literal only counts when a delimiter follows — at end-of-prefix it
	// may still be growing, so drop it.
	const literal = (): unknown => {
		const m = /^-?(?:\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|true|false|null)/.exec(src.slice(i));
		if (!m || i + m[0].length >= src.length) {
			i = src.length;
			return undefined;
		}
		i += m[0].length;
		return JSON.parse(m[0]) as unknown;
	};
	const value = (): unknown => {
		ws();
		const c = src.charAt(i); // '' past the end
		if (c === '"') return str();
		if (c === '{') return obj();
		if (c === '[') return arr();
		if (c === '') return undefined;
		return literal();
	};
	const arr = (): unknown[] => {
		const out: unknown[] = [];
		i++; // '['
		while (i < src.length) {
			ws();
			if (src.charAt(i) === ']') {
				i++;
				break;
			}
			const v = value();
			if (v !== undefined) out.push(v);
			ws();
			if (src.charAt(i) !== ',') break;
			i++;
		}
		return out;
	};
	const obj = (): Record<string, unknown> => {
		const out: Record<string, unknown> = {};
		i++; // '{'
		while (i < src.length) {
			ws();
			if (src.charAt(i) === '}') {
				i++;
				break;
			}
			if (src.charAt(i) !== '"') break;
			const key = str();
			ws();
			if (src.charAt(i) !== ':') break; // key arrived, colon didn't
			i++;
			const v = value();
			if (v === undefined) break; // value still streaming: drop the pair
			out[key] = v;
			ws();
			if (src.charAt(i) !== ',') break;
			i++;
		}
		return out;
	};
	return value();
}

/** The (possibly partial) new_text of an edit_file args value or prefix. */
function partialEditNewText(args: unknown): string | undefined {
	const t = (args as { edits?: { new_text?: unknown }[] } | null)?.edits?.[0]?.new_text;
	return typeof t === 'string' ? t : undefined;
}

/**
 * Truncated variant of a diff preview for the streaming simulation: ctx/del
 * lines come from the old file and are always known, so only the add lines
 * grow — keep the first half, with the last visible add cut mid-text.
 */
function halfDiffPreview(preview: DiffPreview): DiffPreview {
	return {
		...preview,
		hunks: preview.hunks.map((hunk) => {
			const keep = Math.ceil(hunk.lines.filter((l) => l.kind === 'add').length / 2);
			let adds = 0;
			const lines: DiffPreviewLine[] = [];
			for (const line of hunk.lines) {
				if (line.kind === 'add') {
					adds++;
					if (adds > keep) continue;
					if (adds === keep) {
						lines.push({ ...line, text: line.text.slice(0, Math.ceil(line.text.length * 0.6)) });
						continue;
					}
				}
				lines.push(line);
			}
			return { ...hunk, lines };
		})
	};
}

// ---------------------------------------------------------------------------
// Mock content fixtures
// ---------------------------------------------------------------------------

const SSHD_CONFIG = `     1\t# $OpenBSD: sshd_config,v 1.103 2018/04/09 20:41:22 tj Exp $
     2\t
     3\tPort 22
     4\tAddressFamily any
     5\tListenAddress 0.0.0.0
     6\tListenAddress ::
     7\t
     8\tPermitRootLogin prohibit-password
     9\tPasswordAuthentication no
    10\tPubkeyAuthentication yes
    11\t
    12\tSubsystem sftp /usr/lib/openssh/sftp-server`;

const NGINX_EDIT_ARGS = {
	path: '/etc/nginx/sites-available/reach.conf',
	edits: [
		{
			old_text: '    listen 80;\n    server_name reach.example.com;',
			new_text:
				'    listen 443 ssl;\n' +
				'    server_name reach.example.com;\n' +
				'    ssl_certificate /etc/letsencrypt/live/reach.example.com/fullchain.pem;\n' +
				'    ssl_certificate_key /etc/letsencrypt/live/reach.example.com/privkey.pem;'
		}
	]
};

const NGINX_DIFF = `--- a/etc/nginx/sites-available/reach.conf
+++ b/etc/nginx/sites-available/reach.conf
@@ -7,8 +7,10 @@
     root /var/www/reach;
     index index.html;

-    listen 80;
+    listen 443 ssl;
     server_name reach.example.com;
+    ssl_certificate /etc/letsencrypt/live/reach.example.com/fullchain.pem;
+    ssl_certificate_key /etc/letsencrypt/live/reach.example.com/privkey.pem;

     location / {
         try_files $uri $uri/ =404;`;

/**
 * Structured form of NGINX_DIFF — the shape approval payloads, result
 * uiPayloads, and streamed `tool_call_preview` events now carry. Line
 * numbers are 1-based: del/ctx lines carry oldNo, add/ctx lines newNo.
 */
const NGINX_DIFF_PREVIEW: DiffPreview = {
	path: NGINX_EDIT_ARGS.path,
	isNewFile: false,
	hunks: [
		{
			oldStart: 7,
			newStart: 7,
			lines: [
				{ kind: 'ctx', text: '    root /var/www/reach;', oldNo: 7, newNo: 7 },
				{ kind: 'ctx', text: '    index index.html;', oldNo: 8, newNo: 8 },
				{ kind: 'ctx', text: '', oldNo: 9, newNo: 9 },
				{ kind: 'del', text: '    listen 80;', oldNo: 10 },
				{ kind: 'add', text: '    listen 443 ssl;', newNo: 10 },
				{ kind: 'ctx', text: '    server_name reach.example.com;', oldNo: 11, newNo: 11 },
				{
					kind: 'add',
					text: '    ssl_certificate /etc/letsencrypt/live/reach.example.com/fullchain.pem;',
					newNo: 12
				},
				{
					kind: 'add',
					text: '    ssl_certificate_key /etc/letsencrypt/live/reach.example.com/privkey.pem;',
					newNo: 13
				},
				{ kind: 'ctx', text: '', oldNo: 12, newNo: 14 },
				{ kind: 'ctx', text: '    location / {', oldNo: 13, newNo: 15 },
				{ kind: 'ctx', text: '        try_files $uri $uri/ =404;', oldNo: 14, newNo: 16 }
			]
		}
	]
};

/** The reach.conf site before the HTTPS edit, as read_file would return it. */
const NGINX_SITE_CONFIG = `     1\tserver {
     2\t    listen 80;
     3\t    server_name reach.example.com;
     4\t
     5\t    root /var/www/reach;
     6\t    index index.html;
     7\t
     8\t    location / {
     9\t        try_files $uri $uri/ =404;
    10\t    }
    11\t}`;

const NGINX_RELOAD_COMMAND = 'nginx -t && systemctl reload nginx';

const NGINX_RELOAD_CHUNKS: string[] = [
	'\x1b[1;36m$ nginx -t\x1b[0m\r\n',
	'nginx: the configuration file /etc/nginx/nginx.conf syntax is ok\r\n',
	'nginx: configuration file /etc/nginx/nginx.conf test is successful\r\n',
	'\x1b[1;36m$ systemctl reload nginx\x1b[0m\r\n',
	'\x1b[33m●\x1b[0m waiting for nginx to reload...\r\n',
	'\x1b[32m✓\x1b[0m nginx reloaded, 0 errors\r\n'
];

const TERMINAL_COMMAND = 'cd /var/www/reach && git pull && systemctl reload nginx';

const TERMINAL_CHUNKS: string[] = [
	'\x1b[1;36m$ git pull\x1b[0m\r\n',
	'remote: Enumerating objects: 14, done.\r\n',
	'remote: Counting objects: 100% (14/14), done.\r\n',
	'Updating 3fa2c1e..9b7d21f\r\n',
	'\x1b[32mFast-forward\x1b[0m\r\n',
	' src/lib/agent.ts    | 42 \x1b[32m++++++++++++++++++\x1b[0m\x1b[31m----\x1b[0m\r\n',
	' src/routes/+page.svelte |  9 \x1b[32m+++++\x1b[0m\x1b[31m--\x1b[0m\r\n',
	' 2 files changed, 23 insertions(+), 6 deletions(-)\r\n',
	'\x1b[1;36m$ systemctl reload nginx\x1b[0m\r\n',
	'\x1b[33m●\x1b[0m waiting for nginx to reload...\r\n',
	'\x1b[32m✓\x1b[0m nginx reloaded, 0 errors\r\n'
];

const QUEUED_ESSAY = `The agent loop is a sequence of **rounds**, not a single request.

## One round

Each round streams one assistant message. While it streams, every delta is forwarded to the panel as an event — text, thinking, and tool-call arguments alike. That is why cards render before their JSON is even complete.

## Tool calls

When the model requests tools, the round pauses after the message is persisted. Each call that needs approval waits on its own Accept click; the rest run immediately. Results feed back into the next round as ordinary messages.

## The queue boundary

If you send a message mid-run, it lands in a one-slot queue. At the end of the *current round* — not the whole run — the queued text is injected as a new user message and the loop continues. You should see exactly that in a few seconds.

## Cancelling

The stop button drops the cancellation token observed by the stream, pending approvals, and running tools at once. Whatever partial text exists stays in place.

Keep watching — this message is deliberately long so there is time to queue a follow-up. Type something below and hit send while this is still streaming: the queued bar appears above the composer, and your message gets injected at the round boundary without losing a single delta of this reply.`;

// ---------------------------------------------------------------------------
// Mock backend
// ---------------------------------------------------------------------------

class MockAgentBackend implements AgentBackend {
	private threads = new Map<string, MockThread>();
	private listeners = new Map<string, Set<(e: AgentEvent) => void>>();
	private pendingApprovals = new Map<
		string,
		{ request: ApprovalRequest; resolve: (ok: boolean) => void }
	>();
	private stoppedTerminals = new Set<string>();
	private apiKeys = new Map<string, string>();
	private instances: ProviderInstance[] = [
		{ id: 'mock-or', preset: 'openrouter', name: 'OpenRouter' },
		{ id: 'mock-ds', preset: 'deepseek', name: 'DeepSeek' }
	];
	private toolConfigs = new Map<string, ToolConfig>();
	private titleModel: string | null = 'mock-ds/deepseek-chat';
	private lastIdentity = MOCK_IDENTITY;
	private activeScenario = 'text-thinking';
	private nextMessageId = 1;
	private nextToolCallId = 1;
	private nextThreadId = 1;

	// ── event hub ────────────────────────────────────────────────────────────

	onEvent(identity: string, cb: (e: AgentEvent) => void): UnlistenFn {
		this.lastIdentity = identity;
		let set = this.listeners.get(identity);
		if (!set) {
			set = new Set();
			this.listeners.set(identity, set);
		}
		set.add(cb);
		return () => {
			set.delete(cb);
		};
	}

	private emit(identity: string, e: AgentEvent): void {
		for (const cb of this.listeners.get(identity) ?? []) cb(e);
	}

	// ── thread store ─────────────────────────────────────────────────────────

	private newThread(identity: string): MockThread {
		const now = Date.now();
		const thread: MockThread = {
			summary: {
				id: `mock-thread-${this.nextThreadId++}`,
				identity,
				title: '',
				archived: false,
				createdAt: now,
				updatedAt: now
			},
			messages: new Map(),
			children: new Map(),
			activeChild: new Map(),
			queued: null,
			running: false,
			run: null,
			scenario: this.activeScenario,
			seq: 1
		};
		this.threads.set(thread.summary.id, thread);
		return thread;
	}

	private requireThread(threadId: string): MockThread {
		const thread = this.threads.get(threadId);
		if (!thread) throw new Error(`Thread not found: ${threadId}`);
		return thread;
	}

	private link(thread: MockThread, msg: StoredMessage): void {
		const key = msg.parentId ?? '';
		let kids = thread.children.get(key);
		if (!kids) {
			kids = [];
			thread.children.set(key, kids);
		}
		msg.branchIndex = kids.length;
		kids.push(msg.id);
		// The newest sibling becomes the active branch (fork semantics).
		thread.activeChild.set(key, kids.length - 1);
		thread.messages.set(msg.id, msg);
		thread.summary.updatedAt = msg.createdAt;
	}

	private appendUserMessage(thread: MockThread, text: string): StoredMessage {
		const msg: StoredMessage = {
			id: `m${this.nextMessageId++}`,
			threadId: thread.summary.id,
			parentId: this.leafId(thread),
			branchIndex: 0,
			seq: thread.seq++,
			role: 'user',
			content: [{ type: 'text', text }],
			createdAt: Date.now()
		};
		this.link(thread, msg);
		return msg;
	}

	private leafId(thread: MockThread): string | undefined {
		let parent = '';
		let last: string | undefined;
		for (;;) {
			const kids = thread.children.get(parent);
			if (!kids?.length) return last;
			const idx = Math.min(thread.activeChild.get(parent) ?? kids.length - 1, kids.length - 1);
			last = kids[idx];
			parent = last;
		}
	}

	private computePath(thread: MockThread): PathMessage[] {
		const path: PathMessage[] = [];
		let parent = '';
		for (;;) {
			const kids = thread.children.get(parent);
			if (!kids?.length) return path;
			const idx = Math.min(thread.activeChild.get(parent) ?? kids.length - 1, kids.length - 1);
			const id = kids[idx];
			const msg = thread.messages.get(id);
			if (!msg) return path;
			path.push({ ...msg, branch: { index: idx + 1, count: kids.length } });
			parent = id;
		}
	}

	private snapshot(thread: MockThread): ThreadSnapshot {
		return { thread: { ...thread.summary }, messages: this.computePath(thread) };
	}

	/**
	 * Snapshot with the same enrichments as the Rust side's
	 * `AgentState::enrich_snapshot`: reconcile stale blocks of dead runs to
	 * `cancelled`, then overlay live pending approvals (status + warnings)
	 * onto the returned copies.
	 */
	private snapshotView(thread: MockThread): ThreadSnapshot {
		if (!thread.running) {
			for (const msg of thread.messages.values()) {
				for (const block of msg.content) {
					if (
						block.type === 'tool_call' &&
						(block.status === 'streaming' || block.status === 'pending_approval')
					) {
						block.status = 'cancelled';
					}
				}
			}
		}
		const snapshot = this.snapshot(thread);
		if (this.pendingApprovals.size > 0) {
			for (const msg of snapshot.messages) {
				msg.content = msg.content.map((block) => {
					if (block.type !== 'tool_call') return block;
					const pending = this.pendingApprovals.get(block.id);
					if (!pending) return block;
					return {
						...block,
						status: 'pending_approval' as const,
						warnings:
							pending.request.warnings.length > 0
								? [...pending.request.warnings]
								: block.warnings
					};
				});
			}
		}
		return snapshot;
	}

	// ── messaging ────────────────────────────────────────────────────────────

	async sendMessage(
		identity: string,
		threadId: string,
		text: string,
		opts: AgentSendOpts
	): Promise<'started' | 'queued' | 'queued_full'> {
		this.lastIdentity = identity;
		const thread = this.requireThread(threadId);
		if (thread.running) {
			// One-slot queue (design 01 §3.5); no event until the round boundary.
			if (thread.queued != null) return 'queued_full';
			thread.queued = text;
			return 'queued';
		}
		thread.running = true;
		thread.run = { cancelled: false, rejectApprovals: new Set() };
		// Model snapshot on the thread (design 05 §5).
		thread.summary.model = { model: opts.model, thinking: opts.thinking, effort: opts.effort };
		const msg = this.appendUserMessage(thread, text);
		this.emit(identity, { kind: 'user_message', threadId, message: msg });
		void this.executeRun(identity, thread, thread.run, opts);
		return 'started';
	}

	async cancel(threadId: string): Promise<void> {
		const run = this.threads.get(threadId)?.run;
		if (!run) return; // no active run: no-op, no event (mirrors agent_cancel)
		run.cancelled = true;
		for (const reject of run.rejectApprovals) reject();
	}

	async dequeue(threadId: string): Promise<boolean> {
		const thread = this.threads.get(threadId);
		if (!thread || thread.queued == null) return false;
		thread.queued = null;
		return true;
	}

	async sendNow(
		identity: string,
		threadId: string,
		text: string,
		opts: AgentSendOpts
	): Promise<void> {
		this.lastIdentity = identity;
		const thread = this.requireThread(threadId);
		const wasQueued = thread.queued != null;
		thread.queued = null;
		if (thread.run) {
			thread.run.cancelled = true;
			for (const reject of thread.run.rejectApprovals) reject();
			// Wait for the dying run to fully exit (mirrors agent_send_now).
			const deadline = Date.now() + 10_000;
			while (thread.running && Date.now() < deadline) {
				await new Promise((resolve) => setTimeout(resolve, 10));
			}
			if (thread.running) {
				if (wasQueued) thread.queued = text; // rollback
				throw new Error(
					'The current run did not stop in time. Your message was kept in the queue.'
				);
			}
		}
		// Fresh run (same as sendMessage's 'started' branch).
		thread.running = true;
		thread.run = { cancelled: false, rejectApprovals: new Set() };
		thread.summary.model = { model: opts.model, thinking: opts.thinking, effort: opts.effort };
		const msg = this.appendUserMessage(thread, text);
		this.emit(identity, { kind: 'user_message', threadId, message: msg });
		void this.executeRun(identity, thread, thread.run, opts);
	}

	async approve(toolCallId: string, approved: boolean): Promise<void> {
		const pending = this.pendingApprovals.get(toolCallId);
		this.pendingApprovals.delete(toolCallId);
		if (!pending) throw new Error('No pending approval for this tool call');
		pending.resolve(approved);
	}

	async terminalResize(): Promise<void> {
		// No PTY behind the mock; resize notifications are dropped.
	}

	async terminalStop(toolCallId: string): Promise<void> {
		this.stoppedTerminals.add(toolCallId);
	}

	// ── run engine ───────────────────────────────────────────────────────────

	private async executeRun(
		identity: string,
		thread: MockThread,
		run: MockRun,
		opts: AgentSendOpts | null
	): Promise<void> {
		const threadId = thread.summary.id;
		const scenario = this.scenarios[thread.scenario] ?? this.scenarios['text-thinking'];
		const ctx = this.makeCtx(identity, thread, run, opts);
		try {
			await scenario.run(ctx);
			// Queue injection at the round boundary (design 01 §3.5).
			while (thread.queued != null) {
				const text = thread.queued;
				thread.queued = null;
				const msg = this.appendUserMessage(thread, text);
				this.emit(identity, { kind: 'user_message', threadId, message: msg });
				const startedAt = Date.now();
				const follow = ctx.beginAssistant();
				await ctx.sleep(350);
				const reply = scenario.queuedFollowUp
					? scenario.queuedFollowUp(text)
					: `Following up on "${text}" — a queued message gets its own round, streamed just like the first.`;
				await ctx.streamText(follow, reply);
				ctx.addUsage(180);
				ctx.finishMessage(follow, 0, startedAt);
			}
			this.maybeTitle(identity, thread);
			// Normal exit: unregister first, then emit the terminal event
			// (mirrors try_finish_run in the Rust agent loop), so the
			// composer settles back to the send button.
			thread.running = false;
			thread.run = null;
			this.emit(identity, { kind: 'run_end', threadId });
		} catch (e) {
			// Abnormal exit: the queued message dies with the run (mirrors the
			// real backend's cleanup; no ghost injection into the next run).
			thread.queued = null;
			if (e === CANCELLED) {
				this.emit(identity, { kind: 'cancelled', threadId });
			} else {
				this.emit(identity, {
					kind: 'error',
					threadId,
					message: e instanceof Error ? e.message : String(e)
				});
			}
		} finally {
			thread.running = false;
			thread.run = null;
			thread.summary.updatedAt = Date.now();
		}
	}

	private maybeTitle(identity: string, thread: MockThread): void {
		if (thread.summary.title) return;
		const first = thread.children.get('')?.[0];
		const text = first
			?.split('')
			.reduce(() => thread.messages.get(first), undefined)
			?.content.find((b) => b.type === 'text');
		const title =
			text && text.type === 'text' ? text.text.trim().split('\n')[0].slice(0, 30) : '';
		if (!title) return;
		thread.summary.title = title;
		this.emit(identity, { kind: 'title_updated', threadId: thread.summary.id, title });
	}

	private makeCtx(
		identity: string,
		thread: MockThread,
		run: MockRun,
		opts: AgentSendOpts | null
	): Ctx {
		const threadId = thread.summary.id;
		const sleep = (ms: number) =>
			new Promise<void>((resolve, reject) => {
				setTimeout(() => (run.cancelled ? reject(CANCELLED) : resolve()), ms);
			});
		const streamInto = async (
			messageId: string,
			kind: 'text_delta' | 'thinking_delta',
			text: string
		) => {
			const msg = thread.messages.get(messageId);
			const blockType = kind === 'text_delta' ? 'text' : 'thinking';
			let block = msg?.content.find((b) => b.type === blockType) as
				| { type: 'text' | 'thinking'; text: string }
				| undefined;
			if (msg && !block) {
				block = { type: blockType, text: '' };
				msg.content.push(block as ContentBlock);
			}
			for (const chunk of wordChunks(text)) {
				if (run.cancelled) throw CANCELLED;
				this.emit(identity, { kind, threadId, messageId, delta: chunk });
				if (block) block.text += chunk;
				await sleep(34 + Math.random() * 46);
			}
		};
		return {
			identity,
			threadId,
			sleep,
			beginAssistant: () => {
				const msg: StoredMessage = {
					id: `m${this.nextMessageId++}`,
					threadId,
					parentId: this.leafId(thread),
					branchIndex: 0,
					seq: thread.seq++,
					role: 'assistant',
					content: [],
					createdAt: Date.now()
				};
				this.link(thread, msg);
				return msg.id;
			},
			streamThinking: (messageId, text) => streamInto(messageId, 'thinking_delta', text),
			streamText: (messageId, text) => streamInto(messageId, 'text_delta', text),
			streamToolCall: async (messageId, name, args) => {
				const call: ToolCallView = {
					id: `tc-${this.nextToolCallId++}`,
					messageId,
					name,
					argsJson: '',
					status: 'streaming'
				};
				this.emit(identity, { kind: 'tool_call', threadId, toolCall: { ...call } });
				const msg = thread.messages.get(messageId);
				msg?.content.push({
					type: 'tool_call',
					id: call.id,
					name,
					args: {},
					status: 'streaming'
				});
				const full = JSON.stringify(args);
				// edit_file preview simulation: while new_text drips in, the real
				// backend publishes a growing structured diff — half the add lines
				// first, the full preview once new_text has closed.
				const finalNewText = name === 'edit_file' ? partialEditNewText(args) : undefined;
				const newTextLen = finalNewText?.length ?? 0;
				let previewStage = 0; // 0 = none sent, 1 = half sent, 2 = full sent
				let lastPatched = '';
				for (const chunk of jsonChunks(full)) {
					if (run.cancelled) throw CANCELLED;
					call.argsJson += chunk;
					this.emit(identity, {
						kind: 'tool_call_args_delta',
						threadId,
						toolCallId: call.id,
						argsJsonDelta: chunk
					});
					const patched = parsePartialJson(call.argsJson);
					if (patched !== undefined) {
						const patchedJson = JSON.stringify(patched);
						if (patchedJson !== lastPatched) {
							lastPatched = patchedJson;
							this.emit(identity, {
								kind: 'tool_call_args_patched',
								threadId,
								toolCallId: call.id,
								argsJson: patchedJson
							});
						}
					}
					if (newTextLen > 0 && previewStage < 2 && patched !== undefined) {
						const partialLen = partialEditNewText(patched)?.length ?? 0;
						if (previewStage === 0 && partialLen >= newTextLen / 2) {
							previewStage = 1;
							this.emit(identity, {
								kind: 'tool_call_preview',
								threadId,
								toolCallId: call.id,
								preview: halfDiffPreview(NGINX_DIFF_PREVIEW)
							});
						}
						if (previewStage === 1 && partialLen >= newTextLen) {
							previewStage = 2;
							this.emit(identity, {
								kind: 'tool_call_preview',
								threadId,
								toolCallId: call.id,
								preview: NGINX_DIFF_PREVIEW
							});
						}
					}
					await sleep(40 + Math.random() * 40);
				}
				return call;
			},
			setToolStatus: (call, status, result, warnings) => {
				call.status = status;
				call.result = result;
				call.warnings = warnings;
				this.emit(identity, { kind: 'tool_call', threadId, toolCall: { ...call } });
				const msg = thread.messages.get(call.messageId);
				const block = msg?.content.find((b) => b.type === 'tool_call' && b.id === call.id);
				if (block && block.type === 'tool_call') {
					block.status = status;
					block.result = result;
					block.args = safeParse(call.argsJson);
				}
			},
			requestApproval: (call, title, warnings, payload) => {
				return new Promise<boolean>((resolve, reject) => {
					const request: ApprovalRequest = {
						toolCallId: call.id,
						tool: call.name,
						title,
						payload,
						warnings
					};
					const rejectPending = () => {
						run.rejectApprovals.delete(rejectPending);
						this.pendingApprovals.delete(call.id);
						reject(CANCELLED);
					};
					run.rejectApprovals.add(rejectPending);
					this.pendingApprovals.set(call.id, {
						request,
						resolve: (ok) => {
							run.rejectApprovals.delete(rejectPending);
							resolve(ok);
						}
					});
					const pending: ToolCallView = {
						...call,
						status: 'pending_approval',
						warnings
					};
					this.emit(identity, { kind: 'tool_call', threadId, toolCall: pending });
					this.emit(identity, { kind: 'approval_needed', threadId, approval: request });
				});
			},
			emitTerminal: (toolCallId, text) => {
				this.emit(identity, { kind: 'terminal_output', toolCallId, dataB64: b64(text) });
			},
			terminalStopped: (toolCallId) => this.stoppedTerminals.has(toolCallId),
			addUsage: (completionTokens, cachedTokens) => {
				const prev = thread.summary.lastUsage;
				const usage: Usage = {
					promptTokens: (prev?.promptTokens ?? 1600) + (prev?.completionTokens ?? 0) + 137,
					completionTokens,
					...(cachedTokens != null ? { cachedTokens } : {})
				};
				thread.summary.lastUsage = usage;
				this.emit(identity, { kind: 'usage', threadId, usage });
			},
			finishMessage: (messageId, toolCallCount, startedAt, emitId) => {
				const msg = thread.messages.get(messageId);
				let metadata: MessageMetadata | undefined;
				if (msg) {
					const [instanceId, ...rest] = (opts?.model ??
						thread.summary.model?.model ??
						''
					).split('/');
					const instance = this.instances.find((i) => i.id === instanceId);
					msg.usage = thread.summary.lastUsage;
					msg.metadata = {
						providerInstance: instance?.name ?? instanceId ?? undefined,
						model: rest.join('/') || undefined,
						usage: thread.summary.lastUsage,
						durationMs: Date.now() - startedAt,
						toolCallCount
					};
					metadata = msg.metadata;
				}
				this.emit(identity, {
					kind: 'message_done',
					threadId,
					messageId: emitId ?? messageId,
					metadata
				});
			},
			emitError: (message) => {
				this.emit(identity, { kind: 'error', threadId, message });
			}
		};
	}
		// ── threads (AgentBackend surface) ───────────────────────────────────────

		async threadsList(identity: string): Promise<ThreadSummary[]> {
			return [...this.threads.values()]
				.map((t) => ({ ...t.summary }))
				.filter((t) => t.identity === identity && !t.archived)
				.sort((a, b) => b.updatedAt - a.updatedAt);
		}

		async threadsListAll(): Promise<ThreadSummary[]> {
			return [...this.threads.values()]
				.map((t) => ({ ...t.summary }))
				.sort((a, b) => b.updatedAt - a.updatedAt);
		}

		async threadCreate(identity: string): Promise<ThreadSummary> {
			const thread = this.newThread(identity);
			this.scenarios[thread.scenario]?.seed?.(thread);
			return { ...thread.summary };
		}

		async threadRename(threadId: string, title: string): Promise<void> {
			this.requireThread(threadId).summary.title = title;
		}

		async threadArchive(threadId: string, archived: boolean): Promise<void> {
			this.requireThread(threadId).summary.archived = archived;
		}

		async threadDelete(threadId: string): Promise<void> {
			const thread = this.threads.get(threadId);
			if (thread?.run) {
				thread.run.cancelled = true;
				for (const reject of thread.run.rejectApprovals) reject();
			}
			this.threads.delete(threadId);
		}

		async threadMessages(threadId: string): Promise<ThreadSnapshot> {
			return this.snapshotView(this.requireThread(threadId));
		}

		async threadState(threadId: string): Promise<ThreadState> {
			const thread = this.requireThread(threadId);
			return { ...this.snapshotView(thread), running: thread.running };
		}

		async threadEditMessage(
			identity: string,
			threadId: string,
			messageId: string,
			newContent: string,
			opts: AgentSendOpts
		): Promise<string> {
			const thread = this.requireThread(threadId);
			const original = thread.messages.get(messageId);
			if (!original || original.role !== 'user') throw new Error('Only user messages can be edited');
			if (thread.running) throw new Error('A run is active; cancel it before editing history');
			const msg: StoredMessage = {
				id: `m${this.nextMessageId++}`,
				threadId,
				parentId: original.parentId,
				branchIndex: 0,
				seq: thread.seq++,
				role: 'user',
				content: [{ type: 'text', text: newContent }],
				createdAt: Date.now()
			};
			this.link(thread, msg);
			this.emit(identity, { kind: 'user_message', threadId, message: msg });
			thread.running = true;
			thread.run = { cancelled: false, rejectApprovals: new Set() };
			void this.executeRun(identity, thread, thread.run, opts);
			return msg.id;
		}

		async threadSetActiveBranch(
			threadId: string,
			atMessageId: string,
			direction: 'prev' | 'next'
		): Promise<ThreadSnapshot> {
			const thread = this.requireThread(threadId);
			const msg = thread.messages.get(atMessageId);
			if (!msg) return this.snapshot(thread);
			const key = msg.parentId ?? '';
			const kids = thread.children.get(key) ?? [];
			const idx = kids.indexOf(atMessageId);
			if (idx < 0 || kids.length < 2) return this.snapshot(thread);
			const next = direction === 'prev' ? Math.max(0, idx - 1) : Math.min(kids.length - 1, idx + 1);
			thread.activeChild.set(key, next);
			// Descend to a leaf, preferring the newest fork at each level.
			let parent = kids[next];
			for (;;) {
				const sub = thread.children.get(parent);
				if (!sub?.length) break;
				const subIdx = Math.min(thread.activeChild.get(parent) ?? sub.length - 1, sub.length - 1);
				thread.activeChild.set(parent, subIdx);
				parent = sub[subIdx];
			}
			return this.snapshot(thread);
		}

		// ── providers / models / settings ────────────────────────────────────────

		async providersList(): Promise<ProviderInstance[]> {
			return this.instances.map((i) => ({ ...i }));
		}

		async providerPresets(): Promise<[string, string, string][]> {
			return [
				['openrouter', 'OpenRouter', 'https://openrouter.ai/api/v1'],
				['deepseek', 'DeepSeek', 'https://api.deepseek.com/v1'],
				['kimi', 'Kimi Code', 'https://api.kimi.com/coding/v1']
			];
		}

		async providerAdd(preset: string, apiKey: string, name?: string): Promise<ProviderInstance> {
			const count = this.instances.filter((i) => i.preset === preset).length;
			const display = (await this.providerPresets()).find(([id]) => id === preset)?.[1] ?? preset;
			const inst: ProviderInstance = {
				id: `mock-${preset}-${count + 1}`,
				preset,
				name: name ?? (count === 0 ? display : `${display} ${count + 1}`)
			};
			this.instances.push(inst);
			this.apiKeys.set(inst.id, apiKey);
			return { ...inst };
		}

		async providerUpdate(instanceId: string, name?: string, baseUrl?: string): Promise<void> {
			const inst = this.instances.find((i) => i.id === instanceId);
			if (!inst) throw new Error('Provider instance not found');
			if (name) inst.name = name;
			if (baseUrl !== undefined) inst.baseUrl = baseUrl || undefined;
		}

		async providerDelete(instanceId: string): Promise<void> {
			this.instances = this.instances.filter((i) => i.id !== instanceId);
			this.apiKeys.delete(instanceId);
		}

		async providerSetApiKey(instanceId: string, apiKey: string): Promise<void> {
			this.apiKeys.set(instanceId, apiKey);
		}

		async providerGetApiKey(instanceId: string): Promise<string> {
			return this.apiKeys.get(instanceId) ?? '';
		}

		async providerValidate(instanceId: string): Promise<ModelMeta[]> {
			const inst = this.instances.find((i) => i.id === instanceId);
			return PRESET_MODELS[inst?.preset ?? ''] ?? [];
		}

		async modelsList(): Promise<InstanceModels[]> {
			return this.instances.map((instance) => ({
				instance: { ...instance },
				models: PRESET_MODELS[instance.preset] ?? []
			}));
		}

		async titleModelGet(): Promise<string | null> {
			return this.titleModel;
		}

		async titleModelSet(value: string | null): Promise<void> {
			this.titleModel = value;
		}

		async toolSchemas(): Promise<ToolSettingsEntry[]> {
			return [
				{ name: 'read_file', description: 'Read remote files', defaultRequiresApproval: false, options: [], enabled: true, requireApproval: false, values: {} },
				{ name: 'write_file', description: 'Write remote files', defaultRequiresApproval: true, options: [], enabled: true, requireApproval: true, values: {} },
				{ name: 'edit_file', description: 'Edit remote files', defaultRequiresApproval: true, options: [], enabled: true, requireApproval: true, values: {} },
				{ name: 'list_directory', description: 'List remote directories', defaultRequiresApproval: false, options: [], enabled: true, requireApproval: false, values: {} },
				{ name: 'terminal', description: 'Run commands on remote host', defaultRequiresApproval: true, options: [], enabled: true, requireApproval: true, values: {} },
				{ name: 'fetch', description: 'Fetch a URL as Markdown', defaultRequiresApproval: true, options: [], enabled: true, requireApproval: true, values: {} }
			];
		}

		async toolSetConfig(tool: string, config: ToolConfig): Promise<void> {
			this.toolConfigs.set(tool, config);
		}

		async migrateLegacySettings(): Promise<boolean> {
			return false;
		}

		// ── scenario catalog & player ────────────────────────────────────────────

		listScenarios(): MockScenarioMeta[] {
			return Object.entries(this.scenarios).map(([name, s]) => ({
				name,
				label: s.label,
				description: s.description
			}));
		}

		getScenario(): string {
			return this.activeScenario;
		}

		setScenario(name: string): void {
			if (this.scenarios[name]) this.activeScenario = name;
		}

		/**
		 * Create a fresh thread for `name` and (except for pure-display scenarios)
		 * drive it with the scenario's canned prompt. Returns the new thread id.
		 */
		async playScenario(name: string): Promise<string> {
			this.setScenario(name);
			const thread = this.newThread(this.lastIdentity);
			this.scenarios[name]?.seed?.(thread);
			const prompt = this.scenarios[name]?.prompt;
			if (prompt) {
				await this.sendMessage(this.lastIdentity, thread.summary.id, prompt, {
					model: 'mock-or/anthropic/claude-sonnet-4.5',
					thinking: true,
					effort: 'medium'
				});
			}
			return thread.summary.id;
		}

		private scenarios: Record<string, MockScenario & { prompt?: string }> = {
			'text-thinking': {
				label: 'Text + thinking',
				description: 'Streaming reasoning, then an answer; usage grows.',
				prompt: 'Explain how the agent loop works.',
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.sleep(400);
					await ctx.streamThinking(
						msg,
						'The user asks about the agent loop. Let me think through the rounds: request, stream, tool calls, results, repeat until the model stops.'
					);
					await ctx.sleep(250);
					await ctx.streamText(
						msg,
						'## Agent loop\n\nEach turn is a **round**: one streamed assistant message.\n\n- Text and thinking stream token by token\n- Tool calls drip their arguments live\n- Results feed back as new messages\n\nThe run ends when a round produces no tool calls.'
					);
					ctx.addUsage(220);
					ctx.finishMessage(msg, 0, startedAt);
				}
			},
			'tool-read-approval': {
				label: 'read_file approval',
				description: 'A sensitive read waits for Accept before content arrives.',
				prompt: 'Read /etc/ssh/sshd_config and tell me whether root login is allowed.',
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.sleep(300);
					await ctx.streamText(msg, 'Let me check the SSH daemon config.\n');
					const call = await ctx.streamToolCall(msg, 'read_file', { path: '/etc/ssh/sshd_config' });
					const ok = await ctx.requestApproval(call, '/etc/ssh/sshd_config', [
						{ kind: 'sensitive_pattern', pattern: '**/.ssh/*' }
					]);
					if (!ok) {
						ctx.setToolStatus(call, 'rejected', {
							llmText: 'Permission to run tool denied by user',
							isError: true
						});
						await ctx.sleep(300);
						await ctx.streamText(msg, 'Understood — I will not read that file.');
					} else {
						ctx.setToolStatus(call, 'running');
						await ctx.sleep(500);
						ctx.setToolStatus(call, 'success', {
							llmText: SSHD_CONFIG,
							isError: false,
							uiPayload: { path: '/etc/ssh/sshd_config', content: SSHD_CONFIG }
						});
						await ctx.sleep(250);
						await ctx.streamText(
							msg,
							'Root login is set to `prohibit-password` (key only), and password auth is disabled entirely. That is the recommended posture.'
						);
					}
					ctx.addUsage(140);
					ctx.finishMessage(msg, 1, startedAt);
				}
			},
			'tool-edit-diff': {
				label: 'edit_file diff',
				description: 'An nginx edit shows a unified diff in the approval card.',
				prompt: 'Switch the nginx site to HTTPS on port 443.',
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.sleep(300);
					const call = await ctx.streamToolCall(msg, 'edit_file', NGINX_EDIT_ARGS);
					const ok = await ctx.requestApproval(
						call,
						'/etc/nginx/sites-available/reach.conf',
						[],
						{ diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
					);
					if (!ok) {
						ctx.setToolStatus(call, 'rejected', {
							llmText: 'Permission to run tool denied by user\nNo edits were made.',
							isError: true
						});
						await ctx.streamText(msg, 'No problem — nothing was changed.');
					} else {
						ctx.setToolStatus(call, 'running');
						await ctx.sleep(600);
						ctx.setToolStatus(call, 'success', {
							llmText: `Edited ${NGINX_EDIT_ARGS.path}:\n\n\`\`\`diff\n${NGINX_DIFF}\n\`\`\``,
							isError: false,
							uiPayload: { diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
						});
						await ctx.streamText(msg, 'Done — the site now listens on 443 with the LE certs. Run `nginx -t` to verify.');
					}
					ctx.addUsage(180);
					ctx.finishMessage(msg, 1, startedAt);
				}
			},
			'tool-edit-reject': {
				label: 'edit_file rejected',
				description: 'Same diff card; click Reject to see the rejected state.',
				prompt: 'Apply the HTTPS change to the nginx site.',
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.sleep(300);
					const call = await ctx.streamToolCall(msg, 'edit_file', NGINX_EDIT_ARGS);
					const ok = await ctx.requestApproval(
						call,
						'/etc/nginx/sites-available/reach.conf',
						[],
						{ diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
					);
					if (ok) {
						ctx.setToolStatus(call, 'success', {
							llmText: 'Edited.',
							isError: false,
							uiPayload: { diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
						});
					} else {
						ctx.setToolStatus(call, 'rejected', {
							llmText: 'Permission to run tool denied by user\nNo edits were made.',
							isError: true,
							uiPayload: { diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
						});
						await ctx.streamText(msg, 'Understood, leaving the site on port 80.');
					}
					ctx.addUsage(120);
					ctx.finishMessage(msg, 1, startedAt);
				}
			},
			'terminal-live': {
				label: 'terminal live',
				description: 'A PTY-backed command streams ANSI-colored output.',
				prompt: 'Pull the latest site and reload nginx.',
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.sleep(300);
					const call = await ctx.streamToolCall(msg, 'terminal', { command: TERMINAL_COMMAND });
					const ok = await ctx.requestApproval(call, TERMINAL_COMMAND, [
						{ kind: 'dangerous_keywords', keywords: ['systemctl'] }
					]);
					if (!ok) {
						ctx.setToolStatus(call, 'rejected', {
							llmText: 'Permission to run tool denied by user',
							isError: true
						});
					} else {
						ctx.setToolStatus(call, 'running');
						for (const chunk of TERMINAL_CHUNKS) {
							if (ctx.terminalStopped(call.id)) break;
							ctx.emitTerminal(call.id, chunk);
							await ctx.sleep(280);
						}
						const out = TERMINAL_CHUNKS.map(stripAnsi).join('');
						ctx.setToolStatus(call, 'success', {
							llmText: `Command executed successfully.\n\n\`\`\`\n${out.trim()}\n\`\`\``,
							isError: false,
							uiPayload: {
								command: TERMINAL_COMMAND,
								output: out.trim(),
								exitCode: 0
							}
						});
						await ctx.streamText(msg, 'Deployed and reloaded cleanly — no errors.');
					}
					ctx.addUsage(160);
					ctx.finishMessage(msg, 1, startedAt);
				}
			},
			'multi-round-tools': {
				label: 'Multi-round tools',
				description:
					'read → edit → reload across three rounds; each round is its own message with its own meta anchor.',
				prompt: 'Check the nginx site config, switch it to HTTPS, and reload nginx.',
				run: async (ctx) => {
					// Round 1: read the current config (auto-approved read).
					let startedAt = Date.now();
					let msg = ctx.beginAssistant();
					await ctx.sleep(300);
					await ctx.streamText(msg, "I'll start by reading the current site config.\n");
					const readCall = await ctx.streamToolCall(msg, 'read_file', {
						path: '/etc/nginx/sites-available/reach.conf'
					});
					ctx.setToolStatus(readCall, 'running');
					await ctx.sleep(450);
					ctx.setToolStatus(readCall, 'success', {
						llmText: NGINX_SITE_CONFIG,
						isError: false,
						uiPayload: {
							path: '/etc/nginx/sites-available/reach.conf',
							content: NGINX_SITE_CONFIG
						}
					});
					ctx.addUsage(80);
					// Persisted id ≠ streaming id, like the real backend's tool-call rounds.
					ctx.finishMessage(msg, 1, startedAt, `${msg}p`);

					// Round 2: apply the HTTPS edit (diff approval).
					startedAt = Date.now();
					msg = ctx.beginAssistant();
					await ctx.sleep(350);
					await ctx.streamText(
						msg,
						"The site is still plain HTTP on port 80. I'll switch it to 443 with the LE certs.\n"
					);
					const editCall = await ctx.streamToolCall(msg, 'edit_file', NGINX_EDIT_ARGS);
					const ok = await ctx.requestApproval(
						editCall,
						'/etc/nginx/sites-available/reach.conf',
						[],
						{ diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
					);
					if (!ok) {
						ctx.setToolStatus(editCall, 'rejected', {
							llmText: 'Permission to run tool denied by user\nNo edits were made.',
							isError: true,
							uiPayload: { diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
						});
						ctx.addUsage(60);
						ctx.finishMessage(msg, 1, startedAt, `${msg}p`);
						// Final round: acknowledge the rejection; no reload.
						startedAt = Date.now();
						msg = ctx.beginAssistant();
						await ctx.sleep(300);
						await ctx.streamText(
							msg,
							"Understood — the site stays on port 80 and I won't reload nginx. Nothing was changed."
						);
						ctx.addUsage(40);
						ctx.finishMessage(msg, 0, startedAt);
						return;
					}
					ctx.setToolStatus(editCall, 'running');
					await ctx.sleep(500);
					ctx.setToolStatus(editCall, 'success', {
						llmText: `Edited ${NGINX_EDIT_ARGS.path}:\n\n\`\`\`diff\n${NGINX_DIFF}\n\`\`\``,
						isError: false,
						uiPayload: { diff: NGINX_DIFF_PREVIEW, path: NGINX_EDIT_ARGS.path }
					});
					ctx.addUsage(120);
					ctx.finishMessage(msg, 1, startedAt, `${msg}p`);

					// Round 3: test the config and reload nginx, with live output.
					startedAt = Date.now();
					msg = ctx.beginAssistant();
					await ctx.sleep(350);
					await ctx.streamText(msg, "Config updated. Now I'll test it and reload nginx.\n");
					const termCall = await ctx.streamToolCall(msg, 'terminal', {
						command: NGINX_RELOAD_COMMAND
					});
					ctx.setToolStatus(termCall, 'running');
					for (const chunk of NGINX_RELOAD_CHUNKS) {
						if (ctx.terminalStopped(termCall.id)) break;
						ctx.emitTerminal(termCall.id, chunk);
						await ctx.sleep(280);
					}
					const out = NGINX_RELOAD_CHUNKS.map(stripAnsi).join('');
					ctx.setToolStatus(termCall, 'success', {
						llmText: `Command executed successfully.\n\n\`\`\`\n${out.trim()}\n\`\`\``,
						isError: false,
						uiPayload: {
							command: NGINX_RELOAD_COMMAND,
							output: out.trim(),
							exitCode: 0
						}
					});
					ctx.addUsage(100);
					ctx.finishMessage(msg, 1, startedAt, `${msg}p`);

					// Round 4: final summary, no tool calls.
					startedAt = Date.now();
					msg = ctx.beginAssistant();
					await ctx.sleep(300);
					await ctx.streamText(
						msg,
						'All done — the site now listens on 443 with the LE certs, `nginx -t` passed, and the reload came back clean.'
					);
					ctx.addUsage(50);
					ctx.finishMessage(msg, 0, startedAt);
				}
			},
			'parallel-tools': {
				label: 'Parallel tool calls',
				description:
					'One round with three tool calls: two approvals pending at once, one auto-run.',
				prompt: 'Check the sshd and nginx configs, then test and reload nginx.',
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.sleep(300);
					await ctx.streamText(
						msg,
						"I'll pull both configs and reload nginx — all in one go.\n"
					);

					// All three calls stream into the same message, then run
					// concurrently; each approval resolves independently.
					const sshdCall = await ctx.streamToolCall(msg, 'read_file', {
						path: '/etc/ssh/sshd_config'
					});
					const nginxCall = await ctx.streamToolCall(msg, 'read_file', {
						path: '/etc/nginx/sites-available/reach.conf'
					});
					const reloadCall = await ctx.streamToolCall(msg, 'terminal', {
						command: NGINX_RELOAD_COMMAND
					});

					let sshdApproved = false;
					let reloadApproved = false;

					const sshdFlow = (async () => {
						const ok = await ctx.requestApproval(sshdCall, '/etc/ssh/sshd_config', [
							{ kind: 'sensitive_pattern', pattern: '**/.ssh/*' }
						]);
						sshdApproved = ok;
						if (!ok) {
							ctx.setToolStatus(sshdCall, 'rejected', {
								llmText: 'Permission to run tool denied by user',
								isError: true
							});
							return;
						}
						ctx.setToolStatus(sshdCall, 'running');
						await ctx.sleep(500);
						ctx.setToolStatus(sshdCall, 'success', {
							llmText: SSHD_CONFIG,
							isError: false,
							uiPayload: { path: '/etc/ssh/sshd_config', content: SSHD_CONFIG }
						});
					})();

					const nginxFlow = (async () => {
						// Not approval-gated: starts running right away.
						ctx.setToolStatus(nginxCall, 'running');
						await ctx.sleep(450);
						ctx.setToolStatus(nginxCall, 'success', {
							llmText: NGINX_SITE_CONFIG,
							isError: false,
							uiPayload: {
								path: '/etc/nginx/sites-available/reach.conf',
								content: NGINX_SITE_CONFIG
							}
						});
					})();

					const reloadFlow = (async () => {
						const ok = await ctx.requestApproval(reloadCall, NGINX_RELOAD_COMMAND, [
							{ kind: 'dangerous_keywords', keywords: ['systemctl'] }
						]);
						reloadApproved = ok;
						if (!ok) {
							ctx.setToolStatus(reloadCall, 'rejected', {
								llmText: 'Permission to run tool denied by user',
								isError: true
							});
							return;
						}
						ctx.setToolStatus(reloadCall, 'running');
						for (const chunk of NGINX_RELOAD_CHUNKS) {
							if (ctx.terminalStopped(reloadCall.id)) break;
							ctx.emitTerminal(reloadCall.id, chunk);
							await ctx.sleep(280);
						}
						const out = NGINX_RELOAD_CHUNKS.map(stripAnsi).join('');
						ctx.setToolStatus(reloadCall, 'success', {
							llmText: `Command executed successfully.\n\n\`\`\`\n${out.trim()}\n\`\`\``,
							isError: false,
							uiPayload: {
								command: NGINX_RELOAD_COMMAND,
								output: out.trim(),
								exitCode: 0
							}
						});
					})();

					await Promise.all([sshdFlow, nginxFlow, reloadFlow]);
					ctx.addUsage(160);
					// Persisted id ≠ streaming id, like the real backend's tool-call rounds.
					ctx.finishMessage(msg, 3, startedAt, `${msg}p`);

					// Round 2: summarize what actually happened.
					const round2At = Date.now();
					const follow = ctx.beginAssistant();
					await ctx.sleep(350);
					const notes = [
						sshdApproved
							? 'sshd is in good shape: root login is key-only and password auth is off.'
							: 'You declined the sshd read, so I could not check it.',
						'The nginx site is still plain HTTP on port 80.',
						reloadApproved
							? '`nginx -t` passed and the reload came back clean.'
							: 'You declined the reload, so nginx was not reloaded.'
					];
					await ctx.streamText(follow, notes.join(' '));
					ctx.addUsage(60);
					ctx.finishMessage(follow, 0, round2At);
				}
			},
			queued: {
				label: 'Queued message',
				description: 'Send a follow-up while the long answer still streams.',
				prompt: 'Explain the agent loop in detail.',
				queuedFollowUp: (text) =>
					`You asked mid-run: "${text}" — and here it is, injected at the round boundary. The first answer stayed intact.`,
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.sleep(300);
					await ctx.streamText(msg, QUEUED_ESSAY);
					ctx.addUsage(640);
					ctx.finishMessage(msg, 0, startedAt);
				}
			},
			branch: {
				label: 'Branches',
				description: 'A forked conversation with < 1 / 2 > navigation (no auto-run).',
				seed: (thread) => {
					// Hand-build a fork: user -> answer -> two sibling follow-ups.
					const mk = (
						role: 'user' | 'assistant',
						text: string,
						parentId: string | undefined,
						thread: MockThread
					): StoredMessage => {
						const msg: StoredMessage = {
							id: `m${this.nextMessageId++}`,
							threadId: thread.summary.id,
							parentId,
							branchIndex: 0,
							seq: thread.seq++,
							role,
							content: [{ type: 'text', text }],
							createdAt: Date.now()
						};
						this['link'](thread, msg);
						return msg;
					};
					const u1 = mk('user', 'How much disk is free on /?', undefined, thread);
					const a1 = mk('assistant', '`df -h /` shows 42G free of 100G (42%).', u1.id, thread);
					const u2a = mk('user', 'And memory?', a1.id, thread);
					mk('assistant', '`free -h`: 2.1G used of 8G, 5.4G available.', u2a.id, thread);
					// Sibling branch of u2a: a different follow-up the user tried earlier.
					const u2b = mk('user', 'And CPU load?', a1.id, thread);
					mk('assistant', '`uptime`: load average 0.42, 0.38, 0.35 — the box is idle.', u2b.id, thread);
					// Restore the first branch as active so < 1 / 2 > shows on the left.
					thread.activeChild.set(a1.id, 0);
					thread.summary.title = 'Server capacity check';
				},
				run: async (ctx) => {
					const startedAt = Date.now();
					const msg = ctx.beginAssistant();
					await ctx.streamText(msg, 'This branch continues from here — note the fork point stays intact.');
					ctx.addUsage(90);
					ctx.finishMessage(msg, 0, startedAt);
				}
			},
			error: {
				label: 'Error',
				description: 'The stream fails mid-answer and surfaces the error.',
				prompt: 'Deploy the new release.',
				run: async (ctx) => {
					const msg = ctx.beginAssistant();
					await ctx.sleep(300);
					await ctx.streamText(msg, 'Starting the deploy');
					await ctx.sleep(400);
					ctx.emitError('API error 529: overloaded_error — the provider is at capacity. Try again in a moment.');
				}
			}
		};
	}

	export const mockBackend = new MockAgentBackend();
