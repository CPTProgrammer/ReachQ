<script lang="ts">
	import { t } from '$lib/state/i18n.svelte';
	import { tunnelTypeLabel } from '$lib/utils/tunnel';
	import type { TunnelConfig } from '$lib/ipc/tunnel';
	import TunnelFlowIcon from './TunnelFlowIcon.svelte';

	interface Props {
		tunnel: TunnelConfig;
		onstart: () => void;
		onstop: () => void;
		ondelete: () => void;
	}

	let { tunnel, onstart, onstop, ondelete }: Props = $props();

	let typeLabel = $derived(
		tunnel.tunnel_type === 'Local'
			? 'L'
			: tunnel.tunnel_type === 'Remote'
				? 'R'
				: 'D'
	);

	let typeColor = $derived(
		tunnel.tunnel_type === 'Local'
			? 'var(--color-accent)'
			: tunnel.tunnel_type === 'Remote'
				? 'var(--color-warning)'
				: 'var(--color-success)'
	);
</script>

<div class="tunnel-card">
	<div class="card-main">
		<span
			class="type-badge"
			style:color={typeColor}
			style:border-color={typeColor}
			title={tunnelTypeLabel(tunnel.tunnel_type)}
		>
			{typeLabel}
		</span>

		<div class="tunnel-info">
			<span class="tunnel-row" class:listen={tunnel.tunnel_type === 'Remote'}>
				<TunnelFlowIcon type={tunnel.tunnel_type} />
				<span class="side-badge">{t('tunnel.server')}</span>
				<span class="addr">
					{#if tunnel.tunnel_type === 'Dynamic'}
						{t('tunnel.any_destination')}
					{:else if tunnel.tunnel_type === 'Remote'}
						localhost:{tunnel.local_port}
					{:else}
						{tunnel.remote_host}:{tunnel.remote_port}
					{/if}
				</span>
			</span>
			<span class="tunnel-row" class:listen={tunnel.tunnel_type !== 'Remote'}>
				<span class="side-badge">{t('tunnel.local_machine')}</span>
				<span class="addr">
					{#if tunnel.tunnel_type === 'Remote'}
						{tunnel.remote_host}:{tunnel.remote_port}
					{:else}
						localhost:{tunnel.local_port}
					{/if}
				</span>
			</span>
		</div>

		<div class="tunnel-status">
			<span
				class="status-dot"
				class:active={tunnel.active}
				title={tunnel.active ? t('tunnel.active') : t('tunnel.inactive')}
			></span>
		</div>
	</div>

	<div class="tunnel-actions">
		{#if tunnel.active}
			<button class="action-btn stop-btn" onclick={onstop} title={t('tunnel.stop_tunnel')} aria-label={t('tunnel.stop_tunnel')}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
					<rect x="6" y="6" width="12" height="12" rx="1" />
				</svg>
			</button>
		{:else}
			<button class="action-btn start-btn" onclick={onstart} title={t('tunnel.start_tunnel')} aria-label={t('tunnel.start_tunnel')}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
					<path d="M8 5v14l11-7z" />
				</svg>
			</button>
		{/if}
		<button class="action-btn delete-btn" onclick={ondelete} title={t('tunnel.remove')} aria-label={t('tunnel.remove')}>
			<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
				<path d="M3 6h18" />
				<path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" />
				<path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" />
			</svg>
		</button>
	</div>
</div>

<style>
	.tunnel-card {
		display: flex;
		align-items: center;
		gap: 4px;
		padding: 6px 8px;
		border-radius: var(--radius-card, 8px);
		transition: background-color var(--duration-default) var(--ease-default);
	}

	.tunnel-card:hover {
		background-color: color-mix(in srgb, var(--color-contrast) 4%, transparent);
	}

	.card-main {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.type-badge {
		flex-shrink: 0;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		font-size: 0.625rem;
		font-weight: 700;
		text-transform: uppercase;
		border: 1px solid;
		border-radius: 4px;
	}

	.tunnel-info {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.tunnel-row {
		display: flex;
		align-items: center;
		gap: 4px;
		font-size: 0.75rem;
		font-family: var(--font-mono, monospace);
		color: var(--color-text-secondary);
		white-space: nowrap;
	}

	.tunnel-row.listen {
		color: var(--color-text-primary);
	}

	.side-badge {
		flex-shrink: 0;
		padding: 0 3px;
		border-radius: 3px;
		font-size: 0.625rem;
		color: var(--color-text-secondary);
		background-color: color-mix(in srgb, var(--color-contrast) 6%, transparent);
	}

	.addr {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.tunnel-status {
		flex-shrink: 0;
		padding: 0 4px;
	}

	.status-dot {
		display: block;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background-color: var(--color-text-secondary);
		opacity: 0.4;
		transition: background-color var(--duration-default) var(--ease-default),
			opacity var(--duration-default) var(--ease-default);
	}

	.status-dot.active {
		background-color: var(--color-success);
		opacity: 1;
		box-shadow: 0 0 6px rgba(52, 199, 89, 0.4);
	}

	.tunnel-actions {
		display: flex;
		align-items: center;
		gap: 2px;
	}

	.action-btn {
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
		transition:
			background-color var(--duration-default) var(--ease-default),
			color var(--duration-default) var(--ease-default);
	}

	.action-btn:hover {
		background-color: color-mix(in srgb, var(--color-contrast) 8%, transparent);
		color: var(--color-text-primary);
	}

	.action-btn:active {
		transform: scale(0.92);
	}

	.start-btn:hover {
		color: var(--color-success);
	}

	.stop-btn:hover {
		color: var(--color-danger);
	}

	.delete-btn:hover {
		color: var(--color-danger);
	}
</style>
