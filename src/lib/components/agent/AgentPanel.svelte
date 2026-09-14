<script module lang="ts">
	export interface Props {
		/** SSH identity (`user@host:port[#via=hash]`, design 01 §1.1). */
		identity: string;
		/** Rendered inside a detached pop-out window: always open, fills the
		 * window, no left-edge resizer (design 01 §1.2). */
		detached?: boolean;
		/** Disable the 50%-of-parent max-width cap (mock preview page). */
		unrestrictedWidth?: boolean;
	}
</script>

<script lang="ts">
	import {
		getPanelState,
		loadThread,
		popOutPanel,
		subscribeIdentity,
		unsubscribeIdentity,
		updatePanelState
	} from '$lib/state/agent.svelte';
	import {
		getActiveThreadId,
		getThreads,
		loadThreads,
		selectThread,
		threadDisplayTitle
	} from '$lib/state/agent-threads.svelte';
	import { loadModels, loadProviders } from '$lib/state/agent-settings.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import AgentChat from './AgentChat.svelte';
	import AgentComposer from './AgentComposer.svelte';
	import AgentThreads from './AgentThreads.svelte';

	let { identity, detached = false, unrestrictedWidth = false }: Props = $props();

	let panel = $derived(getPanelState(identity));
	let activeThreadId = $derived(getActiveThreadId());
	let isOpen = $derived(detached || panel.open);
	let threads = $derived(getThreads());
	let activeThread = $derived(threads.find((x) => x.id === activeThreadId));
	let threadTitle = $derived(activeThread ? threadDisplayTitle(activeThread) : '');

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

	let asideEl = $state<HTMLElement>();

	function startPanelDrag(e: MouseEvent): void {
		e.preventDefault();
		if (!asideEl) return;
		const aside = asideEl;
		const startX = e.clientX;
		// Measure the *rendered* width: CSS max-width may be clamping the
		// stored panelWidth, so the state value can be stale here.
		const startW = aside.getBoundingClientRect().width;
		const move = (ev: MouseEvent) => {
			// The cap is half of the terminal tab's inner window (the panel's
			// parent container), not the app window.
			const parentW = aside.parentElement?.getBoundingClientRect().width ?? window.innerWidth;
			const max = unrestrictedWidth ? Infinity : parentW * 0.5;
			// Min beats max (mirroring CSS min-width vs max-width): in a
			// container narrower than 720px the panel may exceed half.
			const w = Math.round(Math.max(Math.min(startW + (startX - ev.clientX), max), 360));
			updatePanelState(identity, { panelWidth: w });
		};
		const up = () => {
			window.removeEventListener('mousemove', move);
			window.removeEventListener('mouseup', up);
			// Sync the stored width to the rendered width: CSS min-/max-width
			// may have overridden the values written during the drag.
			updatePanelState(identity, { panelWidth: Math.round(aside.getBoundingClientRect().width) });
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
	<aside
		bind:this={asideEl}
		class="agent-panel"
		class:detached
		class:unrestricted={unrestrictedWidth}
		style:width={detached ? '100%' : `${panel.panelWidth}px`}
	>
		{#if !detached}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div class="panel-resizer" onmousedown={startPanelDrag}></div>
		{/if}
		<div class="panel-body" class:has-threads={!panel.threadsCollapsed} style:--threads-w="{panel.threadsWidth}px">
			<div class="chat-col">
				{#if !detached}
					<!-- Panel-level chrome: pop-out (the detached window has its own
					     title bar with dock-back instead) + current thread title. -->
					<div class="chat-header">
						<button
							type="button"
							class="icon-btn"
							title={t('agent.pop_out')}
							aria-label={t('agent.pop_out')}
							onclick={() => void popOutPanel(identity)}
						>
							<!-- two overlapping outlined squares (front occludes back) -->
							<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
								<path class="st0" d="M 3 6 L 3 17 A 4 4 0 0 0 7 21 L 18 21 M 11 13 L 17 7 M 13 7 L 17 7 M 17 7 L 17 11"/>
								<rect x="7" y="3" width="14" height="14" rx="2"/>
							</svg>
						</button>
						<span class="chat-header-title">{threadTitle}</span>
					</div>
				{/if}
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
		min-width: 360px;
		/* Cap at half of the parent container (the terminal tab's inner
		   window); the browser re-evaluates it on any layout change. CSS
		   min-width wins over max-width, so narrow containers may exceed half. */
		max-width: 50%;
		border-left: 1px solid var(--color-border);
		background: var(--color-bg-elevated);
	}

	.agent-panel.detached {
		border-left: none;
	}

	/* Detached fills its own window; unrestricted (mock preview) has no cap. */
	.agent-panel.detached,
	.agent-panel.unrestricted {
		max-width: none;
	}

	.panel-resizer {
		position: absolute;
		left: 0px;
		top: 0;
		bottom: 0;
		width: 4px;
		cursor: col-resize;
		z-index: 10;
		background: transparent;
		transition: background-color 150ms ease;
	}

	.panel-resizer:hover {
		background: var(--color-accent);
	}

	.panel-body {
		flex: 1;
		display: flex;
		min-width: 0;
		min-height: 0;
		position: relative;
	}

	.panel-body :global(.threads-col) {
		position: absolute;
		right: 0;
		top: 0;
		bottom: 0;
		z-index: 20;
	}

	.has-threads .chat-col {
		margin-right: clamp(0px, calc(100% - 360px), var(--threads-w));
	}

	.chat-col {
		--chat-max-width: 900px;
		flex: 1;
		display: flex;
		flex-direction: column;
		min-width: 0;
		min-height: 0;
	}

	.chat-header {
		display: flex;
		align-items: center;
		gap: 6px;
		height: 32px;
		min-height: 32px;
		padding: 0 8px 0 5px;
		border-bottom: 1px solid var(--color-border);
		flex-shrink: 0;
	}

	.chat-header-title {
		font-size: 0.75rem;
		color: var(--color-text-secondary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		min-width: 0;
	}

	.icon-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		flex-shrink: 0;
	}

	.icon-btn:hover {
		background: rgba(255, 255, 255, 0.1);
		color: var(--color-text-primary);
	}

	.threads-resizer {
		position: absolute;
		right: var(--threads-w);
		top: 0;
		bottom: 0;
		width: 4px;
		margin-left: -2px;
		margin-right: -2px;
		z-index: 21;
		flex-shrink: 0;
		cursor: col-resize;
		background: transparent;
		transition: background-color 150ms ease;
	}

	.threads-resizer:hover {
		background: var(--color-accent);
	}
</style>
