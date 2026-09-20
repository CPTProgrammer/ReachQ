<script module lang="ts">
	export interface Props {
		/** Owner scope: thread list + new-thread ownership. */
		scope: string;
	}
</script>

<script lang="ts">
	import { getPanelState, updatePanelState } from '$lib/state/agent.svelte';
	import {
		archiveThread,
		createThread,
		getActiveThreadId,
		getThreads,
		relativeTime,
		renameThread,
		selectThread,
		threadDisplayTitle
	} from '$lib/state/agent-threads.svelte';
	import { t } from '$lib/state/i18n.svelte';

	let { scope }: Props = $props();

	let panel = $derived(getPanelState(scope));
	let threads = $derived(getThreads());
	let activeThreadId = $derived(getActiveThreadId());

	// ── Inline rename ─────────────────────────────────────────────────────────

	let renamingId = $state<string | null>(null);
	let renameValue = $state('');

	function startRename(thread: { id: string; title: string }): void {
		renamingId = thread.id;
		renameValue = thread.title;
	}

	async function commitRename(): Promise<void> {
		const id = renamingId;
		if (!id) return;
		const title = renameValue.trim();
		renamingId = null;
		const current = threads.find((x) => x.id === id);
		if (title && title !== current?.title) await renameThread(id, title);
	}

	function renameKeydown(e: KeyboardEvent): void {
		if (e.key === 'Enter') {
			e.preventDefault();
			void commitRename();
		} else if (e.key === 'Escape') {
			e.preventDefault();
			renamingId = null;
		}
	}
</script>

{#if panel.threadsCollapsed}
	<!-- Collapsed: a 24px vertical strip; click to expand (design 01 §2). -->
	<button
		type="button"
		class="threads-strip"
		title={t('agent.threads')}
		aria-label={t('agent.threads')}
		onclick={() => updatePanelState(scope, { threadsCollapsed: false })}
	>
		<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
		<span class="strip-label">{t('agent.threads')}</span>
	</button>
{:else}
	<div class="threads-col" style:width={`${panel.threadsWidth}px`}>
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<div
			class="threads-head"
			title={t('agent.threads')}
			onclick={() => updatePanelState(scope, { threadsCollapsed: true })}
		>
			<span class="threads-title">{t('agent.threads')}</span>
			<svg class="collapse-icon" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
		</div>

		<button type="button" class="new-btn" onclick={() => void createThread(scope)}>
			<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>
			{t('agent.new_thread')}
		</button>

		<div class="threads-list">
			{#if threads.length === 0}
				<div class="threads-empty">{t('agent.no_threads')}</div>
			{/if}
			{#each threads as thread (thread.id)}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<!-- svelte-ignore a11y_click_events_have_key_events -->
				<div
					class="thread-item"
					class:active={thread.id === activeThreadId}
					onclick={() => selectThread(thread.id)}
				>
					{#if renamingId === thread.id}
						<!-- svelte-ignore a11y_autofocus -->
						<input
							class="rename-input"
							bind:value={renameValue}
							onkeydown={renameKeydown}
							onblur={() => void commitRename()}
							onclick={(e) => e.stopPropagation()}
							autofocus
						/>
					{:else}
						<div class="thread-title" class:untitled={!thread.title}>{threadDisplayTitle(thread)}</div>
						<div class="thread-time">{relativeTime(thread.updatedAt)}</div>
						<div class="thread-actions">
							<button
								type="button"
								class="icon-btn"
								title={t('agent.edit_title')}
								aria-label={t('agent.edit_title')}
								onclick={(e) => {
									e.stopPropagation();
									startRename(thread);
								}}
							>
								<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
							</button>
							<button
								type="button"
								class="icon-btn"
								title={t('agent.archive')}
								aria-label={t('agent.archive')}
								onclick={(e) => {
									e.stopPropagation();
									void archiveThread(thread.id);
								}}
							>
								<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="4" rx="1"/><path d="M5 8v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8"/><path d="M10 12h4"/></svg>
							</button>
						</div>
					{/if}
				</div>
			{/each}
		</div>
	</div>
{/if}

<style>
	.threads-strip {
		width: 24px;
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 10px;
		padding: 10px 0;
		border: none;
		border-left: 1px solid var(--color-border);
		background: var(--color-bg-secondary);
		color: var(--color-text-secondary);
		cursor: pointer;
	}

	.threads-strip:hover {
		color: var(--color-text-primary);
	}

	.strip-label {
		writing-mode: vertical-rl;
		font-size: 0.62rem;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		opacity: 0.7;
	}

	.threads-col {
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		min-height: 0;
		border-left: 1px solid var(--color-border);
		background: var(--color-bg-secondary);
	}

	.threads-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 10px;
		cursor: pointer;
		flex-shrink: 0;
		height: 32px;
	}

	.threads-title {
		font-size: 0.7rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--color-text-secondary);
	}

	.collapse-icon {
		color: var(--color-text-secondary);
		opacity: 0;
		transition: opacity 150ms ease;
	}

	.threads-head:hover .collapse-icon {
		opacity: 1;
	}

	.new-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 5px;
		margin: 0 8px 8px;
		padding: 5px 0;
		border: 1px dashed var(--color-border);
		border-radius: var(--radius-btn);
		background: transparent;
		color: var(--color-text-secondary);
		font-size: 0.72rem;
		font-family: var(--font-sans);
		cursor: pointer;
		flex-shrink: 0;
		transition:
			color 150ms ease,
			border-color 150ms ease;
	}

	.new-btn:hover {
		color: var(--color-text-primary);
		border-color: var(--color-text-secondary);
	}

	.threads-list {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
		padding: 0 6px;
		display: flex;
		flex-direction: column;
		gap: 1px;
	}

	.threads-empty {
		padding: 14px 8px;
		font-size: 0.7rem;
		color: var(--color-text-secondary);
		text-align: center;
	}

	.thread-item {
		position: relative;
		padding: 6px 8px;
		border-radius: 6px;
		cursor: pointer;
	}

	.thread-item:hover {
		background: color-mix(in srgb, var(--color-contrast) 5%, transparent);
	}

	.thread-item.active {
		background: color-mix(in srgb, var(--color-accent) 14%, transparent);
	}

	.thread-title {
		font-size: 0.75rem;
		color: var(--color-text-primary);
		overflow: hidden;
		white-space: nowrap;
		padding-right: 44px;
		position: relative;
		-webkit-mask-image: linear-gradient(to right, black calc(100% - 55px), transparent calc(100% - 11px));
		-webkit-mask-size: 100%;
		-webkit-mask-position: left;
		-webkit-mask-repeat: no-repeat;
		mask-image: linear-gradient(to right, black calc(100% - 55px), transparent calc(100% - 11px));
		mask-size: 100%;
		mask-position: left;
		mask-repeat: no-repeat;
		transition: -webkit-mask-size 0.2s ease, mask-size 0.2s ease;
	}

	.thread-title.untitled {
		color: var(--color-text-secondary);
	}

	.thread-item:hover .thread-title {
		-webkit-mask-size: calc(100% - 22px);
		mask-size: calc(100% - 22px);
	}

	.thread-time {
		margin-top: 1px;
		font-size: 0.65rem;
		color: var(--color-text-secondary);
	}

	.thread-actions {
		position: absolute;
		right: 4px;
		top: 50%;
		transform: translateY(-50%);
		display: none;
		gap: 2px;
	}

	.thread-item:hover .thread-actions {
		display: flex;
	}

	.icon-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 20px;
		height: 20px;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
	}

	.icon-btn:hover {
		background: color-mix(in srgb, var(--color-contrast) 10%, transparent);
		color: var(--color-text-primary);
	}

	.rename-input {
		width: 100%;
		box-sizing: border-box;
		padding: 3px 6px;
		border: 1px solid var(--color-accent);
		border-radius: 4px;
		background: var(--color-bg-primary);
		color: var(--color-text-primary);
		font-size: 0.75rem;
		font-family: var(--font-sans);
		outline: none;
	}
</style>
