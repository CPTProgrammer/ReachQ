//! Agent settings state (design 01 §4, 05 §5): provider instances, model
//! list, per-identity composer state (model / thinking / effort), title
//! model, and tool configs shared with the settings page.

import { getAgentBackend } from './agent-backend.svelte';
import { getThreads } from './agent-threads.svelte';
import type {
	AgentSendOpts,
	InstanceModels,
	ModelMeta,
	ProviderInstance,
	ToolSettingsEntry
} from '$lib/ipc/agent';

// ---------------------------------------------------------------------------
// Providers & models (backend-owned config, mirrored for the UI)
// ---------------------------------------------------------------------------

let instances = $state<ProviderInstance[]>([]);
let instanceModels = $state<InstanceModels[]>([]);
let modelsLoading = $state(false);
let titleModel = $state<string | null>(null);
let toolSettings = $state<ToolSettingsEntry[]>([]);

export function getProviderInstances(): ProviderInstance[] {
	return instances;
}

export function getInstanceModels(): InstanceModels[] {
	return instanceModels;
}

export function getModelsLoading(): boolean {
	return modelsLoading;
}

export function getTitleModel(): string | null {
	return titleModel;
}

export function getToolSettings(): ToolSettingsEntry[] {
	return toolSettings;
}

export async function loadProviders(): Promise<void> {
	instances = await getAgentBackend().providersList();
}

export async function loadModels(forceRefresh = false): Promise<void> {
	modelsLoading = true;
	try {
		instanceModels = await getAgentBackend().modelsList(forceRefresh);
	} finally {
		modelsLoading = false;
	}
}

export async function loadTitleModel(): Promise<void> {
	titleModel = await getAgentBackend().titleModelGet();
}

export async function setTitleModel(value: string | null): Promise<void> {
	await getAgentBackend().titleModelSet(value);
	titleModel = value;
}

export async function loadToolSettings(): Promise<void> {
	toolSettings = await getAgentBackend().toolSchemas();
}

export async function setToolConfig(
	tool: string,
	enabled: boolean,
	requireApproval: boolean,
	options: Record<string, unknown>
): Promise<void> {
	await getAgentBackend().toolSetConfig(tool, { enabled, requireApproval, options });
	await loadToolSettings();
}

/** Find a model by "{instanceId}/{modelId}". */
export function findModel(selection: string | null | undefined): {
	instance: ProviderInstance;
	model: ModelMeta;
} | null {
	if (!selection) return null;
	const slash = selection.indexOf('/');
	if (slash < 0) return null;
	const instanceId = selection.slice(0, slash);
	const modelId = selection.slice(slash + 1);
	const group = instanceModels.find((g) => g.instance.id === instanceId);
	const model = group?.models?.find((m) => m.id === modelId);
	if (!group || !model) return null;
	return { instance: group.instance, model };
}

/** True when at least one instance has an API key and models (composer can work). */
export function agentConfigured(): boolean {
	return instanceModels.some((g) => (g.models?.length ?? 0) > 0);
}

// ---------------------------------------------------------------------------
// Per-scope composer state: model / thinking / effort
// (localStorage `reach-agent-model:{scope}`, design 05 §5)
// ---------------------------------------------------------------------------

export interface ComposerSelection {
	model: string; // "{instanceId}/{modelId}"
	thinking: boolean;
	effort: string | null;
}

let composerSelections = $state<Record<string, ComposerSelection>>({});

function modelKey(scope: string): string {
	return `reach-agent-model:${scope}`;
}

/** Scope's last-used selection (new threads). */
export function getScopeSelection(scope: string): ComposerSelection | null {
	if (composerSelections[scope]) return composerSelections[scope];
	try {
		const raw = localStorage.getItem(modelKey(scope));
		if (!raw) return null;
		const parsed = JSON.parse(raw) as { model: string; thinking?: boolean; effort?: string };
		if (!parsed.model) return null;
		const sel: ComposerSelection = {
			model: parsed.model,
			thinking: parsed.thinking ?? false,
			effort: parsed.effort ?? null
		};
		composerSelections[scope] = sel;
		return sel;
	} catch {
		return null;
	}
}

export function setScopeSelection(scope: string, sel: ComposerSelection): void {
	composerSelections[scope] = sel;
	try {
		localStorage.setItem(modelKey(scope), JSON.stringify(sel));
	} catch {
		/* non-fatal */
	}
}

/**
 * Resolve the composer selection for a thread (design 05 §5 priority):
 * thread snapshot -> scope last-used -> global default (first model of
 * the first configured instance). Stale snapshots fall back silently.
 */
export function resolveSelection(
	scope: string,
	threadSnapshot: { model: string; thinking: boolean; effort?: string } | null | undefined
): ComposerSelection | null {
	if (threadSnapshot?.model && findModel(threadSnapshot.model)) {
		return {
			model: threadSnapshot.model,
			thinking: threadSnapshot.thinking,
			effort: threadSnapshot.effort ?? null
		};
	}
	const recent = getScopeSelection(scope);
	if (recent && findModel(recent.model)) return recent;
	for (const group of instanceModels) {
		const first = group.models?.[0];
		if (first) {
			return {
				model: `${group.instance.id}/${first.id}`,
				thinking: first.thinkingMandatory,
				effort: first.defaultEffort ?? null
			};
		}
	}
	return null;
}

/**
 * Effective send options for a thread (design 01 §3.6 / 05 §5):
 * thread snapshot -> scope last-used -> global default, with the
 * thinking flag coerced against model capabilities.
 */
export function resolveSendOpts(
	scope: string,
	threadId: string | null,
	connectionId?: string
): AgentSendOpts | null {
	const summary = threadId ? getThreads().find((t) => t.id === threadId) : undefined;
	const sel =
		resolveSelection(scope, summary?.model ?? null) ??
		getScopeSelection(scope) ??
		(summary?.model
			? { model: summary.model.model, thinking: summary.model.thinking, effort: summary.model.effort ?? null }
			: null);
	if (!sel) return null;
	const meta = findModel(sel.model)?.model;
	const thinking = meta
		? meta.thinkingMandatory || (meta.supportsThinking && sel.thinking)
		: sel.thinking;
	return {
		model: sel.model,
		thinking,
		effort: thinking ? (sel.effort ?? undefined) : undefined,
		connectionHint: connectionId
	};
}
