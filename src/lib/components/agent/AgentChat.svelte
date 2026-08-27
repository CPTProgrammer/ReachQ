<script module lang="ts">
	export interface Props {
		identity: string;
		threadId: string | null;
	}
</script>

<script lang="ts">
	import type { ContentBlock, MessageMetadata, PathMessage, ToolCallView } from '$lib/ipc/agent';
	import {
		agentDequeueMessage,
		agentEditAndFork,
		agentSendNow,
		agentSwitchBranch,
		getThreadRuntime,
		type ThreadRuntime
	} from '$lib/state/agent.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import ToolCallCard from './ToolCallCard.svelte';
	import { renderMarkdown } from './markdown';
	import { appendDraft } from './composer-draft.svelte';
	import { formatDuration, formatTokens, messageText, resolveSendOpts } from './utils';

	let { identity, threadId }: Props = $props();

	let runtime = $derived(threadId ? getThreadRuntime(threadId) : null);

	// ── Scroll ────────────────────────────────────────────────────────────────

	let scrollEl: HTMLDivElement | undefined = $state();
	let nearBottom = $state(true);

	function onScroll(): void {
		const el = scrollEl;
		if (!el) return;
		nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
	}

	/** Reactive signature of everything that can grow the message list. */
	function contentSig(rt: ThreadRuntime): string {
		let s = `${rt.messages.length}:`;
		for (const m of rt.messages) {
			s += m.id;
			for (const b of m.content) {
				if (b.type === 'text' || b.type === 'thinking') s += `.${b.text.length}`;
				else {
					const v = rt.toolCalls[b.id];
					s += `.${b.status}${v ? `${v.argsJson.length}${v.status}` : ''}`;
				}
			}
		}
		return s;
	}

	$effect(() => {
		const rt = runtime;
		const el = scrollEl;
		if (!rt || !el) return;
		contentSig(rt); // subscribe
		if (nearBottom) el.scrollTop = el.scrollHeight;
	});

	// ── User message inline editing / fork (design 01 §2.4) ──────────────────

	let editing: { id: string; text: string } | null = $state(null);
	let editEl: HTMLTextAreaElement | undefined = $state();

	// Jump to the bottom on thread switch.
	let lastThreadId = $state<string | null>(null);
	$effect(() => {
		if (threadId === lastThreadId) return;
		lastThreadId = threadId;
		nearBottom = true;
		editing = null;
		requestAnimationFrame(() => {
			if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
		});
	});

	let editIndex = $derived.by(() => {
		if (!editing || !runtime) return -1;
		return runtime.messages.findIndex((m) => m.id === editing?.id);
	});

	function startEdit(msg: PathMessage): void {
		if (msg.role !== 'user') return;
		editing = { id: msg.id, text: messageText(msg) };
	}

	function cancelEdit(): void {
		editing = null;
	}

	async function submitEdit(): Promise<void> {
		if (!editing || !threadId) return;
		const text = editing.text.trim();
		if (!text) return;
		const opts = resolveSendOpts(identity, threadId);
		if (!opts) return;
		const messageId = editing.id;
		editing = null;
		await agentEditAndFork(identity, threadId, messageId, text, opts);
	}

	function editKeydown(e: KeyboardEvent): void {
		if (e.key === 'Escape') {
			e.preventDefault();
			cancelEdit();
			return;
		}
		if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey && !e.isComposing) {
			e.preventDefault();
			void submitEdit();
		}
	}

	$effect(() => {
		if (editing && editEl) {
			editEl.focus();
			editEl.style.height = 'auto';
			editEl.style.height = `${Math.min(editEl.scrollHeight, 240)}px`;
		}
	});

	// ── Thinking blocks (auto-expand while streaming; manual toggle wins) ─────

	let thinkingOverrides = $state<Record<string, boolean>>({});

	function thinkingExpanded(msg: PathMessage, index: number): boolean {
		const key = `${msg.id}:${index}`;
		const override = thinkingOverrides[key];
		if (override !== undefined) return override;
		return runtime?.streamingMessageId === msg.id && index === msg.content.length - 1;
	}

	function toggleThinking(msg: PathMessage, index: number): void {
		const key = `${msg.id}:${index}`;
		thinkingOverrides[key] = !thinkingExpanded(msg, index);
	}

	// ── Tool call views (live view preferred; snapshot fallback) ─────────────

	function toolCallView(msg: PathMessage, block: Extract<ContentBlock, { type: 'tool_call' }>): ToolCallView {
		const live = runtime?.toolCalls[block.id];
		if (live) return live;
		return {
			id: block.id,
			messageId: msg.id,
			name: block.name,
			argsJson: JSON.stringify(block.args ?? {}, null, 2),
			status: block.status,
			result: block.result
		};
	}

	// ── Message metadata hover card ───────────────────────────────────────────

	let metaHoverId = $state<string | null>(null);

	function hasMeta(meta: MessageMetadata): boolean {
		return !!(
			meta.providerInstance ||
			meta.model ||
			meta.usage ||
			meta.durationMs != null ||
			meta.toolCallCount != null
		);
	}

	// ── Branch navigation ─────────────────────────────────────────────────────

	async function switchBranch(msg: PathMessage, direction: 'prev' | 'next'): Promise<void> {
		if (!threadId) return;
		await agentSwitchBranch(threadId, msg.id, direction);
	}

	// ── Queued message bar (design 01 §3.5) ──────────────────────────────────

	function queuedEdit(): void {
		const rt = runtime;
		if (!rt?.queued || !threadId) return;
		appendDraft(threadId, rt.queued.text);
		rt.queued = null;
		void agentDequeueMessage(threadId);
	}

	function queuedDelete(): void {
		const rt = runtime;
		if (!rt || !threadId) return;
		rt.queued = null;
		void agentDequeueMessage(threadId);
	}

	async function queuedSendNow(): Promise<void> {
		const rt = runtime;
		if (!rt?.queued || !threadId) return;
		const text = rt.queued.text;
		const opts = resolveSendOpts(identity, threadId);
		// Atomic backend-side: supersede queue + cancel + wait + fresh run.
		if (opts) await agentSendNow(identity, threadId, text, opts);
	}

	// ── Markdown code-block copy (event delegation) ───────────────────────────

	function onChatClick(e: MouseEvent): void {
		const target = (e.target as HTMLElement).closest('.md-copy') as HTMLElement | null;
		if (!target) return;
		const code = decodeURIComponent(target.dataset.code ?? '');
		navigator.clipboard.writeText(code).then(() => {
			target.textContent = t('agent.copied');
			setTimeout(() => (target.textContent = t('agent.copy')), 1500);
		});
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="chat-area">
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="messages" bind:this={scrollEl} onscroll={onScroll} onclick={onChatClick}>
		{#if !runtime || runtime.messages.length === 0}
			<div class="empty-state">
				<p>{t('agent.empty_thread')}</p>
			</div>
		{:else}
			{#each runtime.messages as msg, i (msg.id)}
				<div
					class="msg-row"
					class:user-row={msg.role === 'user'}
					class:dimmed={editIndex >= 0 && i > editIndex}
				>
					{#if msg.role === 'user'}
						{#if editing?.id === msg.id}
							<div class="user-editing">
								<textarea
									bind:this={editEl}
									bind:value={editing.text}
									onkeydown={editKeydown}
									rows="1"
								></textarea>
								<button
									type="button"
									class="edit-send"
									title={t('agent.send')}
									aria-label={t('agent.send')}
									onclick={() => void submitEdit()}
								>
									<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13"/><polygon points="22 2 15 22 11 13 2 9 22 2"/></svg>
								</button>
							</div>
						{:else}
							<button
								type="button"
								class="bubble user"
								title={t('agent.edit_message_hint')}
								onclick={() => startEdit(msg)}
							>{messageText(msg)}</button>
						{/if}
					{:else}
						<div class="assistant-block">
							{#each msg.content as block, bi (bi)}
								{#if block.type === 'text'}
									{#if block.text}
										<div class="md">{@html renderMarkdown(block.text)}</div>
									{/if}
								{:else if block.type === 'thinking'}
									<div class="thinking">
										<button
											type="button"
											class="thinking-head"
											onclick={() => toggleThinking(msg, bi)}
										>
											<svg
												class="thinking-chevron"
												class:open={thinkingExpanded(msg, bi)}
												width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
											><path d="m9 6 6 6-6 6"/></svg>
											{t('agent.thinking')}
										</button>
										{#if thinkingExpanded(msg, bi)}
											<div class="thinking-body">{block.text}</div>
										{/if}
									</div>
								{:else if block.type === 'tool_call'}
									<ToolCallCard call={toolCallView(msg, block)} />
								{/if}
							{/each}
							{#if msg.content.length === 0}
								<span class="typing-indicator"><span></span><span></span><span></span></span>
							{/if}
							{#if msg.metadata && hasMeta(msg.metadata)}
								{@const meta = msg.metadata}
								<div class="meta-anchor">
									<button
										type="button"
										class="meta-icon"
										aria-label="metadata"
										onmouseenter={() => (metaHoverId = msg.id)}
										onmouseleave={() => (metaHoverId = null)}
									>
										<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 16v-5"/><path d="M12 8h.01"/></svg>
									</button>
									{#if metaHoverId === msg.id}
										<div class="meta-card">
											{#if meta.providerInstance}
												<div class="meta-row"><span>{t('agent.meta_provider')}</span><span>{meta.providerInstance}</span></div>
											{/if}
											{#if meta.model}
												<div class="meta-row"><span>{t('agent.meta_model')}</span><span>{meta.model}</span></div>
											{/if}
											{#if meta.usage}
												<div class="meta-row"><span>{t('agent.meta_tokens')}</span><span>{formatTokens(meta.usage.promptTokens)} / {formatTokens(meta.usage.completionTokens)}</span></div>
												{#if meta.usage.cachedTokens != null}
													<div class="meta-row"><span>{t('agent.meta_cached')}</span><span>{formatTokens(meta.usage.cachedTokens)}</span></div>
												{/if}
											{/if}
											{#if meta.durationMs != null}
												<div class="meta-row"><span>{t('agent.meta_duration')}</span><span>{formatDuration(meta.durationMs)}</span></div>
											{/if}
											{#if meta.toolCallCount != null}
												<div class="meta-row"><span>{t('agent.meta_tool_calls')}</span><span>{meta.toolCallCount}</span></div>
											{/if}
										</div>
									{/if}
								</div>
							{/if}
						</div>
					{/if}
					{#if msg.branch.count > 1}
						<div class="branch-nav" class:user-branch={msg.role === 'user'} title={t('agent.branch_nav_tooltip')}>
							<button type="button" aria-label="prev" onclick={() => switchBranch(msg, 'prev')}>
								<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
							</button>
							<span>{msg.branch.index} / {msg.branch.count}</span>
							<button type="button" aria-label="next" onclick={() => switchBranch(msg, 'next')}>
								<svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>
							</button>
						</div>
					{/if}
				</div>
			{/each}
		{/if}
	</div>

	{#if runtime?.error}
		<div class="error-bar">{t('agent.error')}: {runtime.error}</div>
	{/if}

	{#if runtime?.queued}
		<div class="queued-bar">
			<svg class="queued-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 2h12"/><path d="M6 22h12"/><path d="M8 2v4l4 4 4-4V2"/><path d="M8 22v-4l4-4 4 4v4"/></svg>
			<span class="queued-text">{t('agent.queued', { text: runtime.queued.text })}</span>
			<button type="button" class="icon-btn" title={t('agent.edit_queued')} aria-label={t('agent.edit_queued')} onclick={queuedEdit}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
			</button>
			<button type="button" class="icon-btn" title={t('agent.delete_queued')} aria-label={t('agent.delete_queued')} onclick={queuedDelete}>
				<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
			</button>
			<button type="button" class="send-now" onclick={() => void queuedSendNow()}>{t('agent.send_now')}</button>
		</div>
	{/if}
</div>

<style>
	.chat-area {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
	}

	.messages {
		flex: 1;
		overflow-y: auto;
		padding: 12px 10px;
		display: flex;
		flex-direction: column;
		gap: 10px;
		min-height: 0;
	}

	.empty-state {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--color-text-secondary);
		font-size: 0.75rem;
		text-align: center;
		padding: 24px;
	}

	.empty-state p {
		margin: 0;
	}

	.msg-row {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		transition: opacity 150ms ease;
	}

	.msg-row.user-row {
		align-items: flex-end;
	}

	.msg-row.dimmed {
		opacity: 0.4;
	}

	/* ── user bubble ── */

	.bubble.user {
		max-width: 88%;
		padding: 7px 11px;
		border: 1px solid transparent;
		border-radius: var(--radius-card);
		background: var(--color-bg-secondary);
		color: var(--color-text-primary);
		font-family: var(--font-sans);
		font-size: 0.8rem;
		line-height: 1.45;
		text-align: left;
		white-space: pre-wrap;
		word-break: break-word;
		cursor: pointer;
		transition: border-color 150ms ease;
	}

	.bubble.user:hover {
		border-color: var(--color-accent);
	}

	.user-editing {
		position: relative;
		width: 100%;
	}

	.user-editing textarea {
		width: 100%;
		box-sizing: border-box;
		padding: 7px 11px;
		border: 1px solid var(--color-accent);
		border-radius: var(--radius-card);
		background: var(--color-bg-secondary);
		color: var(--color-text-primary);
		font-family: var(--font-sans);
		font-size: 0.8rem;
		line-height: 1.45;
		resize: none;
		outline: none;
	}

	.edit-send {
		position: absolute;
		right: 0;
		bottom: -13px;
		display: flex;
		align-items: center;
		justify-content: center;
		width: 26px;
		height: 26px;
		border: 1px solid var(--color-border);
		border-radius: 50%;
		background: var(--color-accent);
		color: #fff;
		cursor: pointer;
		box-shadow: var(--shadow-elevated);
		z-index: 5;
	}

	/* ── assistant blocks ── */

	.assistant-block {
		position: relative;
		width: 100%;
		padding-left: 20px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.md {
		font-size: 0.8rem;
		line-height: 1.55;
		color: var(--color-text-primary);
		word-break: break-word;
		user-select: text;
	}

	.md :global(p) {
		margin: 0 0 8px;
	}

	.md :global(p:last-child) {
		margin-bottom: 0;
	}

	.md :global(h1),
	.md :global(h2),
	.md :global(h3),
	.md :global(h4) {
		margin: 10px 0 6px;
		font-size: 0.9rem;
		line-height: 1.3;
	}

	.md :global(ul),
	.md :global(ol) {
		margin: 4px 0 8px;
		padding-left: 20px;
	}

	.md :global(blockquote) {
		margin: 4px 0 8px;
		padding: 2px 10px;
		border-left: 3px solid var(--color-border);
		color: var(--color-text-secondary);
	}

	.md :global(hr) {
		border: none;
		border-top: 1px solid var(--color-border);
		margin: 8px 0;
	}

	.md :global(a) {
		color: var(--color-accent);
	}

	.md :global(.md-inline) {
		padding: 1px 4px;
		border-radius: 4px;
		background: var(--color-bg-secondary);
		font-family: var(--font-mono);
		font-size: 0.72rem;
	}

	.md :global(.md-code) {
		margin: 6px 0;
		border: 1px solid var(--color-border);
		border-radius: 8px;
		overflow: hidden;
		background: var(--color-bg-primary);
	}

	.md :global(.md-code-head) {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 3px 8px;
		font-size: 0.65rem;
		color: var(--color-text-secondary);
		border-bottom: 1px solid var(--color-border);
	}

	.md :global(.md-copy) {
		border: none;
		background: transparent;
		color: var(--color-accent);
		cursor: pointer;
		font-size: 0.65rem;
		font-family: var(--font-sans);
		padding: 1px 5px;
		border-radius: 4px;
	}

	.md :global(.md-copy:hover) {
		background: rgba(255, 255, 255, 0.06);
	}

	.md :global(pre) {
		margin: 0;
		padding: 8px 10px;
		overflow-x: auto;
	}

	.md :global(pre code) {
		font-family: var(--font-mono);
		font-size: 0.72rem;
		line-height: 1.5;
	}

	/* ── thinking ── */

	.thinking {
		border-left: 2px solid var(--color-border);
		padding-left: 8px;
	}

	.thinking-head {
		display: flex;
		align-items: center;
		gap: 4px;
		border: none;
		background: transparent;
		padding: 0;
		font-size: 0.7rem;
		font-weight: 500;
		color: var(--color-text-secondary);
		cursor: pointer;
	}

	.thinking-chevron {
		transition: transform 150ms ease;
	}

	.thinking-chevron.open {
		transform: rotate(90deg);
	}

	.thinking-body {
		margin-top: 4px;
		font-size: 0.72rem;
		line-height: 1.5;
		color: var(--color-text-secondary);
		white-space: pre-wrap;
		word-break: break-word;
		max-height: 240px;
		overflow-y: auto;
		user-select: text;
	}

	/* ── typing indicator ── */

	.typing-indicator {
		display: inline-flex;
		gap: 3px;
		padding: 4px 0;
	}

	.typing-indicator span {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: var(--color-text-secondary);
		animation: typingBounce 1s infinite;
	}

	.typing-indicator span:nth-child(2) {
		animation-delay: 0.15s;
	}

	.typing-indicator span:nth-child(3) {
		animation-delay: 0.3s;
	}

	@keyframes typingBounce {
		0%,
		60%,
		100% {
			transform: translateY(0);
		}
		30% {
			transform: translateY(-4px);
		}
	}

	/* ── metadata ── */

	.meta-anchor {
		position: absolute;
		left: 0;
		bottom: 0;
		z-index: 6;
	}

	.meta-icon {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 16px;
		height: 16px;
		border: none;
		border-radius: 50%;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: default;
		opacity: 0.7;
	}

	.meta-icon:hover {
		opacity: 1;
		color: var(--color-text-primary);
	}

	.meta-card {
		position: absolute;
		left: 0;
		bottom: calc(100% + 6px);
		min-width: 180px;
		padding: 7px 9px;
		background: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: 8px;
		box-shadow: var(--shadow-elevated);
		display: flex;
		flex-direction: column;
		gap: 3px;
	}

	.meta-row {
		display: flex;
		justify-content: space-between;
		gap: 12px;
		font-size: 0.68rem;
		white-space: nowrap;
	}

	.meta-row span:first-child {
		color: var(--color-text-secondary);
	}

	.meta-row span:last-child {
		color: var(--color-text-primary);
		font-family: var(--font-mono);
	}

	/* ── branch nav ── */

	.branch-nav {
		display: flex;
		align-items: center;
		gap: 2px;
		margin-top: 3px;
		font-size: 0.65rem;
		color: var(--color-text-secondary);
	}

	.branch-nav.user-branch {
		align-self: flex-end;
	}

	.branch-nav button {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 18px;
		height: 18px;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--color-text-secondary);
		cursor: pointer;
	}

	.branch-nav button:hover {
		background: rgba(255, 255, 255, 0.08);
		color: var(--color-text-primary);
	}

	/* ── error + queued bars ── */

	.error-bar {
		flex-shrink: 0;
		padding: 6px 12px;
		border-top: 1px solid var(--color-border);
		background: color-mix(in srgb, var(--color-danger) 10%, transparent);
		color: var(--color-danger);
		font-size: 0.72rem;
		word-break: break-word;
	}

	.queued-bar {
		flex-shrink: 0;
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 5px 10px;
		border-top: 1px solid var(--color-border);
		background: var(--color-bg-secondary);
	}

	.queued-icon {
		flex-shrink: 0;
		color: var(--color-warning);
	}

	.queued-text {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-size: 0.72rem;
		color: var(--color-text-secondary);
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
		flex-shrink: 0;
	}

	.icon-btn:hover {
		background: rgba(255, 255, 255, 0.08);
		color: var(--color-text-primary);
	}

	.send-now {
		flex-shrink: 0;
		border: none;
		background: transparent;
		color: var(--color-accent);
		font-size: 0.72rem;
		font-weight: 500;
		cursor: pointer;
		padding: 3px 6px;
		border-radius: 5px;
	}

	.send-now:hover {
		background: rgba(255, 255, 255, 0.08);
	}
</style>
