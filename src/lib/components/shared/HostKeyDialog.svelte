<script lang="ts">
	import Modal from '$lib/components/shared/Modal.svelte';
	import Button from '$lib/components/shared/Button.svelte';
	import { getPendingHostKey, respondHostKey } from '$lib/state/host-key.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import { escapeHtml } from '$lib/utils/html';

	let request = $derived(getPendingHostKey());
	let deciding = $state(false);

	async function handleDecision(decision: 'accept' | 'accept-once' | 'reject'): Promise<void> {
		if (!request || deciding) return;
		deciding = true;
		try {
			await respondHostKey(request.host, request.port, decision);
		} catch {
			// If the pending entry was already cleaned up (e.g. timeout), just dismiss.
		} finally {
			deciding = false;
		}
	}

	function onClose(): void {
		// Map dialog close (Escape / backdrop click) to "reject".
		handleDecision('reject');
	}
</script>

{#if request}
	<Modal
		open={true}
		onclose={onClose}
		title={request.isNew ? t('host_key.new_title') : t('host_key.changed_title')}
		maxWidth="540px"
	>
		{#snippet children()}
			<div class="host-key-content">
				<div class="warning-box" class:changed={!request.isNew}>
					{#if request.isNew}
						<p>{@html t('host_key.new_body', { host: escapeHtml(request.host), port: escapeHtml(String(request.port)) })}</p>
					{:else}
						<p class="changed-warning">{t('host_key.changed_warning')}</p>
						<p>{@html t('host_key.changed_body', { host: escapeHtml(request.host), port: escapeHtml(String(request.port)) })}</p>
					{/if}
				</div>

				<div class="key-details">
					<div class="detail-row">
						<span class="label">{t('host_key.key_type')}</span>
						<code>{request.keyType}</code>
					</div>
					<div class="detail-row">
						<span class="label">{t('host_key.fingerprint')}</span>
						<code class="fingerprint">{request.fingerprint}</code>
					</div>
					{#if !request.isNew && request.oldFingerprint}
						<div class="detail-row old">
							<span class="label">{t('host_key.old_fingerprint')}</span>
							<code class="fingerprint old-fp">{request.oldFingerprint}</code>
						</div>
					{/if}
				</div>
			</div>
		{/snippet}

		{#snippet actions()}
			<Button variant="danger" onclick={() => handleDecision('reject')} disabled={deciding}>
				{t('host_key.reject')}
			</Button>
			<Button variant="secondary" onclick={() => handleDecision('accept-once')} disabled={deciding}>
				{t('host_key.accept_once')}
			</Button>
			<Button
				variant={request.isNew ? 'primary' : 'warning'}
				onclick={() => handleDecision('accept')}
				disabled={deciding}
			>
				{request.isNew ? t('host_key.accept_save') : t('host_key.update_save')}
			</Button>
		{/snippet}
	</Modal>
{/if}

<style>
	.host-key-content {
		font-size: 0.875rem;
		line-height: 1.5;
	}

	.warning-box {
		background: rgba(59, 130, 246, 0.08);
		border: 1px solid rgba(59, 130, 246, 0.25);
		border-radius: var(--radius-default);
		padding: 12px 16px;
		margin-bottom: 16px;
	}

	.warning-box.changed {
		background: rgba(239, 68, 68, 0.08);
		border-color: rgba(239, 68, 68, 0.30);
	}

	.changed-warning {
		color: #ef4444;
		font-weight: 600;
		margin-top: 0;
		margin-bottom: 8px;
	}

	.warning-box p {
		margin: 0;
	}

	.key-details {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.detail-row {
		display: flex;
		flex-direction: column;
		gap: 3px;
	}

	.label {
		font-size: 0.75rem;
		color: var(--color-text-secondary);
		font-weight: 500;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}

	.fingerprint {
		font-size: 0.75rem;
		word-break: break-all;
		background: var(--color-bg-code, rgba(0, 0, 0, 0.15));
		padding: 6px 8px;
		border-radius: 4px;
	}

	.old-fp {
		opacity: 0.5;
		text-decoration: line-through;
	}
</style>
