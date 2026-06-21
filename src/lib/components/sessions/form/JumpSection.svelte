<script lang="ts">
	import Toggle from '$lib/components/shared/Toggle.svelte';
	import Input from '$lib/components/shared/Input.svelte';
	import HostPortRow from './HostPortRow.svelte';
	import AuthFields from './AuthFields.svelte';
	import { t } from '$lib/state/i18n.svelte';

	interface Hop {
		host: string;
		port: string;
		username: string;
		authType: string;
		password: string;
		keyPath: string;
		keyPassphrase: string;
	}

	interface Props {
		enabled: boolean;
		hops: Hop[];
		disabled?: boolean;
		showAgent?: boolean;
	}

	let {
		enabled = $bindable(false),
		hops = $bindable([]),
		disabled = false,
		showAgent = false,
	}: Props = $props();

	function addHop(): void {
		hops = [...hops, { host: '', port: '22', username: 'root', authType: 'password', password: '', keyPath: '', keyPassphrase: '' }];
	}

	function removeHop(index: number): void {
		hops = hops.filter((_, i) => i !== index);
	}
</script>

<div class="toggle-row">
	<Toggle bind:checked={enabled} label={t('session.jump_host_enable')} {disabled} />
	<span class="beta-badge">BETA</span>
</div>

{#if enabled}
	<p class="jump-hint">{t('session.jump_host_hint')}</p>

	{#each hops as hop, i (i)}
		<div class="jump-hop">
			<div class="jump-hop-header">
				<span class="jump-hop-label">{t('session.jump_hop', { n: String(i + 1) })}</span>
				<button type="button" class="jump-hop-remove" onclick={() => removeHop(i)} {disabled}>
					{t('session.jump_remove_hop')}
				</button>
			</div>
			<HostPortRow bind:host={hop.host} bind:port={hop.port} {disabled} />
			<Input label={t('session.username')} bind:value={hop.username} placeholder="root" {disabled} />

			<AuthFields bind:authType={hop.authType} bind:password={hop.password}
				bind:keyPath={hop.keyPath} bind:keyPassphrase={hop.keyPassphrase}
				{disabled} {showAgent} />
		</div>
	{/each}

	<button type="button" class="jump-add-btn" onclick={addHop} {disabled}>
		+ {t('session.jump_add_hop')}
	</button>
{/if}

<style>
	.toggle-row {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.beta-badge {
		padding: 1px 5px;
		font-size: 0.5rem;
		font-weight: 700;
		letter-spacing: 0.05em;
		color: #fff;
		background: linear-gradient(135deg, #ff6b35, #f7c948);
		border-radius: 3px;
		line-height: 1.4;
	}

	.jump-hint {
		margin: 0;
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}

	.jump-hop {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 10px;
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		background-color: rgba(255, 255, 255, 0.02);
	}

	.jump-hop-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
	}

	.jump-hop-label {
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--color-text-secondary);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.jump-hop-remove {
		padding: 2px 8px;
		font-family: var(--font-sans);
		font-size: 0.6875rem;
		color: var(--color-danger);
		background: transparent;
		border: 1px solid rgba(255, 69, 58, 0.3);
		border-radius: 4px;
		cursor: pointer;
	}

	.jump-hop-remove:hover:not(:disabled) {
		background-color: rgba(255, 69, 58, 0.08);
	}

	.jump-hop-remove:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.jump-add-btn {
		padding: 6px 12px;
		font-family: var(--font-sans);
		font-size: 0.75rem;
		font-weight: 500;
		color: var(--color-accent);
		background: transparent;
		border: 1px dashed var(--color-accent);
		border-radius: var(--radius-btn);
		cursor: pointer;
		transition: background-color var(--duration-default) var(--ease-default);
	}

	.jump-add-btn:hover:not(:disabled) {
		background-color: rgba(0, 122, 255, 0.08);
	}

	.jump-add-btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}
</style>
