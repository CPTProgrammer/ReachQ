<script lang="ts">
	//! Dev-only Agent panel preview (design 01 §4.1): the real panel driven by
	//! the mock backend's scenario player. No Tauri required.
	import { onMount } from 'svelte';
	import AgentPanel from '$lib/components/agent/AgentPanel.svelte';
	import { MOCK_SCOPE, mockBackend } from '$lib/ipc/agent-backend.mock';
	import { tauriBackend } from '$lib/ipc/agent-backend';
	import {
		setAgentBackend,
		updatePanelState,
		loadThread,
		subscribeScope
	} from '$lib/state/agent.svelte';
	import { loadThreads, selectThread } from '$lib/state/agent-threads.svelte';
	import { loadModels, loadProviders } from '$lib/state/agent-settings.svelte';

	// Install the mock at component init, NOT in onMount: child effects
	// (AgentPanel's subscribeScope/loadThreads/...) run before this page's
	// onMount, and they issue IPC as soon as the panel is open — the backend
	// must already be the mock by then.
	setAgentBackend(mockBackend);

	let scenario = $state(mockBackend.getScenario());
	let running = $state(false);
	let error = $state<string | null>(null);

	const scenarios = mockBackend.listScenarios();

	async function play(name: string): Promise<void> {
		running = true;
		error = null;
		try {
			const threadId = await mockBackend.playScenario(name);
			await loadThreads(MOCK_SCOPE);
			selectThread(threadId);
			await loadThread(threadId);
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			running = false;
		}
	}

	onMount(() => {
		updatePanelState(MOCK_SCOPE, { open: true });
		subscribeScope(MOCK_SCOPE);
		void loadProviders();
		void loadModels();
		void loadThreads(MOCK_SCOPE);
		void play(scenario);
		return () => setAgentBackend(tauriBackend);
	});
</script>

<div class="agent-preview">
	<aside class="scenario-picker">
		<h2>Scenarios</h2>
		{#each scenarios as s (s.name)}
			<button
				type="button"
				class="scenario"
				class:active={scenario === s.name}
				disabled={running}
				onclick={() => {
					scenario = s.name;
					void play(s.name);
				}}
			>
				<span class="scenario-label">{s.label}</span>
				<span class="scenario-desc">{s.description}</span>
			</button>
		{/each}
		{#if error}
			<p class="scenario-error">{error}</p>
		{/if}
	</aside>
	<div class="panel-host">
		<!-- No max-width cap here: the panel may fill the host (design 01 §4.1). -->
		<AgentPanel scope={MOCK_SCOPE} unrestrictedWidth />
	</div>
</div>

<style>
	.agent-preview {
		display: flex;
		gap: 20px;
		height: 100%;
	}

	.scenario-picker {
		width: 240px;
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		gap: 8px;
		overflow-y: auto;
	}

	.scenario-picker h2 {
		margin: 0 0 8px;
		font-size: 0.8125rem;
		font-weight: 600;
	}

	.scenario {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 10px 12px;
		text-align: left;
		background-color: var(--color-bg-secondary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-card);
		cursor: pointer;
		transition: border-color 120ms ease;
	}

	.scenario:hover:not(:disabled) {
		border-color: var(--color-accent);
	}

	.scenario.active {
		border-color: var(--color-accent);
	}

	.scenario:disabled {
		opacity: 0.6;
		cursor: default;
	}

	.scenario-label {
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--color-text-primary);
	}

	.scenario-desc {
		font-size: 0.75rem;
		color: var(--color-text-secondary);
		line-height: 1.4;
	}

	.scenario-error {
		font-size: 0.75rem;
		color: var(--color-error, #e5534b);
	}

	.panel-host {
		flex: 1;
		min-width: 0;
		display: flex;
		border: 1px solid var(--color-border);
		overflow: hidden;
		background-color: var(--color-bg-primary);
	}
</style>
