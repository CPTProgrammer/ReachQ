<script lang="ts">
	import {
		tunnelCreate,
		tunnelStart,
		tunnelStop,
		tunnelDelete,
		tunnelList,
		type TunnelConfig
	} from '$lib/ipc/tunnel';
	import { addToast } from '$lib/state/toasts.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import { listen } from '@tauri-apps/api/event';
	import { TUNNEL_TYPES, tunnelTypeLabel, type TunnelTypeName } from '$lib/utils/tunnel';
	import TunnelCard from './TunnelCard.svelte';
	import Input from '$lib/components/shared/Input.svelte';
	import Button from '$lib/components/shared/Button.svelte';
	import { untrack } from 'svelte';

	interface Props {
		connectionId?: string;
		sessionId?: string;
	}

	let { connectionId, sessionId }: Props = $props();

	let tunnels = $state<TunnelConfig[]>([]);
	let loading = $state(true);
	let showForm = $state(false);

	// Scope the panel to the active tab, like the file explorer:
	// session-bound tunnels show on any tab of their session, ephemeral
	// (quick-connect) tunnels only on the connection that created them.
	let visibleTunnels = $derived(
		tunnels.filter(
			(t) =>
				(sessionId !== undefined && t.session_id === sessionId) ||
				(connectionId !== undefined && t.connection_id === connectionId)
		)
	);

	let formType = $state<TunnelTypeName>('Local');
	let formLocalPort = $state('8080');
	let formRemoteHost = $state('localhost');
	let formRemotePort = $state('80');
	let creating = $state(false);

	// Live mapping preview shown under the form, matching TunnelCard's rendering.
	let mappingPreview = $derived.by(() => {
		const source = formType === 'Remote' ? t('tunnel.server') : 'localhost';
		const listen = formLocalPort || '?';
		if (formType === 'Dynamic') {
			return `${source}:${listen} → SOCKS`;
		}
		const host = formRemoteHost.trim() || '?';
		const port = formRemotePort || '?';
		return `${source}:${listen} → ${host}:${port}`;
	});

	async function loadTunnels(): Promise<void> {
		try {
			tunnels = await tunnelList();
		} catch (err) {
			console.error('Failed to load tunnels:', err);
		} finally {
			loading = false;
		}
	}

	function handleNewTunnel(): void {
		formType = 'Local';
		formLocalPort = '8080';
		formRemoteHost = 'localhost';
		formRemotePort = '80';
		showForm = true;
	}

	function handleCancelForm(): void {
		showForm = false;
	}

	async function handleCreateTunnel(): Promise<void> {
		// Session-bound tunnels persist in the vault; quick-connect tunnels
		// are ephemeral and bound to the runtime connection.
		if (!sessionId && !connectionId) {
			addToast(t('tunnel.no_connection'), 'error');
			return;
		}

		const localPort = parseInt(formLocalPort, 10);
		const remotePort = parseInt(formRemotePort, 10);
		const isDynamic = formType === 'Dynamic';

		if (isNaN(localPort) || localPort < 1 || localPort > 65535) {
			addToast(t('tunnel.invalid_local_port'), 'error');
			return;
		}

		// Dynamic (SOCKS) tunnels have no fixed target host/port.
		if (!isDynamic) {
			if (isNaN(remotePort) || remotePort < 1 || remotePort > 65535) {
				addToast(t('tunnel.invalid_remote_port'), 'error');
				return;
			}

			if (!formRemoteHost.trim()) {
				addToast(t('tunnel.remote_host_required'), 'error');
				return;
			}
		}

		try {
			creating = true;
			const tunnel = await tunnelCreate(
				formType,
				localPort,
				isDynamic ? '' : formRemoteHost.trim(),
				isDynamic ? 0 : remotePort,
				sessionId ? null : (connectionId ?? null),
				sessionId ?? null
			);
			tunnels.push(tunnel);
			tunnels = tunnels;
			showForm = false;
			addToast(t('tunnel.created_toast', { port: localPort }), 'success');
		} catch (err) {
			addToast(`Failed to create tunnel: ${err}`, 'error');
		} finally {
			creating = false;
		}
	}

	async function handleStart(tunnel: TunnelConfig): Promise<void> {
		try {
			await tunnelStart(tunnel.id, connectionId ?? null);
			const idx = tunnels.findIndex((t) => t.id === tunnel.id);
			if (idx >= 0) {
				tunnels[idx] = { ...tunnels[idx], active: true };
			}
			addToast(t('tunnel.started_toast', { port: tunnel.local_port }), 'success');
		} catch (err) {
			addToast(`Failed to start tunnel: ${err}`, 'error');
		}
	}

	async function handleStop(tunnel: TunnelConfig): Promise<void> {
		try {
			await tunnelStop(tunnel.id);
			const idx = tunnels.findIndex((t) => t.id === tunnel.id);
			if (idx >= 0) {
				tunnels[idx] = { ...tunnels[idx], active: false };
			}
			addToast(t('tunnel.stopped_toast', { port: tunnel.local_port }), 'info');
		} catch (err) {
			addToast(`Failed to stop tunnel: ${err}`, 'error');
		}
	}

	async function handleDelete(tunnel: TunnelConfig): Promise<void> {
		try {
			await tunnelDelete(tunnel.id);
			tunnels = tunnels.filter((t) => t.id !== tunnel.id);
			addToast(t('tunnel.removed_toast'), 'info');
		} catch (err) {
			addToast(`Failed to remove tunnel: ${err}`, 'error');
		}
	}

	$effect(() => {
		untrack(() => loadTunnels());

		// Backend-initiated stops (connection teardown kills its tunnels)
		// arrive as 'tunnel-changed'; 'vault-unlocked' covers the case where
		// this panel listed tunnels before the vault finished unlocking —
		// persistent tunnels live in the vault and read as empty while locked.
		const unlisteners: Array<() => void> = [];
		let cancelled = false;
		for (const event of ['tunnel-changed', 'vault-unlocked']) {
			listen(event, () => {
				loadTunnels();
			}).then((u) => {
				if (cancelled) u();
				else unlisteners.push(u);
			});
		}
		return () => {
			cancelled = true;
			for (const u of unlisteners) u();
		};
	});
</script>

<div class="tunnel-manager">
	<div class="actions-row">
		<button class="new-tunnel-btn" onclick={handleNewTunnel}>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none">
				<path
					d="M12 5v14M5 12h14"
					stroke="currentColor"
					stroke-width="2"
					stroke-linecap="round"
				/>
			</svg>
			{t('tunnel.new')}
		</button>
	</div>

	{#if showForm}
		<div class="create-form">
			<div class="form-header">
				<span class="form-title">{t('tunnel.create_tunnel')}</span>
				<button class="form-close" onclick={handleCancelForm} aria-label={t('common.close')}>
					<svg width="12" height="12" viewBox="0 0 14 14" fill="none">
						<path d="M1 1L13 13M13 1L1 13" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
					</svg>
				</button>
			</div>

			<div class="form-body">
				<div class="type-selector">
					{#each TUNNEL_TYPES as tunnelType (tunnelType)}
						<button
							class="type-option"
							class:selected={formType === tunnelType}
							onclick={() => (formType = tunnelType)}
						>
							{tunnelTypeLabel(tunnelType)}
						</button>
					{/each}
				</div>

				<Input label={t('tunnel.local_port')} bind:value={formLocalPort} type="number" placeholder="8080" />

				{#if formType !== 'Dynamic'}
					<Input
						label={t('tunnel.remote_host')}
						bind:value={formRemoteHost}
						placeholder="localhost"
						hint={t('tunnel.target_host_hint')}
					/>
					<Input label={t('tunnel.remote_port')} bind:value={formRemotePort} type="number" placeholder="80" />
				{/if}

				<div class="mapping-preview">
					<span class="preview-label">{t('tunnel.preview')}</span>
					<code class="preview-mapping">{mappingPreview}</code>
				</div>

				<div class="form-actions">
					<Button variant="ghost" size="sm" onclick={handleCancelForm}>{t('common.cancel')}</Button>
					<Button variant="primary" size="sm" onclick={handleCreateTunnel} disabled={creating}>
						{creating ? t('tunnel.creating') : t('tunnel.create')}
					</Button>
				</div>
			</div>
		</div>
	{/if}

	{#if loading}
		<div class="loading-state">
			<span class="spinner"></span>
			<span class="loading-text">{t('tunnel.loading')}</span>
		</div>
	{:else if visibleTunnels.length === 0 && !showForm}
		<p class="empty-state">{t('tunnel.no_tunnels')}</p>
	{:else}
		{#if visibleTunnels.length > 0}
			<div class="divider"></div>
			<div class="tunnels-scroll">
				{#each visibleTunnels as tunnel (tunnel.id)}
					<div class="tunnel-row">
						<TunnelCard
							{tunnel}
							onstart={() => handleStart(tunnel)}
							onstop={() => handleStop(tunnel)}
						/>
						<button
							class="delete-btn"
							onclick={() => handleDelete(tunnel)}
							title={t('tunnel.remove')}
							aria-label={t('tunnel.remove')}
						>
							<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
								<path d="M3 6h18" />
								<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
								<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
							</svg>
						</button>
					</div>
				{/each}
			</div>
		{/if}
	{/if}
</div>

<style>
	.tunnel-manager {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 4px 0;
	}

	.actions-row {
		display: flex;
		gap: 6px;
	}

	.new-tunnel-btn {
		display: flex;
		align-items: center;
		gap: 6px;
		flex: 1;
		padding: 8px 10px;
		font-family: var(--font-sans);
		font-size: 0.8125rem;
		font-weight: 500;
		border-radius: var(--radius-btn);
		cursor: pointer;
		color: var(--color-accent);
		background: transparent;
		border: 1px solid var(--color-accent);
		transition:
			background-color var(--duration-default) var(--ease-default),
			color var(--duration-default) var(--ease-default);
	}

	.new-tunnel-btn:hover {
		background-color: rgba(0, 122, 255, 0.1);
	}

	.new-tunnel-btn:active {
		transform: scale(0.98);
	}

	.create-form {
		border: 1px solid var(--color-border);
		border-radius: var(--radius-card, 8px);
		overflow: hidden;
	}

	.form-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 10px;
		border-bottom: 1px solid var(--color-border);
		background-color: color-mix(in srgb, var(--color-contrast) 2%, transparent);
	}

	.form-title {
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--color-text-primary);
	}

	.form-close {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 20px;
		height: 20px;
		padding: 0;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		transition: background-color var(--duration-default) var(--ease-default);
	}

	.form-close:hover {
		background-color: color-mix(in srgb, var(--color-contrast) 8%, transparent);
		color: var(--color-text-primary);
	}

	.form-body {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 10px;
	}

	.type-selector {
		display: flex;
		gap: 4px;
		background-color: color-mix(in srgb, var(--color-contrast) 3%, transparent);
		border-radius: var(--radius-btn);
		padding: 3px;
	}

	.type-option {
		flex: 1;
		padding: 5px 8px;
		font-family: var(--font-sans);
		font-size: 0.75rem;
		font-weight: 500;
		border: none;
		border-radius: 6px;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		transition:
			background-color var(--duration-default) var(--ease-default),
			color var(--duration-default) var(--ease-default);
	}

	.type-option:hover {
		color: var(--color-text-primary);
	}

	.type-option.selected {
		background-color: var(--color-accent);
		color: #fff;
	}

	.form-actions {
		display: flex;
		justify-content: flex-end;
		gap: 6px;
		padding-top: 4px;
	}

	.mapping-preview {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 8px;
		border-radius: var(--radius-btn);
		background-color: color-mix(in srgb, var(--color-contrast) 3%, transparent);
	}

	.preview-label {
		flex-shrink: 0;
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}

	.preview-mapping {
		font-family: var(--font-mono, monospace);
		font-size: 0.75rem;
		color: var(--color-text-primary);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.divider {
		height: 1px;
		background-color: var(--color-border);
		opacity: 0.5;
		margin: 2px 0;
	}

	.tunnels-scroll {
		display: flex;
		flex-direction: column;
		gap: 2px;
		overflow-y: auto;
	}

	.tunnel-row {
		display: flex;
		align-items: center;
		gap: 2px;
	}

	.tunnel-row :global(.tunnel-card) {
		flex: 1;
	}

	.delete-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		padding: 0;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		opacity: 0;
		transition:
			background-color var(--duration-default) var(--ease-default),
			color var(--duration-default) var(--ease-default),
			opacity var(--duration-default) var(--ease-default);
	}

	.tunnel-row:hover .delete-btn {
		opacity: 1;
	}

	.delete-btn:hover {
		background-color: color-mix(in srgb, var(--color-contrast) 8%, transparent);
		color: var(--color-danger);
	}

	.delete-btn:active {
		transform: scale(0.92);
	}

	.loading-state {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 8px;
		padding: 16px 0;
	}

	.loading-text {
		font-size: 0.75rem;
		color: var(--color-text-secondary);
	}

	.empty-state {
		margin: 0;
		padding: 12px 4px;
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
		opacity: 0.7;
		text-align: center;
	}

	.spinner {
		display: inline-block;
		width: 14px;
		height: 14px;
		border: 2px solid rgba(255, 255, 255, 0.15);
		border-top-color: var(--color-accent);
		border-radius: 50%;
		animation: spin 0.6s linear infinite;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
