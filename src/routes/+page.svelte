<script lang="ts">
	import { onMount } from 'svelte';
	import { getTabs, getActiveTab, updateTabTitle, updateTabConnection, closeTab, markTabPendingClose, registerTabCloseHandler, type Tab } from '$lib/state/tabs.svelte';
	import { getActivePage } from '$lib/state/navigation.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import { ptySpawn, ptyClose } from '$lib/ipc/pty';
	import { sshDisconnect } from '$lib/ipc/ssh';
	import { onPendingCloseDone } from '$lib/ipc/agent';
	import { monitoringStart, monitoringStop, monitoringGetStats } from '$lib/ipc/monitoring';
	import { updateStats, removeStats } from '$lib/state/monitoring.svelte';
	import {
		identityForConnection,
		forgetConnection,
		getPanelState,
		subscribeIdentity,
		unsubscribeIdentity,
		getThreadRuntime,
		agentCancelRun
	} from '$lib/state/agent.svelte';
	import { getThreads, getAllThreads } from '$lib/state/agent-threads.svelte';
	import Terminal from '$lib/components/terminal/Terminal.svelte';
	import MonitoringBar from '$lib/components/terminal/MonitoringBar.svelte';
	import AgentPanel from '$lib/components/agent/AgentPanel.svelte';
	import AgentWindow from '$lib/components/agent/AgentWindow.svelte';
	import AnsiblePage from '$lib/components/ansible/AnsiblePage.svelte';
	import TofuPage from '$lib/components/tofu/TofuPage.svelte';
	import EditorWindow from '$lib/components/editor/EditorWindow.svelte';

	const isEditorWindow = typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('editor');
	// Detached agent panel window (`?agent=<identity>`, design 01 §1.2).
	const agentWindowIdentity = (typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('agent')) || null;

	/**
	 * Handle terminal title changes from OSC 2 escape sequences.
	 * When the shell sends a title like "root@hostname: /root",
	 * extract the username and update the tab title to reflect the
	 * effective user (e.g. after sudo su -).
	 */
	function handleTerminalTitleChange(tabId: string, terminalTitle: string): void {
		const tab = getTabs().find((t) => t.id === tabId);
		if (!tab || tab.type !== 'ssh') return;

		// Terminal titles from bash are typically: "user@hostname: dir"
		// Extract the username from before the first @
		const atIndex = terminalTitle.indexOf('@');
		if (atIndex <= 0) return;

		const effectiveUser = terminalTitle.substring(0, atIndex).trim();
		if (!effectiveUser) return;

		// Get the original host (IP/hostname) from the current tab title
		const currentAtIndex = tab.title.indexOf('@');
		const originalHost = currentAtIndex >= 0 ? tab.title.substring(currentAtIndex + 1) : '';
		if (!originalHost) return;

		const newTitle = `${effectiveUser}@${originalHost}`;
		if (newTitle !== tab.title) {
			updateTabTitle(tabId, newTitle);
		}
	}

	let tabs = $derived(getTabs());
	let activeTab = $derived(getActiveTab());
	let activePage = $derived(getActivePage());

	let spawnedPtys = $state(new Set<string>());
	let connectedSsh = $state(new Set<string>());
	let monitoredConnections = $state(new Set<string>());
	let pollIntervals = $state(new Map<string, ReturnType<typeof setInterval>>());

	// --- Agent panel binding (design 01 §1) -----------------------------------

	let activeIdentity = $derived(
		activeTab?.type === 'ssh' ? identityForConnection(activeTab.connectionId) : null
	);
	let activePanelOpen = $state(false);

	// Warm the panel state (lazy localStorage read mutates state, so it must not
	// run in a template expression) and subscribe to the identity's event stream
	// while its panel is open. Closing the panel intentionally keeps the
	// subscription — the agent loop continues in the background (design 01 §5).
	// A detached panel renders in its own window instead of docked here.
	$effect(() => {
		const identity = activeIdentity;
		if (!identity) {
			activePanelOpen = false;
			return;
		}
		const panel = getPanelState(identity);
		activePanelOpen = panel.open && !panel.detached;
		if (activePanelOpen) {
			subscribeIdentity(identity);
		}
	});

	// --- Pending-close tab flow (design 01 §1.4) -------------------------------

	/** Connections already torn down by the close flow — the cleanup effect must not disconnect them again. */
	const releasedConnections = new Set<string>();
	/** Unlisteners for `ssh-pending-close-done-{connectionId}`, per pending connection. */
	const pendingCloseUnlisteners = new Map<string, () => void>();

	/** Best-effort cancel of the identity's running agent loops before a force disconnect. */
	function cancelIdentityRuns(identity: string): void {
		const threadIds = new Set<string>();
		for (const thread of [...getThreads(), ...getAllThreads()]) {
			if (thread.identity === identity) threadIds.add(thread.id);
		}
		for (const threadId of threadIds) {
			if (getThreadRuntime(threadId)?.running) {
				// Preserve any queued message by returning it to the composer draft.
				agentCancelRun(threadId).catch(() => {});
			}
		}
	}

	function finalizeTabClose(tabId: string, connectionId: string): void {
		pendingCloseUnlisteners.get(connectionId)?.();
		pendingCloseUnlisteners.delete(connectionId);
		releasedConnections.add(connectionId);
		closeTab(tabId);
	}

	async function handleSshTabClose(tab: Tab): Promise<void> {
		const connectionId = tab.connectionId;
		if (!connectionId) {
			closeTab(tab.id);
			return;
		}

		if (tab.pendingClose) {
			// Second click on a pending-close tab = explicit force disconnect:
			// cancel the identity's active run, then bypass the agent lease.
			const identity = identityForConnection(connectionId);
			if (identity) cancelIdentityRuns(identity);
			try {
				await sshDisconnect(connectionId, true);
			} catch (err) {
				console.error(`Failed to force-disconnect SSH ${connectionId}:`, err);
			}
			finalizeTabClose(tab.id, connectionId);
			return;
		}

		let disconnected = true;
		try {
			disconnected = await sshDisconnect(connectionId);
		} catch (err) {
			console.error(`Failed to disconnect SSH ${connectionId}:`, err);
		}
		if (disconnected) {
			finalizeTabClose(tab.id, connectionId);
			return;
		}

		// An agent lease holds the connection: the tab switches to the
		// pending-close view and closes for real once the backend releases the
		// lease and fires `ssh-pending-close-done-{connectionId}`.
		markTabPendingClose(tab.id);
		if (!pendingCloseUnlisteners.has(connectionId)) {
			let unlisten: (() => void) | null = null;
			let cancelled = false;
			pendingCloseUnlisteners.set(connectionId, () => {
				cancelled = true;
				unlisten?.();
			});
			onPendingCloseDone(connectionId, (connId) => {
				const pendingTab = getTabs().find((t) => t.connectionId === connId && t.pendingClose);
				if (pendingTab) finalizeTabClose(pendingTab.id, connId);
			}).then((u) => {
				if (cancelled) u();
				else unlisten = u;
			});
		}
	}

	onMount(() => {
		registerTabCloseHandler(handleSshTabClose);
		return () => registerTabCloseHandler(null);
	});

	// Spawn PTY for new local tabs
	$effect(() => {
		for (const tab of tabs) {
			if (tab.type === 'local' && !spawnedPtys.has(tab.id)) {
				spawnedPtys.add(tab.id);
				ptySpawn(tab.id).catch((err) => {
					console.error(`Failed to spawn PTY for tab ${tab.id}:`, err);
				});
			}
			if (tab.type === 'ssh' && tab.connectionId) {
				connectedSsh.add(tab.connectionId);
			}
		}
	});

	// Start monitoring for new SSH connections (poll-based)
	$effect(() => {
		for (const tab of tabs) {
			if (tab.type === 'ssh' && tab.connectionId && !monitoredConnections.has(tab.connectionId)) {
				const connId = tab.connectionId;
				monitoredConnections.add(connId);

				monitoringStart(connId).catch((err) => {
					console.error(`Failed to start monitoring for ${connId}:`, err);
				});

				// Poll for stats every 3 seconds
				const poll = async () => {
					try {
						const stats = await monitoringGetStats(connId);
						updateStats(connId, stats);
					} catch {
						// Stats not available yet — ignore
					}
				};
				// Initial fetch after a short delay
				setTimeout(poll, 1500);
				const interval = setInterval(poll, 3000);
				pollIntervals.set(connId, interval);
			}
		}
	});

	// Clean up PTYs, SSH connections, and monitoring for closed tabs
	$effect(() => {
		const tabIds = new Set(tabs.map((t) => t.id));
		const activeConnectionIds = new Set(
			tabs.filter((t) => t.connectionId).map((t) => t.connectionId!)
		);

		for (const id of spawnedPtys) {
			if (!tabIds.has(id)) {
				spawnedPtys.delete(id);
				ptyClose(id).catch((err) => {
					console.error(`Failed to close PTY ${id}:`, err);
				});
			}
		}

		for (const connId of connectedSsh) {
			if (!activeConnectionIds.has(connId)) {
				connectedSsh.delete(connId);
				// The close flow already disconnected this one (or the backend did
				// on lease release) — skip the redundant disconnect.
				if (!releasedConnections.delete(connId)) {
					sshDisconnect(connId).catch((err) => {
						console.error(`Failed to disconnect SSH ${connId}:`, err);
					});
				}
				// Drop the identity mapping; stop the event subscription when no
				// other connection uses this identity anymore.
				const identity = identityForConnection(connId);
				forgetConnection(connId);
				if (
					identity &&
					!tabs.some(
						(t) => t.connectionId && t.connectionId !== connId && identityForConnection(t.connectionId) === identity
					)
				) {
					unsubscribeIdentity(identity);
				}
			}
		}

		for (const connId of monitoredConnections) {
			if (!activeConnectionIds.has(connId)) {
				monitoredConnections.delete(connId);

				monitoringStop(connId).catch((err) => {
					console.error(`Failed to stop monitoring for ${connId}:`, err);
				});

				const interval = pollIntervals.get(connId);
				if (interval) {
					clearInterval(interval);
					pollIntervals.delete(connId);
				}

				removeStats(connId);
			}
		}
	});
</script>

{#if isEditorWindow}
	<EditorWindow />
{:else if agentWindowIdentity}
	<AgentWindow identity={agentWindowIdentity} />
{:else}
	<div class="page-container">
		<div class="page-view" class:active={activePage === 'terminal'}>
			{#if tabs.length === 0}
				<div class="empty-state">
					<svg width="48" height="48" viewBox="0 0 24 24" fill="none" class="empty-icon">
						<path
							d="M4 17l6-5-6-5"
							stroke="currentColor"
							stroke-width="1.5"
							stroke-linecap="round"
							stroke-linejoin="round"
						/>
						<path
							d="M12 19h8"
							stroke="currentColor"
							stroke-width="1.5"
							stroke-linecap="round"
						/>
					</svg>
					<h2 class="empty-title">{t('terminal.title')}</h2>
					<p class="empty-subtitle">{t('terminal.empty_hint')}</p>
				</div>
			{:else}
				<div class="terminal-row">
					<div class="terminal-area">
						{#each tabs as tab (tab.id)}
							<div class="terminal-wrapper" class:active={tab.id === activeTab?.id}>
								{#if tab.pendingClose}
									<div class="pending-close">
										<div class="pending-close-spinner"></div>
										<h3 class="pending-close-title">{t('agent.pending_close_title')}</h3>
										<p class="pending-close-body">{t('agent.pending_close_body')}</p>
									</div>
								{:else}
									<Terminal
										ptyId={tab.id}
										type={tab.type}
										connectionId={tab.connectionId}
										active={tab.id === activeTab?.id}
										onTitleChange={(title) => handleTerminalTitleChange(tab.id, title)}
										sshConnectParams={tab.sshConnectParams}
										onReconnected={(newId) => updateTabConnection(tab.id, newId)}
									/>
								{/if}
							</div>
						{/each}
					</div>
					{#if activeIdentity && activePanelOpen}
						<AgentPanel identity={activeIdentity} />
					{/if}
				</div>
				<MonitoringBar connectionId={activeTab?.connectionId} sshUser={activeTab?.title?.split('@')[0]} />
			{/if}
		</div>

		{#if activePage === 'ansible'}
			<AnsiblePage />
		{:else if activePage === 'tofu'}
			<TofuPage />
		{/if}
	</div>
{/if}

<style>
	.page-container {
		display: flex;
		flex-direction: column;
		width: 100%;
		height: 100%;
		overflow: hidden;
		position: relative;
	}

	.page-view {
		position: absolute;
		inset: 0;
		display: none;
		flex-direction: column;
	}

	.page-view.active {
		display: flex;
	}

	.empty-state {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 12px;
		padding: 32px;
		flex: 1;
	}

	.empty-icon {
		color: var(--color-text-secondary);
		opacity: 0.4;
	}

	.empty-title {
		margin: 0;
		font-size: 1.5rem;
		font-weight: 500;
		color: var(--color-text-secondary);
		letter-spacing: -0.01em;
	}

	.empty-subtitle {
		margin: 0;
		font-size: 0.8125rem;
		color: var(--color-text-secondary);
		opacity: 0.6;
	}

	.terminal-row {
		flex: 1;
		display: flex;
		min-height: 0;
		overflow: hidden;
	}

	.terminal-area {
		flex: 1;
		min-width: 0;
		position: relative;
		overflow: hidden;
	}

	.terminal-wrapper {
		position: absolute;
		inset: 0;
		display: none;
	}

	.terminal-wrapper.active {
		display: block;
	}

	.pending-close {
		height: 100%;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 12px;
		padding: 32px;
	}

	/* Same spinner as the app preloader (src/app.html #preloader .loader). */
	.pending-close-spinner {
		width: 24px;
		height: 24px;
		border: 2px solid rgba(255, 255, 255, 0.06);
		border-top-color: var(--color-accent);
		border-radius: 50%;
		animation: spin 0.8s linear infinite;
		box-sizing: border-box;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}

	.pending-close-title {
		margin: 0;
		font-size: 0.875rem;
		font-weight: 600;
		color: var(--color-text-primary);
	}

	.pending-close-body {
		margin: 0;
		font-size: 0.75rem;
		line-height: 1.5;
		color: var(--color-text-secondary);
		text-align: center;
		max-width: 320px;
	}
</style>
