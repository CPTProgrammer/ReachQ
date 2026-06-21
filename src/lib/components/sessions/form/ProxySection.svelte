<script lang="ts">
	import Input from '$lib/components/shared/Input.svelte';
	import Toggle from '$lib/components/shared/Toggle.svelte';
	import { t } from '$lib/state/i18n.svelte';

	interface Props {
		enabled: boolean;
		proxyType: string;
		host: string;
		port: string;
		username: string;
		password: string;
		disabled?: boolean;
	}

	let {
		enabled = $bindable(false),
		proxyType = $bindable('socks5'),
		host = $bindable('127.0.0.1'),
		port = $bindable('9050'),
		username = $bindable(''),
		password = $bindable(''),
		disabled = false,
	}: Props = $props();
</script>

<Toggle bind:checked={enabled} label={t('session.proxy_enable')} {disabled} />

{#if enabled}
	<div class="proxy-fields">
		<div class="proxy-type-row">
			<button type="button" class="proxy-type-btn" class:active={proxyType === 'socks5'} onclick={() => (proxyType = 'socks5')} {disabled}>SOCKS5</button>
			<button type="button" class="proxy-type-btn" class:active={proxyType === 'socks4'} onclick={() => (proxyType = 'socks4')} {disabled}>SOCKS4</button>
			<button type="button" class="proxy-type-btn" class:active={proxyType === 'http'} onclick={() => (proxyType = 'http')} {disabled}>HTTP</button>
		</div>
		<div class="row">
			<div class="field-host">
				<Input label={t('session.proxy_host')} bind:value={host} placeholder="127.0.0.1" {disabled} />
			</div>
			<div class="field-port">
				<Input label={t('session.proxy_port')} bind:value={port} type="number" placeholder="9050" {disabled} />
			</div>
		</div>
		<div class="row">
			<div class="field-host">
				<Input label={t('session.proxy_username')} bind:value={username} {disabled} />
			</div>
			<div class="field-host">
				<Input label={t('session.proxy_password')} bind:value={password} type="password" {disabled} />
			</div>
		</div>
		<p class="proxy-hint">
			{#if proxyType === 'socks5'}
				{t('session.proxy_hint_socks5')}
				{:else if proxyType === 'http'}
					{t('session.proxy_hint_http')}
				{:else}
					{t('session.proxy_hint_socks4')}
			{/if}
		</p>
	</div>
{/if}

<style>
	.proxy-fields {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 10px;
		background: rgba(255, 255, 255, 0.02);
		border: 1px solid var(--color-border);
		border-radius: 8px;
	}

	.proxy-type-row {
		display: flex;
		gap: 0;
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		overflow: hidden;
	}

	.proxy-type-btn {
		flex: 1;
		padding: 5px 8px;
		font-family: var(--font-sans);
		font-size: 0.6875rem;
		font-weight: 500;
		border: none;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		transition: background-color var(--duration-default) var(--ease-default), color var(--duration-default) var(--ease-default);
	}

	.proxy-type-btn.active {
		background-color: var(--color-accent);
		color: #fff;
	}

	.proxy-type-btn:not(.active):hover {
		background-color: rgba(255, 255, 255, 0.06);
	}

	.proxy-hint {
		margin: 0;
		font-size: 0.625rem;
		color: var(--color-text-secondary);
		opacity: 0.7;
	}

	.row {
		display: flex;
		gap: 10px;
		align-items: flex-start;
	}

	.field-host {
		flex: 1;
		min-width: 0;
	}

	.field-port {
		width: 80px;
		flex-shrink: 0;
	}
</style>
