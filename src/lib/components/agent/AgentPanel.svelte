<script module lang="ts">
	export interface Props {
		/** SSH identity (`user@host:port[#via=hash]`, design 01 §1.1). */
		identity: string;
		/** Rendered inside a detached pop-out window: always open, fills the
		 * window, no left-edge resizer (design 01 §1.2). */
		detached?: boolean;
	}
</script>

<script lang="ts">
	import {
		getPanelState,
		loadThread,
		subscribeIdentity,
		unsubscribeIdentity,
		updatePanelState
	} from '$lib/state/agent.svelte';
	import {
		getActiveThreadId,
		getThreads,
		loadThreads,
		selectThread
	} from '$lib/state/agent-threads.svelte';
	import { loadModels, loadProviders } from '$lib/state/agent-settings.svelte';
	import AgentChat from './AgentChat.svelte';
	import AgentComposer from './AgentComposer.svelte';
	import AgentThreads from './AgentThreads.svelte';

	let { identity, detached = false }: Props = $props();

	let panel = $derived(getPanelState(identity));
	let activeThreadId = $derived(getActiveThreadId());
	let isOpen = $derived(detached || panel.open);

	// Subscribe + load when the panel is open (design 01 §5). The event
	// subscription intentionally survives panel close (loop keeps running);
	// it is only torn down on identity change / unmount.
	$effect(() => {
		if (!isOpen) return;
		subscribeIdentity(identity);
		void loadThreads(identity);
		void loadProviders();
		void loadModels();
	});

	$effect(() => {
		const id = identity;
		return () => unsubscribeIdentity(id);
	});

	// Keep the active thread valid for this identity's list.
	$effect(() => {
		if (!isOpen) return;
		const list = getThreads();
		const active = getActiveThreadId();
		if (active && list.some((x) => x.id === active)) return;
		if (list.length > 0) selectThread(list[0].id);
	});

	// Load (or reload) the thread snapshot when the selection changes while
	// open; `lastLoaded` resets on close so reopening re-syncs (design 01 §5).
	let lastLoaded = $state<string | null>(null);
	$effect(() => {
		const open = isOpen;
		const tid = activeThreadId;
		if (!open) {
			lastLoaded = null;
			return;
		}
		if (tid && tid !== lastLoaded) {
			lastLoaded = tid;
			void loadThread(tid);
		}
	});

	// ── Two-level resize drag (design 01 §2) ─────────────────────────────────

	function startPanelDrag(e: MouseEvent): void {
		e.preventDefault();
		const startX = e.clientX;
		const startW = panel.panelWidth;
		const move = (ev: MouseEvent) => {
			const max = window.innerWidth * 0.5;
			const w = Math.round(Math.min(Math.max(startW + (startX - ev.clientX), 360), max));
			updatePanelState(identity, { panelWidth: w });
		};
		const up = () => {
			window.removeEventListener('mousemove', move);
			window.removeEventListener('mouseup', up);
		};
		window.addEventListener('mousemove', move);
		window.addEventListener('mouseup', up);
	}

	function startThreadsDrag(e: MouseEvent): void {
		e.preventDefault();
		const startX = e.clientX;
		const startW = panel.threadsWidth;
		const move = (ev: MouseEvent) => {
			const w = Math.round(Math.min(Math.max(startW + (startX - ev.clientX), 140), 320));
			updatePanelState(identity, { threadsWidth: w });
		};
		const up = () => {
			window.removeEventListener('mousemove', move);
			window.removeEventListener('mouseup', up);
		};
		window.addEventListener('mousemove', move);
		window.addEventListener('mouseup', up);
	}
</script>

{#if isOpen}
	<aside class="agent-panel" class:detached style:width={detached ? '100%' : `${panel.panelWidth}px`}>
		{#if !detached}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div class="panel-resizer" onmousedown={startPanelDrag}></div>
		{/if}
		<div class="panel-body">
			<div class="chat-col">
				<AgentChat {identity} threadId={activeThreadId} />
				<AgentComposer {identity} threadId={activeThreadId} />
			</div>
			{#if !panel.threadsCollapsed}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div class="threads-resizer" onmousedown={startThreadsDrag}></div>
			{/if}
			<AgentThreads {identity} />
		</div>
	</aside>
{/if}

<style>
	.agent-panel {
		position: relative;
		flex-shrink: 0;
		height: 100%;
		display: flex;
		border-left: 1px solid var(--color-border);
		background: var(--color-bg-elevated);
	}

	.agent-panel.detached {
		border-left: none;
	}

	.panel-resizer {
		position: absolute;
		left: -3px;
		top: 0;
		bottom: 0;
		width: 6px;
		cursor: col-resize;
		z-index: 10;
	}

	.panel-body {
		flex: 1;
		display: flex;
		min-width: 0;
		min-height: 0;
	}

	.chat-col {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-width: 0;
		min-height: 0;
	}

	.threads-resizer {
		width: 4px;
		margin-left: -2px;
		margin-right: -2px;
		z-index: 5;
		flex-shrink: 0;
		cursor: col-resize;
		background: transparent;
		transition: background-color 150ms ease;
	}

	.threads-resizer:hover {
		background: var(--color-accent);
	}
</style>
