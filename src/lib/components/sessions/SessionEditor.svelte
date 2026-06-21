<script lang="ts">
	import Modal from '$lib/components/shared/Modal.svelte';
	import Button from '$lib/components/shared/Button.svelte';
	import Input from '$lib/components/shared/Input.svelte';
	import HostPortRow from './form/HostPortRow.svelte';
	import AuthFields from './form/AuthFields.svelte';
	import ProxySection from './form/ProxySection.svelte';
	import JumpSection from './form/JumpSection.svelte';
	import Toggle from '$lib/components/shared/Toggle.svelte';
	import { sessionCreate, sessionUpdate, type SessionConfig, type AuthMethod, type JumpHostConfig, type Folder } from '$lib/ipc/sessions';
	import { t } from '$lib/state/i18n.svelte';

	interface Props {
		open: boolean;
		editSession?: SessionConfig;
		vaultId?: string | null; // Which vault to save to (null = private)
		folders?: Folder[];
		onsave?: () => void;
	}

	let { open = $bindable(), editSession, vaultId = null, folders = [], onsave }: Props = $props();

	let name = $state('');
	let host = $state('');
	let portStr = $state('22');
	let username = $state('root');
	let authType = $state<'Password' | 'Key' | 'Agent'>('Password');
	let password = $state('');
	let keyPath = $state('');
	let keyPassphrase = $state('');
	let tagsStr = $state('');
	let folderIdStr = $state('');
	let jumpEnabled = $state(false);
	let jumpHops = $state<Array<{host: string; port: string; username: string; authType: 'Password' | 'Key' | 'Agent'; password: string; keyPath: string; keyPassphrase: string}>>([]);
	let proxyEnabled = $state(false);
	let proxyType = $state<'socks5' | 'socks4' | 'http'>('socks5');
	let proxyHost = $state('127.0.0.1');
	let proxyPort = $state('9050');
	let proxyUsername = $state('');
	let proxyPassword = $state('');
	let saving = $state(false);
	let error = $state<string | undefined>();
	let colorInit = $state(true);

	let isEditing = $derived(!!editSession);
	let canSave = $derived(name.trim().length > 0 && host.trim().length > 0 && username.trim().length > 0 && !saving);

	// Populate fields when editing, reset when creating
	$effect(() => {
		if (editSession) {
			name = editSession.name;
			host = editSession.host;
			portStr = String(editSession.port);
			username = editSession.username;
			authType = editSession.auth_method.type;
			password = editSession.auth_method.password ?? '';
			keyPath = editSession.auth_method.path ?? '';
			keyPassphrase = editSession.auth_method.passphrase ?? '';
			tagsStr = editSession.tags.join(', ');
			folderIdStr = editSession.folder_id ?? '';
			if (editSession.jump_chain && editSession.jump_chain.length > 0) {
				jumpEnabled = true;
				jumpHops = editSession.jump_chain.map(j => ({
					host: j.host,
					port: String(j.port),
					username: j.username,
					authType: j.auth_method.type,
					password: j.auth_method.type === 'Password' ? (j.auth_method.password ?? '') : '',
					keyPath: j.auth_method.type === 'Key' ? (j.auth_method.path ?? '') : '',
					keyPassphrase: j.auth_method.type === 'Key' ? (j.auth_method.passphrase ?? '') : '',
				}));
			} else {
				jumpEnabled = false;
				jumpHops = [];
			}
			if (editSession.proxy) {
				proxyEnabled = true;
				proxyType = (editSession.proxy.proxy_type as 'socks5' | 'socks4' | 'http') ?? 'socks5';
				proxyHost = editSession.proxy.host ?? '127.0.0.1';
				proxyPort = String(editSession.proxy.port ?? 9050);
				proxyUsername = editSession.proxy.username ?? '';
				proxyPassword = editSession.proxy.password ?? '';
			} else {
				proxyEnabled = false;
			}
			colorInit = editSession.color_init ?? true;
		} else {
			name = '';
			host = '';
			portStr = '22';
			username = 'root';
			authType = 'Password';
			password = '';
			keyPath = '';
			keyPassphrase = '';
			tagsStr = '';
			folderIdStr = '';
			jumpEnabled = false;
			jumpHops = [];
			proxyEnabled = false;
			proxyType = 'socks5';
			proxyHost = '127.0.0.1';
			proxyPort = '9050';
			proxyUsername = '';
			proxyPassword = '';
			colorInit = true;
		}
		error = undefined;
	});

	async function handleSave(): Promise<void> {
		if (!canSave) return;
		saving = true;
		error = undefined;

		const port = parseInt(portStr, 10) || 22;
		const authMethod: AuthMethod = authType === 'Password'
			? { type: 'Password', password: password || undefined }
			: authType === 'Key'
				? { type: 'Key', path: keyPath.trim(), passphrase: keyPassphrase || undefined }
				: { type: 'Agent' };
		const tags = tagsStr.split(',').map(t => t.trim()).filter(Boolean);

		const jumpChain: JumpHostConfig[] | undefined = jumpEnabled && jumpHops.length > 0
			? jumpHops.map(h => {
				const hopAuth: AuthMethod = h.authType === 'Password'
					? { type: 'Password', password: h.password || undefined }
					: h.authType === 'Key'
						? { type: 'Key', path: h.keyPath.trim(), passphrase: h.keyPassphrase || undefined }
						: { type: 'Agent' };
				return {
					host: h.host.trim(),
					port: parseInt(h.port, 10) || 22,
					username: h.username.trim(),
					auth_method: hopAuth,
				};
			})
			: undefined;

		const proxyConfig = proxyEnabled ? {
			proxy_type: proxyType,
			host: proxyHost.trim(),
			port: parseInt(proxyPort, 10) || 9050,
			username: proxyUsername.trim() || null,
			password: proxyPassword || null,
		} : null;

		try {
			if (isEditing && editSession) {
				await sessionUpdate({
					...editSession,
					name: name.trim(),
					host: host.trim(),
					port,
					username: username.trim(),
					auth_method: authMethod,
					folder_id: folderIdStr || null,
					tags,
					jump_chain: jumpChain ?? editSession.jump_chain ?? null,
					proxy: proxyConfig,
					color_init: colorInit,
				});
			} else {
				await sessionCreate({
					name: name.trim(),
					host: host.trim(),
					port,
					username: username.trim(),
					authMethod: authMethod,
					folderId: folderIdStr || null,
					tags,
					vaultId,
					jumpChain: jumpChain ?? null,
					proxy: proxyConfig,
					colorInit: colorInit,
				});
			}
			onsave?.();
			open = false;
		} catch (err) {
			error = String(err);
		} finally {
			saving = false;
		}
	}

	function handleClose(): void {
		if (!saving) {
			open = false;
		}
	}
</script>

<Modal {open} onclose={handleClose} title={isEditing ? t('session.edit_session') : t('session.new')}>
	<form class="form" onsubmit={(e) => { e.preventDefault(); handleSave(); }}>
		<Input label={t('session.name')} bind:value={name} placeholder="My Server" disabled={saving} />

		<HostPortRow bind:host bind:port={portStr} disabled={saving} />

		<Input label={t('session.username')} bind:value={username} placeholder="root" disabled={saving} />

		<AuthFields bind:authType bind:password bind:keyPath bind:keyPassphrase
			disabled={saving} showAgent />

		<div class="jump-section section">
			<JumpSection bind:enabled={jumpEnabled} bind:hops={jumpHops}
				disabled={saving} showAgent />
		</div>

		<div class="section">
			<ProxySection bind:enabled={proxyEnabled} bind:proxyType bind:host={proxyHost}
				bind:port={proxyPort} bind:username={proxyUsername}
				bind:password={proxyPassword} disabled={saving} />
		</div>

		<div class="colorize-section section">
			<Toggle bind:checked={colorInit} label="Auto colorize shell" disabled={saving} />
		</div>

		<Input label={t('session.tags')} bind:value={tagsStr} placeholder="production, web, linux" disabled={saving} />

		{#if folders.length > 0}
			<div class="folder-section">
				<span class="folder-label">{t('session.folder')}</span>
				<select class="folder-select" bind:value={folderIdStr} disabled={saving}>
					<option value="">{t('session.no_folder')}</option>
					{#each folders as folder (folder.id)}
						<option value={folder.id}>{folder.name}</option>
					{/each}
				</select>
			</div>
		{/if}

		{#if error}
			<div class="error-message">{error}</div>
		{/if}
	</form>

	{#snippet actions()}
		<Button variant="secondary" onclick={handleClose} disabled={saving}>
			{t('common.cancel')}
		</Button>
		<Button variant="primary" onclick={handleSave} disabled={!canSave}>
			{#if saving}
				<span class="spinner"></span>
				{t('session.saving')}
			{:else}
				{isEditing ? t('session.update_session') : t('session.save_session')}
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

	.folder-section {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.folder-label {
		font-size: 0.6875rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--color-text-secondary);
	}

	.folder-select {
		padding: 6px 10px;
		background: var(--color-bg-primary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		color: var(--color-text-primary);
		font-family: var(--font-sans);
		font-size: 0.75rem;
		outline: none;
	}

	.folder-select:focus {
		border-color: var(--color-accent);
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
