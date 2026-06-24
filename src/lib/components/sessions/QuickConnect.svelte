<script lang="ts">
	import Modal from '$lib/components/shared/Modal.svelte';
	import Button from '$lib/components/shared/Button.svelte';
	import Input from '$lib/components/shared/Input.svelte';
	import HostPortRow from './form/HostPortRow.svelte';
	import AuthFields from './form/AuthFields.svelte';
	import ProxySection from './form/ProxySection.svelte';
	import JumpSection from './form/JumpSection.svelte';
	import Toggle from '$lib/components/shared/Toggle.svelte';
	import { sshConnect, type JumpHostConnectParams } from '$lib/ipc/ssh';
	import { createTab } from '$lib/state/tabs.svelte';
	import { getPendingHostKey, clearPendingHostKey } from '$lib/state/host-key.svelte';
	import { t } from '$lib/state/i18n.svelte';

	interface Props {
		open: boolean;
	}

	let { open = $bindable() }: Props = $props();

	let host = $state('');
	let portStr = $state('22');
	let username = $state('root');
	let authMethod = $state<'Password' | 'Key'>('Password');
	let password = $state('');
	let keyPath = $state('');
	let keyPassphrase = $state('');
	let jumpEnabled = $state(false);
	let jumpHops = $state<Array<{host: string; port: string; username: string; authType: 'Password' | 'Key' | 'Agent'; password: string; keyPath: string; keyPassphrase: string}>>([]);
	let proxyEnabled = $state(false);
	let proxyType = $state<'socks5' | 'socks4' | 'http'>('socks5');
	let proxyHost = $state('127.0.0.1');
	let proxyPort = $state('9050');
	let proxyUsername = $state('');
	let proxyPassword = $state('');
	let connecting = $state(false);
	let error = $state<string | undefined>();
	let colorInit = $state(true);

	// Auto-close this modal when a host-key verification dialog appears,
	// so the HostKeyDialog isn't blocked behind the connection-in-progress state.
	$effect(() => {
		if (connecting && getPendingHostKey()) {
			open = false;
		}
	});

	let port = $derived(parseInt(portStr, 10) || 22);
	let canConnect = $derived(host.trim().length > 0 && username.trim().length > 0 && !connecting);

	async function handleConnect(): Promise<void> {
		if (!canConnect) return;
		connecting = true;
		error = undefined;

		const id = crypto.randomUUID();

		try {
			const jumpChain: JumpHostConnectParams[] | undefined = jumpEnabled && jumpHops.length > 0
				? jumpHops.map(h => ({
					host: h.host.trim(),
					port: parseInt(h.port, 10) || 22,
					username: h.username.trim(),
					authMethod: h.authType,
					password: h.authType === 'Password' ? h.password : undefined,
					keyPath: h.authType === 'Key' ? h.keyPath.trim() : undefined,
					keyPassphrase: h.authType === 'Key' && h.keyPassphrase ? h.keyPassphrase : undefined,
				}))
				: undefined;

			const connectParams = {
				id,
				host: host.trim(),
				port,
				username: username.trim(),
				authMethod,
				password: authMethod === 'Password' ? password : undefined,
				keyPath: authMethod === 'Key' ? keyPath.trim() : undefined,
				keyPassphrase: authMethod === 'Key' && keyPassphrase ? keyPassphrase : undefined,
					cols: 80,
					rows: 24,
					jumpChain,
					colorInit,
				proxy: proxyEnabled ? {
					proxy_type: proxyType,
					host: proxyHost.trim(),
					port: parseInt(proxyPort, 10) || 9050,
					username: proxyUsername.trim() || undefined,
					password: proxyPassword || undefined,
				} : undefined,
			};
			await sshConnect(connectParams);

			const tab = createTab('ssh', `${username.trim()}@${host.trim()}`, id);
			tab.sshConnectParams = connectParams;

			// Reset form
			host = '';
			portStr = '22';
			username = 'root';
			password = '';
			keyPath = '';
			keyPassphrase = '';
			jumpEnabled = false;
			jumpHops = [];
			proxyEnabled = false;
			proxyType = 'socks5';
			proxyHost = '127.0.0.1';
			proxyPort = '9050';
			proxyUsername = '';
			proxyPassword = '';
			error = undefined;
			open = false;
			colorInit = true;
		} catch (err) {
			error = String(err);
			// If a host-key timeout killed the connection, dismiss the
			// HostKeyDialog (only if it matches this connection) and
			// keep this modal open so the user can retry.
			const pendingHK = getPendingHostKey();
			if (pendingHK && pendingHK.host === host.trim() && pendingHK.port === port) {
				clearPendingHostKey();
				open = true;
			}
		} finally {
			connecting = false;
		}
	}

	function handleClose(): void {
		if (!connecting) {
			open = false;
		}
	}
</script>

<Modal {open} onclose={handleClose} title={t('session.quick_connect')}>
	<form class="form" onsubmit={(e) => { e.preventDefault(); handleConnect(); }}>
		<HostPortRow bind:host bind:port={portStr} disabled={connecting} />

		<Input label={t('session.username')} bind:value={username} placeholder="root" disabled={connecting} />

		<AuthFields bind:authType={authMethod} bind:password bind:keyPath
			bind:keyPassphrase disabled={connecting} />

		<div class="jump-section section">
			<JumpSection bind:enabled={jumpEnabled} bind:hops={jumpHops}
				disabled={connecting} />
		</div>

		<div class="section">
			<ProxySection bind:enabled={proxyEnabled} bind:proxyType bind:host={proxyHost}
				bind:port={proxyPort} bind:username={proxyUsername}
				bind:password={proxyPassword} disabled={connecting} />
		</div>

		<div class="colorize-section section">
			<Toggle bind:checked={colorInit} label={t('session.auto_colorize')} disabled={connecting} />
		</div>

		{#if error}
			<div class="error-message">{error}</div>
		{/if}
	</form>

	{#snippet actions()}
		<Button variant="secondary" onclick={handleClose} disabled={connecting}>
			{t('common.cancel')}
		</Button>
		<Button variant="primary" onclick={handleConnect} disabled={!canConnect}>
			{#if connecting}
				<span class="spinner"></span>
				{t('session.connecting')}
			{:else}
				{t('session.connect')}
			{/if}
		</Button>
	{/snippet}
</Modal>

<style>
	.form {
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.error-message {
		padding: 8px 12px;
		font-size: 0.8125rem;
		color: var(--color-danger);
		background-color: rgba(255, 69, 58, 0.08);
		border: 1px solid rgba(255, 69, 58, 0.2);
		border-radius: var(--radius-btn);
	}

	.spinner {
		display: inline-block;
		width: 14px;
		height: 14px;
		border: 2px solid rgba(255, 255, 255, 0.3);
		border-top-color: #fff;
		border-radius: 50%;
		animation: spin 0.6s linear infinite;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}

	.section {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding-top: 4px;
	}
</style>
