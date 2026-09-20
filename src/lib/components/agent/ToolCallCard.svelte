<script module lang="ts">
	import type { ApprovalWarning, DiffPreview, DiffPreviewLine, ToolCallView } from '$lib/ipc/agent';

	export interface Props {
		call: ToolCallView;
	}
</script>

<script lang="ts">
	import {
		agentApproveCall,
		agentStopTerminal,
		getApprovalPayload,
		getPatchedArgs,
		getToolPreview
	} from '$lib/state/agent.svelte';
	import { untrack } from 'svelte';
	import { t } from '$lib/state/i18n.svelte';
	import PatchView from './PatchView.svelte';
	import ToolTerminal from './ToolTerminal.svelte';
	import { renderMarkdown } from '$lib/utils/markdown';
	import { argString, copyText, extractDiff, prettyJson, safeParse } from '$lib/utils/agent';
	import { parseInlineCode } from '$lib/utils/formatters';

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

	// Patched args (valid JSON while streaming) win over the raw accumulation,
	// which may be unterminated mid-stream.
	let argsJson = $derived(getPatchedArgs(call.id) ?? call.argsJson);
	let args = $derived(safeParse(argsJson));
	let path = $derived(argString(args, ['path', 'file_path', 'filePath', 'target_file']));
	let command = $derived(argString(args, ['command', 'cmd']));
	let url = $derived(argString(args, ['url']));

	let ended = $derived(
		call.status === 'success' ||
			call.status === 'failed' ||
			call.status === 'rejected' ||
			call.status === 'cancelled'
	);

	/** Map a structured backend warning to its localized text. */
	function warningText(warning: ApprovalWarning): string {
		switch (warning.kind) {
			case 'sensitive_pattern':
				return t('agent.warn_sensitive_pattern', { pattern: warning.pattern });
			case 'dangerous_keywords':
				return t('agent.warn_dangerous_keywords', {
					keywords: warning.keywords.map((k) => `\`${k}\``).join(', ')
				});
		}
	}
	/** Terminal view stays hidden until approved + connected (design 01 §2.3). */
	let terminalLive = $derived(
		call.status !== 'streaming' && call.status !== 'pending_approval' && call.status !== 'rejected'
	);

	/** Terminal calls replay from the live buffer while this session has one;
	 *  after an app restart or buffer eviction the persisted text projection
	 *  (result ui_payload) restores the view instead. */
	let terminalFallback = $derived.by((): string | null => {
		if (call.name !== 'terminal' || !ended) return null;
		const payload = call.result?.uiPayload as { output?: unknown } | undefined;
		return typeof payload?.output === 'string' && payload.output ? payload.output : null;
	});

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
				return `\`${path || call.name}\``;
			default:
				return `\`${call.name}\``;
		}
	});

	let titleSegments = $derived(parseInlineCode(title));
	/** Plain-text title (backticks resolved) for the tooltip. */
	let titleTooltip = $derived(titleSegments.map((s) => s.text).join(''));

	let isFileTool = $derived(call.name === 'write_file' || call.name === 'edit_file');

	// Detail priority: approval/result structured diff > live preview > prewarm.
	let finalDiff = $derived(isFileTool ? extractDiff(call, getApprovalPayload(call.id)) : null);
	let livePreview = $derived(isFileTool ? getToolPreview(call.id) : null);
	let activeDiff = $derived(finalDiff ?? livePreview);

	/** Streaming old-text (edit_file) or content (write_file) rendered as plain
	 *  patch rows while no structured preview exists yet. Feeding the same
	 *  PatchView as the real preview keeps the two views geometrically
	 *  identical; the swap away from these rows is one-way. */
	let warmPreview = $derived.by((): DiffPreview | null => {
		if (!isFileTool || call.status !== 'streaming' || activeDiff) return null;
		if (!args || typeof args !== 'object') return null;
		let text = '';
		let kind: 'add' | 'del' = 'del';
		if (call.name === 'write_file') {
			text = argString(args, ['content', 'text', 'new_text']);
			kind = 'add';
		} else {
			text = argString(args, ['old_text', 'oldText', 'search']);
			const edits = (args as Record<string, unknown>).edits;
			if (!text && Array.isArray(edits)) {
				for (const edit of edits) {
					if (!edit || typeof edit !== 'object') continue;
					const e = edit as Record<string, unknown>;
					if (typeof e.old_text === 'string' && e.old_text) {
						text = e.old_text;
						break;
					}
				}
			}
		}
		if (!text) return null;
		const lines = text
			.replace(/\r\n?/g, '\n')
			.split('\n')
			.map((line, i): DiffPreviewLine =>
				kind === 'add' ? { kind, text: line, newNo: i + 1 } : { kind, text: line, oldNo: i + 1 }
			);
		return { path, isNewFile: false, hunks: [{ oldStart: 1, newStart: 1, lines }] };
	});

	/** One PatchView instance serves the warm rows and the real preview, so the
	 *  swap only replaces content. */
	let shownDiff = $derived(activeDiff ?? warmPreview);

	/** While the diff is still arriving (streaming) or awaiting the user's
	 *  approval, it stays visible even when the card is collapsed. Once the
	 *  call settles, the collapse toggle wins. */
	let pinnedDiff = $derived(
		(call.status === 'streaming' || call.status === 'pending_approval') && shownDiff != null
	);

	/** Whether the detail area renders any content (drives the 0px collapsed look). */
	let hasDetail = $derived.by(() => {
		if (showRaw) return true;
		if (call.name === 'terminal') return true; // command block always visible
		// Pinned write/edit diffs stay visible while streaming / awaiting approval.
		if (isFileTool) return expanded || pinnedDiff;
		return expanded && call.result != null;
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
		<span class="tool-title" title={titleTooltip}>{#each titleSegments as segment}{#if segment.code}<code>{segment.text}</code>{:else}{segment.text}{/if}{/each}</span>

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
				{#if call.status === 'running'}
					<button
						type="button"
						class="icon-btn danger"
						title={t('agent.stop_command')}
						aria-label={t('agent.stop_command')}
						onclick={() => agentStopTerminal(call.id)}
					>
						<svg width="11" height="11" viewBox="0 0 24 24" fill="currentColor"><rect x="4" y="4" width="16" height="16" rx="1.5"/></svg>
					</button>
				{/if}
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
			<div class="command-block">{command || prettyJson(call.argsJson)}{#if call.status === 'streaming'}<span class="cursor"></span>{/if}</div>
			{#if terminalLive}
				<ToolTerminal toolCallId={call.id} {expanded} fallbackText={terminalFallback} />
			{/if}
		{:else if isFileTool && (expanded || pinnedDiff)}
			{#if shownDiff}
				<PatchView diff={shownDiff} />
			{:else if call.result}
				<pre class="text-block">{call.result.llmText}</pre>
			{:else}
				<pre class="raw-block">{prettyJson(call.argsJson)}</pre>
			{/if}
		{:else if expanded}
			{#if call.name === 'read_file'}
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
								<span>{#each parseInlineCode(warningText(warning)) as segment}{#if segment.code}<code>{segment.text}</code>{:else}{segment.text}{/if}{/each}</span>
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
		/* clip, unlike hidden, creates no scroll container — the approval
		   bar's sticky positioning resolves against the chat scroller. */
		overflow: clip;
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
		font-size: 0.72rem;
		color: var(--color-text-primary);
	}

	.tool-title code {
		font-family: var(--font-content-mono);
		font-size: 0.7rem;
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
		font-family: var(--font-content-mono);
		font-size: 0.72rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-all;
		user-select: text;
	}

	.cursor {
		display: inline-block;
		width: 6px;
		height: 0.85em;
		margin-left: 1px;
		vertical-align: text-bottom;
		background: var(--color-text-primary);
		animation: blink 1s step-end infinite;
	}

	@keyframes blink {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0;
		}
	}

	.raw-block,
	.text-block {
		margin: 0;
		padding: 8px 10px;
		font-family: var(--font-content-mono);
		font-size: 0.72rem;
		line-height: 1.5;
		color: var(--color-text-secondary);
		white-space: pre-wrap;
		word-break: break-all;
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
		font-family: var(--font-content-mono);
		font-size: 0.7rem;
	}

	/* Sticky so the actions stay reachable while reviewing a long diff: the
	   bar pins to the bottom of the chat viewport until the card's own bottom
	   scrolls in. Opaque background — diff lines scroll underneath it. */
	.approval-bar {
		position: sticky;
		bottom: -10px;
		border-top: 1px solid var(--color-border);
		padding: 8px;
		background: var(--color-bg-secondary);
	}

	.approval-bar.rejected {
		background: color-mix(in srgb, var(--color-danger) 10%, var(--color-bg-secondary));
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

	.warning-line code {
		font-family: var(--font-content-mono);
		font-size: 0.68rem;
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
