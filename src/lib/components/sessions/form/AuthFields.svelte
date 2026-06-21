<script lang="ts">
	import Input from '$lib/components/shared/Input.svelte';
	import Button from '$lib/components/shared/Button.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';

	interface Props {
		authType: string;
		password: string;
		keyPath: string;
		keyPassphrase: string;
		disabled?: boolean;
		showAgent?: boolean;
	}

	let {
		authType = $bindable('password'),
		password = $bindable(''),
		keyPath = $bindable(''),
		keyPassphrase = $bindable(''),
		disabled = false,
		showAgent = false,
	}: Props = $props();

	async function handleBrowse(): Promise<void> {
		try {
			const selected = await openDialog({
				multiple: false,
				directory: false,
				title: t('session.select_key_file'),
				filters: [
					{ name: t('session.ssh_private_key_filter'), extensions: ['pem', 'key', 'ppk', 'rsa', 'ed25519', 'ecdsa', 'dsa'] },
					{ name: 'All Files', extensions: ['*'] },
				],
			});
			if (typeof selected === 'string') keyPath = selected;
		} catch {
			// User cancelled the dialog
		}
	}

	let isPassword = $derived(authType === 'password' || authType === 'Password');
	let isKey = $derived(authType === 'key' || authType === 'Key');
	let isAgent = $derived(authType === 'agent' || authType === 'Agent');
</script>

<div class="auth-section">
	<span class="auth-label">{t('session.auth_method')}</span>
	<div class="auth-toggle">
		<button
			type="button"
			class="auth-btn"
			class:active={isPassword}
			{disabled}
			onclick={() => (authType = 'password')}
		>
			{t('session.auth_password')}
		</button>
		<button
			type="button"
			class="auth-btn"
			class:active={isKey}
			{disabled}
			onclick={() => (authType = 'key')}
		>
			{t('session.auth_key')}
		</button>
		{#if showAgent}
			<button
				type="button"
				class="auth-btn"
				class:active={isAgent}
				{disabled}
				onclick={() => (authType = 'agent')}
			>
				{t('session.auth_agent')}
			</button>
		{/if}
	</div>
</div>

{#if isPassword}
	<Input label={t('session.password_optional')} bind:value={password} type="password" placeholder="Stored encrypted in vault" {disabled} />
{:else if isKey}
	<div class="key-path-row">
		<div class="key-path-input">
			<Input label={t('session.key_path')} bind:value={keyPath} placeholder="~/.ssh/id_rsa" {disabled} />
		</div>
		<Button variant="secondary" size="sm" onclick={handleBrowse} {disabled}>
				{t('session.browse_key')}
		</Button>
	</div>
	<Input label={t('session.passphrase_optional')} bind:value={keyPassphrase} type="password" placeholder="Stored encrypted in vault" {disabled} />
{/if}

<style>
	.auth-section {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.auth-label {
		font-size: 0.6875rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--color-text-secondary);
	}

	.auth-toggle {
		display: flex;
		gap: 0;
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		overflow: hidden;
	}

	.auth-btn {
		flex: 1;
		padding: 7px 12px;
		font-family: var(--font-sans);
		font-size: 0.8125rem;
		font-weight: 500;
		border: none;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		transition:
			background-color var(--duration-default) var(--ease-default),
			color var(--duration-default) var(--ease-default);
	}

	.auth-btn:hover:not(:disabled) {
		background-color: rgba(255, 255, 255, 0.04);
	}

	.auth-btn.active {
		background-color: var(--color-accent);
		color: #fff;
	}

	.auth-btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.auth-btn:global(+ .auth-btn) {
		border-left: 1px solid var(--color-border);
	}

	.key-path-row {
		display: flex;
		align-items: stretch;
		gap: 8px;
	}

	.key-path-input {
		flex: 1;
		min-width: 0;
	}
</style>
