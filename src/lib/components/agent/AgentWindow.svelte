<script module lang="ts">
	export interface Props {
		/** SSH identity this detached window is bound to (from `?agent=`). */
		identity: string;
	}
</script>

<script lang="ts">
	import { onMount } from 'svelte';
	import {
		dockBackPanel,
		initDetachedWindowCloseHook,
		updatePanelState
	} from '$lib/state/agent.svelte';
	import AgentPanel from './AgentPanel.svelte';
	import { agentComputeIdentity } from '$lib/ipc/agent';
	import { sessionList } from '$lib/ipc/sessions';
	import { getActiveThreadId, getThreads, threadDisplayTitle } from '$lib/state/agent-threads.svelte';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { t } from '$lib/state/i18n.svelte';

	let { identity }: Props = $props();

	/** Saved-session name per computed identity (exact link match). */
	let sessionNames = $state(new Map<string, string>());
	let threads = $derived(getThreads());
	let activeThread = $derived(threads.find((x) => x.id === getActiveThreadId()));

	/** `user@host:port[#via=hash]` -> `user@host` (port and via suffix hidden). */
	function hostLabel(id: string): string {
		const base = id.split('#')[0];
		const i = base.lastIndexOf(':');
		return i > -1 && /^\d+$/.test(base.slice(i + 1)) ? base.slice(0, i) : base;
	}

	async function loadSessionNames(): Promise<void> {
		try {
			const list = await sessionList();
			const pairs = await Promise.all(
				list.map(async (s) => {
					try {
						const id = await agentComputeIdentity({
							username: s.username,
							host: s.host,
							port: s.port,
							jumpChain: s.jump_chain,
							proxy: s.proxy
						});
						return [id, s.name] as const;
					} catch {
						return null;
					}
				})
			);
			// FUTURE: instead of resolving identity -> session by searching here,
			// bind the computed identity to the session record at creation/update
			// time and look it up directly. Sessions sharing one link (the
			// identity excludes credentials) collapse to the first match here.
			const map = new Map<string, string>();
			for (const p of pairs) {
				if (p && !map.has(p[0])) map.set(p[0], p[1]);
			}
			sessionNames = map;
		} catch {
			/* no session names — fall back to user@host */
		}
	}

	/** Window title: `user@host · thread title`, session name replacing user@host. */
	let windowTitle = $derived.by(() => {
		const host = sessionNames.get(identity) ?? hostLabel(identity);
		const th = activeThread;
		const title = th ? threadDisplayTitle(th) : '';
		return title ? `${host} · ${title}` : host;
	});

	// Keep the OS-level title (taskbar/Alt-Tab) in sync with the visible bar.
	$effect(() => {
		getCurrentWindow()
			.setTitle(windowTitle)
			.catch(() => {});
	});

	let maximized = $state(false);

	async function checkMaximized(): Promise<void> {
		maximized = await getCurrentWindow().isMaximized();
	}

	function handleMinimize(): void {
		getCurrentWindow().minimize();
	}

	async function handleMaximize(): Promise<void> {
		await getCurrentWindow().toggleMaximize();
		maximized = await getCurrentWindow().isMaximized();
	}

	function handleClose(): void {
		getCurrentWindow().close();
	}

	onMount(() => {
		// A detached window *is* the panel: force it open here. The opener
		// already wrote this state; repeating it covers direct URL opens too.
		updatePanelState(identity, { open: true, detached: true });
		// Closing this window docks the panel back (closed, unless the close
		// came from the dock-back button — design 01 §1.2).
		const cleanupCloseHook = initDetachedWindowCloseHook(identity);
		let unlistenResize: (() => void) | undefined;
		let cancelled = false;
		getCurrentWindow()
			.onResized(() => void checkMaximized())
			.then((fn) => {
				if (cancelled) fn();
				else unlistenResize = fn;
			})
			.catch(() => {});
		void checkMaximized();
		void loadSessionNames();
		return () => {
			cancelled = true;
			cleanupCloseHook();
			unlistenResize?.();
		};
	});
</script>

<div class="agent-window">
	<!-- Custom title bar (the window is created undecorated), styled after
	     the main window TitleBar: dock-back anchored on the left, window
	     controls on the right. -->
	<header class="titlebar" data-tauri-drag-region>
		<div class="titlebar-left" data-tauri-drag-region>
			<button
				type="button"
				class="window-btn dock-back-button"
				title={t('agent.dock_back')}
				aria-label={t('agent.dock_back')}
				onclick={() => void dockBackPanel(identity)}
			>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
					<path class="st0" d="M 3 6 L 3 17 A 4 4 0 0 0 7 21 L 18 21 M 17 7 L 11 13 M 11 8 L 11 13 M 11 13 L 16 13"/>
					<rect x="7" y="3" width="14" height="14" rx="2"/>
				</svg>
			</button>
			<span class="window-title">{windowTitle}</span>
		</div>

		<div class="titlebar-right">
			<button class="window-btn" onclick={handleMinimize} aria-label={t('titlebar.minimize')}>
				<svg width="10" height="10" viewBox="0 0 10 10" fill="none">
					<path d="M1 5h8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
				</svg>
			</button>

			<button class="window-btn" onclick={handleMaximize} aria-label={t('titlebar.maximize')}>
				{#if maximized}
					<svg width="10" height="10" viewBox="0 0 10 10" fill="none">
						<rect x="0.5" y="2.5" width="7" height="7" rx="1" stroke="currentColor" stroke-width="1.2" fill="none" />
						<path d="M2.5 2.5V1.5a1 1 0 011-1h5a1 1 0 011 1v5a1 1 0 01-1 1H7.5" stroke="currentColor" stroke-width="1.2" fill="none" />
					</svg>
				{:else}
					<svg width="10" height="10" viewBox="0 0 10 10" fill="none">
						<rect x="1" y="1" width="8" height="8" rx="1.5" stroke="currentColor" stroke-width="1.2" />
					</svg>
				{/if}
			</button>

			<button class="window-btn window-btn-close" onclick={handleClose} aria-label={t('titlebar.close')}>
				<svg width="10" height="10" viewBox="0 0 10 10" fill="none">
					<path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
				</svg>
			</button>
		</div>
	</header>

	<div class="agent-window-body">
		<AgentPanel {identity} detached />
	</div>
</div>

<style>
	.agent-window {
		display: flex;
		flex-direction: column;
		width: 100vw;
		height: 100vh;
		overflow: hidden;
		background: var(--color-bg-elevated);
	}

	.agent-window-body {
		flex: 1;
		display: flex;
		min-height: 0;
	}

	.agent-window-body :global(.agent-panel) {
		flex: 1;
	}

	/* Same visual language as the main window TitleBar. */
	.titlebar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		height: 38px;
		min-height: 38px;
		padding: 0 5px;
		background-color: var(--color-bg-secondary);
		border-bottom: 1px solid var(--color-border);
		user-select: none;
		-webkit-app-region: drag;
	}

	.titlebar-left {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}

	.window-title {
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--color-text-primary);
		letter-spacing: 0.02em;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.titlebar-right {
		display: flex;
		align-items: center;
		gap: 2px;
		-webkit-app-region: no-drag;
	}

	.window-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		border: none;
		border-radius: var(--radius-btn);
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		-webkit-app-region: no-drag;
		transition: background-color var(--duration-default) var(--ease-default),
			color var(--duration-default) var(--ease-default);
	}

	.window-btn:hover {
		background-color: rgba(255, 255, 255, 0.06);
		color: var(--color-text-primary);
	}

	.window-btn-close:hover {
		background-color: var(--color-danger);
		color: #fff;
	}

	.dock-back-button {
		color: var(--color-accent);
	}

	.dock-back-button:hover {
		color: var(--color-accent-hover);
	}
</style>
