//! AgentBackend: the seam between agent state modules and the backend.
//! Two implementations: `tauriBackend` (real, the only one the real panel
//! uses) and `mockBackend` (DEV-only, used by /preview/agent). Design 01 §4.1.

import type { UnlistenFn } from '@tauri-apps/api/event';
import {
	agentApprove as ipcApprove,
	agentCancel as ipcCancel,
	agentDequeue as ipcDequeue,
	agentSend as ipcSend,
	agentSendNow as ipcSendNow,
	agentTerminalResize as ipcResize,
	agentTerminalStop as ipcStop,
	agentThreads,
	agentToolSettings,
	agentModelsList,
	agentProviders,
	agentTitleModel,
	agentMigrateLegacySettings,
	onAgentEvent,
	type AgentEvent,
	type AgentSendOpts,
	type InstanceModels,
	type ProviderInstance,
	type ModelMeta,
	type ThreadSnapshot,
	type ThreadState,
	type ThreadSummary,
	type ToolConfig,
	type ToolSettingsEntry
} from './agent';

export interface AgentBackend {
	// messaging
	sendMessage(
			identity: string,
			threadId: string,
			text: string,
			opts: AgentSendOpts
		): Promise<'started' | 'queued' | 'queued_full'>;
		sendNow(
			identity: string,
			threadId: string,
			text: string,
			opts: AgentSendOpts
		): Promise<void>;
		cancel(threadId: string): Promise<void>;
		dequeue(threadId: string): Promise<boolean>;
	approve(toolCallId: string, approved: boolean): Promise<void>;
	terminalResize(toolCallId: string, cols: number, rows: number): Promise<void>;
	terminalStop(toolCallId: string): Promise<void>;

	// threads
	threadsList(identity: string): Promise<ThreadSummary[]>;
	threadsListAll(): Promise<ThreadSummary[]>;
	threadCreate(identity: string): Promise<ThreadSummary>;
	threadRename(threadId: string, title: string): Promise<void>;
	threadArchive(threadId: string, archived: boolean): Promise<void>;
	threadDelete(threadId: string): Promise<void>;
	threadMessages(threadId: string): Promise<ThreadSnapshot>;
	threadState(threadId: string): Promise<ThreadState>;
	threadEditMessage(
		identity: string,
		threadId: string,
		messageId: string,
		newContent: string,
		opts: AgentSendOpts
	): Promise<ThreadSnapshot>;
	threadSetActiveBranch(
		threadId: string,
		atMessageId: string,
		direction: 'prev' | 'next'
	): Promise<ThreadSnapshot>;

	// providers & models
	providersList(): Promise<ProviderInstance[]>;
	providerPresets(): Promise<[string, string, string][]>;
	providerAdd(
		preset: string,
		apiKey: string,
		name?: string,
		baseUrl?: string
	): Promise<ProviderInstance>;
	providerUpdate(instanceId: string, name?: string, baseUrl?: string): Promise<void>;
	providerDelete(instanceId: string): Promise<void>;
	providerSetApiKey(instanceId: string, apiKey: string): Promise<void>;
	providerGetApiKey(instanceId: string): Promise<string>;
	providerValidate(instanceId: string): Promise<ModelMeta[]>;
	modelsList(forceRefresh?: boolean): Promise<InstanceModels[]>;
	titleModelGet(): Promise<string | null>;
	titleModelSet(value: string | null): Promise<void>;

	// tool settings
	toolSchemas(): Promise<ToolSettingsEntry[]>;
	toolSetConfig(tool: string, config: ToolConfig): Promise<void>;

	// migration
	migrateLegacySettings(): Promise<boolean>;

	// event stream
	onEvent(identity: string, cb: (e: AgentEvent) => void): UnlistenFn | Promise<UnlistenFn>;
}

export const tauriBackend: AgentBackend = {
	sendMessage: ipcSend,
	sendNow: ipcSendNow,
	cancel: ipcCancel,
	dequeue: ipcDequeue,
	approve: ipcApprove,
	terminalResize: ipcResize,
	terminalStop: ipcStop,

	threadsList: agentThreads.list,
	threadsListAll: agentThreads.listAll,
	threadCreate: agentThreads.create,
	threadRename: agentThreads.rename,
	threadArchive: agentThreads.archive,
	threadDelete: agentThreads.delete,
	threadMessages: agentThreads.messages,
	threadState: agentThreads.state,
	threadEditMessage: agentThreads.editMessage,
	threadSetActiveBranch: agentThreads.setActiveBranch,

	providersList: agentProviders.list,
	providerPresets: agentProviders.presets,
	providerAdd: agentProviders.add,
	providerUpdate: agentProviders.update,
	providerDelete: agentProviders.delete,
	providerSetApiKey: agentProviders.setApiKey,
	providerGetApiKey: agentProviders.getApiKey,
	providerValidate: agentProviders.validate,
	modelsList: agentModelsList,
	titleModelGet: agentTitleModel.get,
	titleModelSet: agentTitleModel.set,

	toolSchemas: agentToolSettings.schemas,
	toolSetConfig: agentToolSettings.setConfig,

	migrateLegacySettings: agentMigrateLegacySettings,

	onEvent: onAgentEvent
};
