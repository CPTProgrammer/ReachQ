<script module lang="ts">
	import type { ToolCallView } from '$lib/ipc/agent';

	export interface Props {
		call: ToolCallView;
	}
</script>

<script lang="ts">
	import {
		agentApproveCall,
		agentStopTerminal
	} from '$lib/state/agent.svelte';
	import { untrack } from 'svelte';
	import { t } from '$lib/state/i18n.svelte';
	import DiffView from './DiffView.svelte';
	import ToolTerminal from './ToolTerminal.svelte';
	import { renderMarkdown } from './markdown';
	import { argString, copyText, extractDiff, prettyJson, safeParse } from './utils';

	let { call }: Props = $props();

	// Default-expanded tools (design 01 §2.3 table); captured once at creation.
	let expanded = $state(
		untrack(
			() =>
				call.name === 'write_file' || call.name === 'edit_file' || call.name === 'terminal'
		)
	);
	let showRaw = $state(false);
	let copied = $state(false);

	let args = $derived(safeParse(call.argsJson));
	let path = $derived(argString(args, ['path', 'file_path', 'filePath', 'target_file']));
	let command = $derived(argString(args, ['command', 'cmd']));
	let url = $derived(argString(args, ['url']));

	let ended = $derived(
		call.status === 'success' ||
			call.status === 'failed' ||
			call.status === 'rejected' ||
			call.status === 'cancelled'
	);
	/** Terminal view stays hidden until approved + connected (design 01 §2.3). */
	let terminalLive = $derived(
		call.status !== 'streaming' && call.status !== 'pending_approval' && call.status !== 'rejected'
	);

	let title = $derived.by(() => {
		switch (call.name) {
			case 'read_file':
				return t('agent.tool_read_file', { path: path || '…' });
			case 'list_directory':
				return t('agent.tool_list_directory', { path: path || '…' });
			case 'terminal':
				return t('agent.tool_terminal');
			case 'fetch':
				return t('agent.tool_fetch', { url: url || '…' });
			case 'write_file':
			case 'edit_file':
				return path || call.name;
			default:
				return call.name;
		}
	});

	let diff = $derived(
		call.name === 'write_file' || call.name === 'edit_file' ? extractDiff(call) : null
	);

	/** Whether the detail area renders any content (drives the 0px collapsed look). */
	let hasDetail = $derived.by(() => {
		if (showRaw) return true;
		if (call.name === 'terminal') return true; // command block always visible
		if (!expanded) return false;
		if (call.name === 'write_file' || call.name === 'edit_file') return true;
		return call.result != null;
	});

	function copyCommand(): void {
		if (!command) return;
		copyText(command).then((ok) => {
			if (!ok) return;
			copied = true;
			setTimeout(() => (copied = false), 1500);
		});
	}
</script>

<div class="tool-card status-{call.status}">
	<!-- ── Title bar ── -->
	<div class="tool-head">
		<span class="status-dot" title={call.status}></span>
		<span class="tool-icon">
			{#if call.name === 'read_file'}
				<!-- book -->
				<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"/></svg>
			{:else if call.name === 'write_file' || call.name === 'edit_file'}
				<!-- pen -->
				<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
			{:else if call.name === 'list_directory'}
				<!-- magnifier -->
				<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/></svg>
			{:else if call.name === 'terminal'}
				<!-- rounded box with >_ -->
				<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="4" width="20" height="16" rx="3"/><path d="m7 9 3 3-3 3"/><path d="M13 15h4"/></svg>
			{:else if call.name === 'fetch'}
				<!-- globe -->
				<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3a13.5 13.5 0 0 1 0 18a13.5 13.5 0 0 1 0-18"/></svg>
			{:else}
				<!-- wrench (unknown tool) -->
				<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M14.7 6.3a4.5 4.5 0 0 0-6 6L3 18l3 3 5.7-5.7a4.5 4.5 0 0 0 6-6L14 13l-3-3 3.7-3.7z"/></svg>
			{/if}
		</span>
		<span class="tool-title" {title}>{title}</span>

		<span class="tool-actions">
			<!-- Raw view toggle -->
			<button
				type="button"
				class="icon-btn"
				class:active={showRaw}
				title={t('agent.raw')}
				aria-label={t('agent.raw')}
				onclick={() => (showRaw = !showRaw)}
			>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m8 6-6 6 6 6"/><path d="m16 6 6 6-6 6"/></svg>
			</button>
			<!-- Expand toggle (controls the tool detail only, not the raw view) -->
			<button
				type="button"
				class="icon-btn"
				class:active={expanded}
				title={expanded ? 'Collapse' : 'Expand'}
				aria-label={expanded ? 'Collapse' : 'Expand'}
				onclick={() => (expanded = !expanded)}
			>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					{#if expanded}<path d="m6 15 6-6 6 6" />{:else}<path d="m6 9 6 6 6-6" />{/if}
				</svg>
			</button>
			<!-- Tool-specific extra buttons -->
			{#if call.name === 'terminal'}
				{#if call.status === 'running'}
					<button
						type="button"
						class="icon-btn danger"
						title={t('agent.stop_command')}
						aria-label={t('agent.stop_command')}
						onclick={() => agentStopTerminal(call.id)}
					>
						<svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="1.5"/></svg>
					</button>
				{/if}
				<button
					type="button"
					class="icon-btn"
					class:success={copied}
					title={copied ? t('agent.copied') : t('agent.copy_command')}
					aria-label={t('agent.copy_command')}
					onclick={copyCommand}
				>
					{#if copied}
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>
					{:else}
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>
					{/if}
				</button>
			{/if}
		</span>
	</div>

	<!-- ── Detail area (always rendered; 0px-tall when there is nothing to show) ── -->
	<div class="tool-detail" class:empty={!hasDetail}>
		{#if showRaw}
			<!-- Raw view: unaffected by the expand toggle. Before the call ends this
			     is the (streaming) arguments; afterwards input + raw output. -->
			<pre class="raw-block">{prettyJson(call.argsJson)}</pre>
			{#if ended && call.result}
				<pre class="raw-block" class:raw-error={call.result.isError}>{call.result.llmText}</pre>
			{/if}
		{:else if call.name === 'terminal'}
			<!-- Command block is visible in both collapsed and expanded states. -->
			<div class="command-block">{command || prettyJson(call.argsJson)}</div>
			{#if terminalLive}
				<ToolTerminal toolCallId={call.id} {expanded} />
			{/if}
		{:else if expanded}
			{#if call.name === 'write_file' || call.name === 'edit_file'}
				{#if diff}
					<DiffView {diff} />
				{:else if call.result}
					<pre class="text-block">{call.result.llmText}</pre>
				{:else}
					<pre class="raw-block">{prettyJson(call.argsJson)}</pre>
				{/if}
			{:else if call.name === 'read_file'}
				{#if call.result}
					<pre class="text-block">{call.result.llmText}</pre>
				{/if}
			{:else if call.name === 'list_directory' || call.name === 'fetch'}
				{#if call.result}
					<div class="md">{@html renderMarkdown(call.result.llmText)}</div>
				{/if}
			{:else if call.result}
				<pre class="text-block">{call.result.llmText}</pre>
			{/if}
		{/if}
	</div>

	<!-- ── Approval bar (pending_approval / rejected only) ── -->
	{#if call.status === 'pending_approval' || call.status === 'rejected'}
		<div class="approval-bar" class:rejected={call.status === 'rejected'}>
			{#if call.status === 'pending_approval'}
				{#if call.warnings && call.warnings.length > 0}
					<div class="warnings">
						{#each call.warnings as warning, i (i)}
							<div class="warning-line">
								<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>
								<span>{warning}</span>
							</div>
						{/each}
					</div>
				{/if}
				<div class="approval-buttons">
					<button type="button" class="approval-btn accept" onclick={() => agentApproveCall(call.id, true)}>
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>
						{t('agent.accept')}
					</button>
					<button type="button" class="approval-btn reject" onclick={() => agentApproveCall(call.id, false)}>
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
						{t('agent.reject')}
					</button>
				</div>
			{:else}
				<div class="rejected-label">
					<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18M6 6l12 12"/></svg>
					{t('agent.rejected')}
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	.tool-card {
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		background: var(--color-bg-secondary);
		overflow: hidden;
		font-size: 0.75rem;
	}

	.tool-card.status-failed,
	.tool-card.status-rejected {
		border-color: color-mix(in srgb, var(--color-danger) 40%, transparent);
	}

	.tool-card.status-pending_approval {
		border-color: color-mix(in srgb, var(--color-warning) 45%, transparent);
	}

	.tool-head {
		display: flex;
		align-items: center;
		gap: 7px;
		padding: 6px 8px;
		min-height: 30px;
	}

	.status-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		flex-shrink: 0;
		background: var(--color-text-secondary);
	}

	.status-streaming .status-dot,
	.status-running .status-dot {
		background: var(--color-accent);
		animation: pulse 1.2s ease-in-out infinite;
	}

	.status-pending_approval .status-dot {
		background: var(--color-warning);
		animation: pulse 1.2s ease-in-out infinite;
	}

	.status-success .status-dot {
		background: var(--color-success);
	}

	.status-failed .status-dot,
	.status-rejected .status-dot {
		background: var(--color-danger);
	}

	.status-cancelled .status-dot {
		background: var(--color-text-secondary);
	}

	@keyframes pulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.35;
		}
	}

	.tool-icon {
		display: flex;
		align-items: center;
		color: var(--color-text-secondary);
		flex-shrink: 0;
	}

	.tool-title {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-family: var(--font-mono);
		font-size: 0.72rem;
		color: var(--color-text-primary);
	}

	.tool-actions {
		display: flex;
		align-items: center;
		gap: 2px;
		flex-shrink: 0;
	}

	.icon-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		border: none;
		border-radius: 5px;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
		transition:
			background-color 150ms ease,
			color 150ms ease;
	}

	.icon-btn:hover {
		background: rgba(255, 255, 255, 0.08);
		color: var(--color-text-primary);
	}

	.icon-btn.active {
		color: var(--color-accent);
	}

	.icon-btn.danger {
		color: var(--color-danger);
	}

	.icon-btn.success {
		color: var(--color-success);
	}

	.tool-detail {
		border-top: 1px solid var(--color-border);
	}

	.tool-detail.empty {
		display: none;
	}

	.command-block {
		padding: 7px 10px;
		background: var(--color-bg-secondary);
		color: var(--color-text-primary);
		font-family: var(--font-mono);
		font-size: 0.72rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-all;
		user-select: text;
	}

	.raw-block,
	.text-block {
		margin: 0;
		padding: 8px 10px;
		font-family: var(--font-mono);
		font-size: 0.72rem;
		line-height: 1.5;
		color: var(--color-text-secondary);
		white-space: pre-wrap;
		word-break: break-all;
		max-height: 320px;
		overflow-y: auto;
		user-select: text;
	}

	.raw-block + .raw-block {
		border-top: 1px solid var(--color-border);
	}

	.raw-block.raw-error,
	.text-block {
		color: var(--color-text-primary);
	}

	.raw-block.raw-error {
		color: var(--color-danger);
	}

	.md {
		padding: 8px 10px;
		font-size: 0.75rem;
		line-height: 1.55;
		color: var(--color-text-primary);
		max-height: 320px;
		overflow-y: auto;
		user-select: text;
	}

	.md :global(p) {
		margin: 0 0 6px;
	}

	/* Preflight resets list-style to none; restore markers for markdown lists. */
	.md :global(ul),
	.md :global(ol) {
		margin: 4px 0 6px;
		padding-left: 18px;
	}

	.md :global(ul) {
		list-style: disc;
	}

	.md :global(ol) {
		list-style: decimal;
	}

	.md :global(pre) {
		margin: 6px 0;
		padding: 8px;
		background: var(--color-bg-primary);
		border-radius: 6px;
		overflow-x: auto;
	}

	.md :global(code) {
		font-family: var(--font-mono);
		font-size: 0.7rem;
	}

	.approval-bar {
		border-top: 1px solid var(--color-border);
		padding: 8px;
	}

	.approval-bar.rejected {
		background: color-mix(in srgb, var(--color-danger) 10%, transparent);
	}

	.warnings {
		display: flex;
		flex-direction: column;
		gap: 3px;
		margin-bottom: 8px;
		padding: 6px 8px;
		border-radius: 6px;
		background: color-mix(in srgb, var(--color-warning) 12%, transparent);
		color: var(--color-warning);
		font-size: 0.7rem;
	}

	.warning-line {
		display: flex;
		align-items: flex-start;
		gap: 6px;
	}

	.warning-line svg {
		flex-shrink: 0;
		margin-top: 2px;
	}

	.approval-buttons {
		display: flex;
		justify-content: flex-start;
		gap: 8px;
	}

	.approval-btn {
		display: flex;
		align-items: center;
		gap: 5px;
		padding: 5px 12px;
		border: 1px solid var(--color-border);
		border-radius: 6px;
		background: transparent;
		font-size: 0.72rem;
		font-weight: 500;
		cursor: pointer;
		transition:
			background-color 150ms ease,
			border-color 150ms ease;
	}

	.approval-btn.accept {
		color: var(--color-success);
	}

	.approval-btn.accept:hover {
		background: color-mix(in srgb, var(--color-success) 14%, transparent);
		border-color: var(--color-success);
	}

	.approval-btn.reject {
		color: var(--color-danger);
	}

	.approval-btn.reject:hover {
		background: color-mix(in srgb, var(--color-danger) 14%, transparent);
		border-color: var(--color-danger);
	}

	.rejected-label {
		display: flex;
		align-items: center;
		gap: 6px;
		color: var(--color-danger);
		font-size: 0.72rem;
		font-weight: 500;
	}
</style>
