//! IPC wrappers for the agent backend (Rust `agent_commands.rs`).
//! Types mirror the Rust serde shapes 1:1.

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// ---------------------------------------------------------------------------
// Shared types (mirror src-tauri/src/agent/types.rs + events.rs)
// ---------------------------------------------------------------------------

export interface Usage {
	promptTokens: number;
	completionTokens: number;
	cachedTokens?: number;
}

export interface MessageMetadata {
	providerInstance?: string;
	model?: string;
	usage?: Usage;
	durationMs?: number;
	/** Time to the first content delta of the round, same origin as durationMs. */
	ttftMs?: number;
	toolCallCount?: number;
}

export type ToolCallStatus =
	| 'streaming'
	| 'pending_approval'
	| 'running'
	| 'success'
	| 'failed'
	| 'rejected'
	| 'cancelled';

export interface ToolResult {
	llmText: string;
	isError: boolean;
	uiPayload?: unknown;
}

/** Structured diff for write_file / edit_file: streamed previews, approval
 *  payloads, and result uiPayloads all share this shape. */
export interface DiffPreviewLine {
	kind: 'add' | 'del' | 'ctx';
	text: string;
	oldNo?: number;
	newNo?: number;
}

export interface DiffPreviewHunk {
	oldStart: number;
	newStart: number;
	lines: DiffPreviewLine[];
}

export interface DiffPreview {
	path: string;
	isNewFile: boolean;
	hunks: DiffPreviewHunk[];
}

export type ContentBlock =
	| { type: 'text'; text: string }
	| { type: 'thinking'; text: string; providerPayload?: unknown }
	| {
			type: 'tool_call';
			id: string;
			name: string;
			args: unknown;
			status: ToolCallStatus;
			result?: ToolResult;
			/** Approval warnings; only present on snapshots enriched by the backend for live pending approvals. */
			warnings?: ApprovalWarning[];
	  };

export interface StoredMessage {
	id: string;
	threadId: string;
	parentId?: string;
	branchIndex: number;
	seq: number;
	role: 'user' | 'assistant';
	content: ContentBlock[];
	usage?: Usage;
	metadata?: MessageMetadata;
	createdAt: number;
}

export interface BranchInfo {
	index: number; // 1-based
	count: number;
}

export interface PathMessage extends StoredMessage {
	branch: BranchInfo;
}

export interface ThreadSummary {
	id: string;
	identity: string;
	title: string;
	archived: boolean;
	createdAt: number;
	updatedAt: number;
	lastUsage?: Usage;
	/** { model: "{instanceId}/{modelId}", thinking, effort } snapshot */
	model?: { model: string; thinking: boolean; effort?: string };
}

export interface ThreadSnapshot {
	thread: ThreadSummary;
	messages: PathMessage[];
}

export interface ThreadState extends ThreadSnapshot {
	running: boolean;
	/** Live diff previews for streaming tool calls, keyed by tool_call id. */
	previews?: Record<string, DiffPreview>;
}

/** Structured approval warning (mirrors Rust `ApprovalWarning`); the card
 *  maps `kind` to an i18n string so translations control wording/order. */
export type ApprovalWarning =
	| { kind: 'sensitive_pattern'; pattern: string }
	| { kind: 'dangerous_keywords'; keywords: string[] };

export interface ToolCallView {
	id: string;
	messageId: string;
	name: string;
	argsJson: string;
	status: ToolCallStatus;
	result?: ToolResult;
	warnings?: ApprovalWarning[];
}

export interface ApprovalRequest {
	toolCallId: string;
	tool: string;
	title: string;
	payload?: unknown;
	warnings: ApprovalWarning[];
}

export type AgentEvent =
	// A new assistant round started; messageId is stable across the whole
	// round (streaming, tool execution, persistence).
	| { kind: 'message_start'; threadId: string; messageId: string }
	| { kind: 'text_delta'; threadId: string; messageId: string; delta: string }
	| { kind: 'thinking_delta'; threadId: string; messageId: string; delta: string }
	| { kind: 'tool_call'; threadId: string; toolCall: ToolCallView }
	| { kind: 'tool_call_args_delta'; threadId: string; toolCallId: string; argsJsonDelta: string }
	// Patched partial args; always valid JSON (safe to JSON.parse directly).
	| { kind: 'tool_call_args_patched'; threadId: string; toolCallId: string; argsJson: string }
	| { kind: 'tool_call_preview'; threadId: string; toolCallId: string; preview: DiffPreview }
	| { kind: 'approval_needed'; threadId: string; approval: ApprovalRequest }
	| { kind: 'user_message'; threadId: string; message: StoredMessage }
	| { kind: 'usage'; threadId: string; usage: Usage }
	| { kind: 'message_done'; threadId: string; messageId: string; metadata?: MessageMetadata }
	| { kind: 'run_end'; threadId: string }
	| { kind: 'error'; threadId: string; message: string }
	| { kind: 'cancelled'; threadId: string }
	| { kind: 'terminal_output'; toolCallId: string; dataB64: string }
	| { kind: 'title_updated'; threadId: string; title: string }
	| { kind: 'thread_updated'; threadId: string };

// ---------------------------------------------------------------------------
// Models / providers / settings types
// ---------------------------------------------------------------------------

export interface ModelMeta {
	id: string;
	displayName: string;
	contextLength: number;
	maxOutputTokens?: number;
	supportsThinking: boolean;
	thinkingMandatory: boolean;
	thinkingEfforts: string[];
	defaultEffort?: string;
	supportsTools: boolean;
	pricing?: [number, number];
}

export interface ProviderInstance {
	id: string;
	preset: string; // 'openrouter' | 'deepseek' | 'kimi'
	name: string;
	baseUrl?: string;
}

export interface InstanceModels {
	instance: ProviderInstance;
	models?: ModelMeta[];
	error?: string;
}

export type ToolOptionKind =
	| { type: 'string' }
	| { type: 'number'; min?: number; max?: number }
	| { type: 'boolean' }
	| { type: 'string_list' }
	| { type: 'enum'; values: string[] };

export interface ToolOptionSpec {
	key: string;
	description: string;
	default: unknown;
}

export interface ToolSettingsEntry {
	name: string;
	description: string;
	defaultRequiresApproval: boolean;
	options: (ToolOptionSpec & ToolOptionKind)[];
	enabled: boolean;
	requireApproval: boolean;
	values: Record<string, unknown>;
}

export interface ToolConfig {
	enabled: boolean;
	requireApproval: boolean;
	options: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export interface AgentSendOpts {
	model: string; // "{instanceId}/{modelId}"
	thinking: boolean;
	effort?: string;
	connectionHint?: string;
}

export function agentSend(
	identity: string,
	threadId: string,
	text: string,
	opts: AgentSendOpts
): Promise<'started' | 'queued' | 'queued_full'> {
	return invoke('agent_send_message', { identity, threadId, text, opts });
}

export function agentCancel(threadId: string): Promise<void> {
	return invoke('agent_cancel', { threadId });
}

/** Remove the queued message (queued bar's delete/edit actions). */
export function agentDequeue(threadId: string): Promise<boolean> {
	return invoke('agent_dequeue', { threadId });
}

/** Force-send the queued message: backend atomically supersedes the queue,
 * cancels the active run, waits for it to exit, then starts a fresh run. */
export function agentSendNow(
	identity: string,
	threadId: string,
	text: string,
	opts: AgentSendOpts
): Promise<void> {
	return invoke('agent_send_now', { identity, threadId, text, opts });
}

export function agentApprove(toolCallId: string, approved: boolean): Promise<void> {
	return invoke('agent_approve', { toolCallId, approved });
}

export function agentTerminalResize(
	toolCallId: string,
	cols: number,
	rows: number
): Promise<void> {
	return invoke('agent_terminal_resize', { toolCallId, cols, rows });
}

export function agentTerminalStop(toolCallId: string): Promise<void> {
	return invoke('agent_terminal_stop', { toolCallId });
}

export const agentThreads = {
	list: (identity: string) => invoke<ThreadSummary[]>('agent_threads_list', { identity }),
	listAll: () => invoke<ThreadSummary[]>('agent_threads_list_all'),
	create: (identity: string) => invoke<ThreadSummary>('agent_thread_create', { identity }),
	rename: (threadId: string, title: string) =>
		invoke<void>('agent_thread_rename', { threadId, title }),
	archive: (threadId: string, archived: boolean) =>
		invoke<void>('agent_thread_archive', { threadId, archived }),
	delete: (threadId: string) => invoke<void>('agent_thread_delete', { threadId }),
	messages: (threadId: string) => invoke<ThreadSnapshot>('agent_thread_messages', { threadId }),
	state: (threadId: string) => invoke<ThreadState>('agent_get_thread_state', { threadId }),
	editMessage: (
		identity: string,
		threadId: string,
		messageId: string,
		newContent: string,
		opts: AgentSendOpts
	) =>
		invoke<ThreadSnapshot>('agent_edit_message', { identity, threadId, messageId, newContent, opts }),
	setActiveBranch: (threadId: string, atMessageId: string, direction: 'prev' | 'next') =>
		invoke<ThreadSnapshot>('agent_set_active_branch', { threadId, atMessageId, direction })
};

export const agentProviders = {
	list: () => invoke<ProviderInstance[]>('agent_providers_list'),
	presets: () => invoke<[string, string, string][]>('agent_provider_presets'),
	add: (preset: string, apiKey: string, name?: string, baseUrl?: string) =>
		invoke<ProviderInstance>('agent_provider_add', { preset, name, baseUrl, apiKey }),
	update: (instanceId: string, name?: string, baseUrl?: string) =>
		invoke<void>('agent_provider_update', { instanceId, name, baseUrl }),
	delete: (instanceId: string) => invoke<void>('agent_provider_delete', { instanceId }),
	setApiKey: (instanceId: string, apiKey: string) =>
		invoke<void>('agent_provider_set_api_key', { instanceId, apiKey }),
	getApiKey: (instanceId: string) => invoke<string>('agent_provider_get_api_key', { instanceId }),
	validate: (instanceId: string) => invoke<ModelMeta[]>('agent_provider_validate', { instanceId })
};

export function agentModelsList(forceRefresh = false): Promise<InstanceModels[]> {
	return invoke('agent_models_list', { forceRefresh });
}

export const agentToolSettings = {
	schemas: () => invoke<ToolSettingsEntry[]>('agent_tool_schemas'),
	setConfig: (tool: string, configValue: ToolConfig) =>
		invoke<void>('agent_set_tool_config', { tool, configValue })
};

export const agentTitleModel = {
	get: () => invoke<string | null>('agent_title_model_get'),
	set: (value: string | null) => invoke<void>('agent_title_model_set', { value })
};

export function agentMigrateLegacySettings(): Promise<boolean> {
	return invoke('agent_migrate_legacy_settings');
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/**
 * Channel name for one identity's event stream. Tauri event names only allow
 * alphanumeric, '-', '/', ':', '_', so the identity
 * ("user@host:port[#via=hash]") is base64url-encoded without padding
 * (alphabet A-Za-z0-9-_). Mirrors `AgentEvent::channel` in
 * src-tauri/src/agent/events.rs.
 */
export function agentEventChannel(identity: string): string {
	const bytes = new TextEncoder().encode(identity);
	let bin = '';
	for (const b of bytes) bin += String.fromCharCode(b);
	const b64url = btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
	return `agent-event-${b64url}`;
}

/** Subscribe to the event stream of one identity (design 01 §5). */
export function onAgentEvent(
	identity: string,
	cb: (e: AgentEvent) => void
): Promise<UnlistenFn> {
	return listen<AgentEvent>(agentEventChannel(identity), (e) => cb(e.payload));
}

/** Fired when a pending-close connection's last lease released (01 §1.4). */
export function onPendingCloseDone(
	connectionId: string,
	cb: (connectionId: string) => void
): Promise<UnlistenFn> {
	return listen<string>(`ssh-pending-close-done-${connectionId}`, () => cb(connectionId));
}
