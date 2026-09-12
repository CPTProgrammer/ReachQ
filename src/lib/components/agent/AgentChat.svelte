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
		getThreadRuntime
	} from '$lib/state/agent.svelte';
	import { getFollowing, setFollowing } from './chat-follow.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import ToolCallCard from './ToolCallCard.svelte';
	import { renderMarkdown } from './markdown';
	import { appendDraft } from './composer-draft.svelte';
	import { formatDuration, formatSpeed, formatTokens, messageText, resolveSendOpts } from './utils';
	import { autogrowTextarea } from '$lib/utils/autogrow';

	let { identity, threadId }: Props = $props();

	let runtime = $derived(threadId ? getThreadRuntime(threadId) : null);

	// ── Scroll (stick-to-bottom) ─────────────────────────────────────────────

	let scrollEl: HTMLDivElement | undefined = $state();
	let following = $derived(getFollowing());

	/** Re-entry hysteresis: scrolling *down* into this zone re-engages follow. */
	const REENTER_PX = 4;

	// Snap-to-bottom marks the scrollTop it landed on; the coalesced scroll
	// event triggered by a programmatic write is recognized by value match
	// (consumed once) instead of being classified as a user gesture.
	let lastProgrammaticTop: number | null = null;
	let lastObservedTop = 0;

	function distanceToBottom(el: HTMLDivElement): number {
		return el.scrollHeight - el.scrollTop - el.clientHeight;
	}

	function snapToBottom(): void {
		const el = scrollEl;
		if (!el) return;
		el.scrollTop = el.scrollHeight;
		lastProgrammaticTop = el.scrollTop;
		lastObservedTop = el.scrollTop;
	}

	function jumpToBottom(): void {
		setFollowing(true);
		snapToBottom();
	}

	function onScroll(): void {
		const el = scrollEl;
		if (!el) return;
		const top = el.scrollTop;
		if (lastProgrammaticTop !== null && top === lastProgrammaticTop) {
			lastProgrammaticTop = null;
		} else if (top < lastObservedTop) {
			setFollowing(false); // any upward movement leaves follow instantly
		} else if (top > lastObservedTop && distanceToBottom(el) < REENTER_PX) {
			setFollowing(true); // scrolling back down into the zone re-enters
		}
		lastObservedTop = top;
	}

	function onWheel(e: WheelEvent): void {
		if (e.deltaY < 0) setFollowing(false);
	}

	let lastTouchY = 0;

	function onTouchStart(e: TouchEvent): void {
		lastTouchY = e.touches[0]?.clientY ?? 0;
	}

	function onTouchMove(e: TouchEvent): void {
		const y = e.touches[0]?.clientY ?? lastTouchY;
		if (y > lastTouchY + 2) setFollowing(false); // finger swipes down: content up
		lastTouchY = y;
	}

	// While following, pin the scroller to the bottom every frame. A frame
	// loop (instead of reacting to state changes) absorbs growth from any
	// source — Svelte renders as well as non-reactive DOM such as streaming
	// xterm output — and batches bursty deltas into at most one scroll per
	// frame.
	$effect(() => {
		if (!following || !scrollEl) return;
		const el = scrollEl;
		let raf = 0;
		const pin = () => {
			const max = el.scrollHeight - el.clientHeight;
			if (el.scrollTop !== max) el.scrollTop = max;
			lastProgrammaticTop = el.scrollTop;
			lastObservedTop = el.scrollTop;
			raf = requestAnimationFrame(pin);
		};
		raf = requestAnimationFrame(pin);
		return () => cancelAnimationFrame(raf);
	});

	// ── User message inline editing / fork (design 01 §2.4) ──────────────────

	let editing: { id: string; text: string; original: string } | null = $state(null);
	let editEl: HTMLTextAreaElement | undefined = $state();
	let editBoxEl: HTMLDivElement | undefined = $state();

	// Jump to the bottom on thread switch.
	let lastThreadId = $state<string | null>(null);
	$effect(() => {
		if (threadId === lastThreadId) return;
		lastThreadId = threadId;
		setFollowing(true);
		editing = null;
		requestAnimationFrame(() => {
			snapToBottom();
		});
	});

	let editIndex = $derived.by(() => {
		if (!editing || !runtime) return -1;
		return runtime.messages.findIndex((m) => m.id === editing?.id);
	});

	function startEdit(msg: PathMessage): void {
		if (msg.role !== 'user') return;
		// A modified edit in progress can only be discarded via cancel button / Escape.
		if (editing && editing.text !== editing.original) return;
		const text = messageText(msg);
		editing = { id: msg.id, text, original: text };
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
		setFollowing(true); // resending jumps the chat to the bottom
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
			autogrowTextarea(editEl);
		}
	});

	// Click-outside cancels an unmodified edit (a modified one requires the
	// cancel button or Escape). Used instead of blur so that window focus
	// loss doesn't discard the edit.
	$effect(() => {
		const ed = editing;
		if (!ed) return;
		const handler = (e: MouseEvent) => {
			if (editBoxEl && !editBoxEl.contains(e.target as Node) && ed.text === ed.original) {
				cancelEdit();
			}
		};
		document.addEventListener('mousedown', handler, true);
		return () => document.removeEventListener('mousedown', handler, true);
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

	let metaHover = $state<{ id: string; below: boolean } | null>(null);

	// Flip the card downward when the anchor sits in the upper half of the
	// scroll viewport, so it isn't clipped by the container's top edge.
	function metaEnter(e: MouseEvent, id: string): void {
		const anchor = e.currentTarget as HTMLElement;
		const scroller = scrollEl;
		let below = false;
		if (scroller) {
			const a = anchor.getBoundingClientRect();
			const s = scroller.getBoundingClientRect();
			below = a.top + a.height / 2 < s.top + s.height / 2;
		}
		metaHover = { id, below };
	}

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
		if (opts) {
			setFollowing(true);
			await agentSendNow(identity, threadId, text, opts);
		}
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
	<div
		class="messages"
		bind:this={scrollEl}
		onscroll={onScroll}
		onwheel={onWheel}
		ontouchstart={onTouchStart}
		ontouchmove={onTouchMove}
		onclick={onChatClick}
	>
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
							<div class="user-editing" bind:this={editBoxEl}>
								<textarea
									bind:this={editEl}
									bind:value={editing.text}
									onkeydown={editKeydown}
									oninput={() => autogrowTextarea(editEl)}
									rows="1"
								></textarea>
								<div class="edit-actions">
									<button
										type="button"
										class="edit-btn"
										title={t('common.cancel')}
										aria-label={t('common.cancel')}
										onclick={cancelEdit}
									>
										<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>
									</button>
									<button
										type="button"
										class="edit-btn send"
										title={t('agent.send')}
										aria-label={t('agent.send')}
										disabled={!editing.text.trim()}
										onclick={() => void submitEdit()}
									>
										<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11.5003 12H5.41872M5.24634 12.7972L4.24158 15.7986C3.69128 17.4424 3.41613 18.2643 3.61359 18.7704C3.78506 19.21 4.15335 19.5432 4.6078 19.6701C5.13111 19.8161 5.92151 19.4604 7.50231 18.7491L17.6367 14.1886C19.1797 13.4942 19.9512 13.1471 20.1896 12.6648C20.3968 12.2458 20.3968 11.7541 20.1896 11.3351C19.9512 10.8529 19.1797 10.5057 17.6367 9.81135L7.48483 5.24303C5.90879 4.53382 5.12078 4.17921 4.59799 4.32468C4.14397 4.45101 3.77572 4.78336 3.60365 5.22209C3.40551 5.72728 3.67772 6.54741 4.22215 8.18767L5.24829 11.2793C5.34179 11.561 5.38855 11.7019 5.407 11.8459C5.42338 11.9738 5.42321 12.1032 5.40651 12.231C5.38768 12.375 5.34057 12.5157 5.24634 12.7972Z"/></svg>
									</button>
								</div>
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
								<div
									class="meta-anchor"
									onmouseenter={(e) => metaEnter(e, msg.id)}
									onmouseleave={() => (metaHover = null)}
								>
									<button type="button" class="meta-icon" aria-label="metadata">
										<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 16v-5"/><path d="M12 8h.01"/></svg>
									</button>
									{#if metaHover?.id === msg.id}
										<div class="meta-card" class:below={metaHover?.below ?? false}>
											{#if meta.providerInstance}
												<div class="meta-row"><span>{t('agent.meta_provider')}</span><span>{meta.providerInstance}</span></div>
											{/if}
											{#if meta.model}
												<div class="meta-row"><span>{t('agent.meta_model')}</span><span>{meta.model}</span></div>
											{/if}
											{#if meta.usage}
												<div class="meta-row"><span>{t('agent.meta_tokens')}</span><span>{t('agent.meta_tokens_value', { input: formatTokens(meta.usage.promptTokens), output: formatTokens(meta.usage.completionTokens) })}</span></div>
												{#if meta.usage.cachedTokens != null}
													<div class="meta-row"><span>{t('agent.meta_cached')}</span><span>{formatTokens(meta.usage.cachedTokens)}</span></div>
												{/if}
											{/if}
											{#if meta.ttftMs != null}
												<div class="meta-row"><span>{t('agent.meta_ttft')}</span><span>{formatDuration(meta.ttftMs)}</span></div>
											{/if}
											{#if meta.durationMs != null}
												<div class="meta-row"><span>{t('agent.meta_duration')}</span><span>{formatDuration(meta.durationMs)}</span></div>
											{/if}
											{#if meta.usage && meta.durationMs != null && meta.durationMs > 0}
												<div class="meta-row"><span>{t('agent.meta_speed')}</span><span>{formatSpeed(meta.usage.completionTokens, meta.durationMs, meta.ttftMs)} tok/s</span></div>
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
			<svg class="queued-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3h12"/><path d="M6 21h12"/><path d="M8 3v5l4 4 4-4V3"/><path d="M8 21v-5l4-4 4 4v5"/></svg>
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

	{#if runtime && runtime.messages.length > 0 && !following}
		<button
			type="button"
			class="to-bottom"
			title={t('agent.scroll_to_bottom')}
			aria-label={t('agent.scroll_to_bottom')}
			onclick={jumpToBottom}
		>
			<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>
		</button>
	{/if}
</div>

<style>
	.chat-area {
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		min-height: 0;
		position: relative;
	}

	.messages {
		flex: 1;
		padding: 10px;
		display: flex;
		flex-direction: column;
		gap: 10px;
		max-width: var(--chat-max-width);
		width: 100%;
		overflow-y: auto;
		/* Stick-to-bottom is driven by the pin loop; scroll anchoring would
		   fight it and emit stray scroll events (see chat-follow). */
		overflow-anchor: none;
	}

	.to-bottom {
		position: absolute;
		right: 14px;
		bottom: 14px;
		z-index: 5;
		display: flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		border-radius: 6px;
		border: 1px solid var(--color-border);
		background: var(--color-bg-elevated);
		color: var(--color-text-secondary);
		cursor: pointer;
		box-shadow: var(--shadow-subtle);
	}

	.to-bottom:hover {
		color: var(--color-text-primary);
		border-color: var(--color-accent);
	}

	/* Keep the pill clear of the error / queued bars sitting below the list. */
	.chat-area:has(.error-bar) .to-bottom,
	.chat-area:has(.queued-bar) .to-bottom {
		bottom: 52px;
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
		pointer-events: none;
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
		width: 88%;
	}

	.user-editing textarea {
		display: block;
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
		overflow-y: hidden;
	}

	.edit-actions {
		position: absolute;
		right: 4px;
		bottom: -20px;
		display: flex;
		gap: 2px;
		padding: 2px;
		border: 1px solid var(--color-border);
		border-radius: 8px;
		background: var(--color-bg-elevated);
		box-shadow: var(--shadow-elevated);
		z-index: 5;
	}

	.edit-btn {
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
	}

	.edit-btn:hover:not(:disabled) {
		background: rgba(255, 255, 255, 0.08);
		color: var(--color-text-primary);
	}

	.edit-btn.send,
	.edit-btn.send:hover:not(:disabled) {
		color: var(--color-accent);
	}

	.edit-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}

	/* ── assistant blocks ── */

	.assistant-block {
		position: relative;
		width: 100%;
		padding: 0 20px;
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

	/* Preflight resets list-style to none; restore markers for markdown lists. */
	.md :global(ul) {
		list-style: disc;
	}

	.md :global(ol) {
		list-style: decimal;
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
		background: var(--color-border);
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
		bottom: 2px;
		z-index: 6;
	}

	.meta-anchor:hover {
		z-index: 7;
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

	/* The ::after bridge overlays the icon, so brighten on anchor hover. */
	.meta-anchor:hover .meta-icon {
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

	.meta-card.below {
		bottom: auto;
		top: calc(100% + 6px);
	}

	/* Hover bridge from the card's bottom edge down through the gap and the
	   icon to the anchor's bottom edge, so the cursor can travel between
	   anchor and card without closing it. Right trapezoid: full anchor width
	   (16px) at the bottom, half card width at the top. Hit-testing follows
	   the clip-path, so only the trapezoid area is hoverable. */
	.meta-card::after {
		content: '';
		position: absolute;
		top: 100%;
		left: 0;
		right: 0;
		height: 22px; /* 6px gap + 16px icon: reaches the anchor's bottom edge */
		clip-path: polygon(0 0, 50% 0, 16px 100%, 0 100%);
	}

	/* Mirrored bridge for the flipped card: from the anchor's top edge
	   (narrow, icon width) down to the card's top edge (wide). */
	.meta-card.below::after {
		top: auto;
		bottom: 100%;
		clip-path: polygon(0 0, 16px 0, 50% 100%, 0 100%);
	}

	.meta-row {
		display: flex;
		justify-content: space-between;
		gap: 12px;
		font-size: 0.68rem;
		white-space: nowrap;
		user-select: text;
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
		width: 100%;
		box-sizing: border-box;
		flex-shrink: 0;
		padding: 6px max(12px, calc((100% - var(--chat-max-width)) / 2 + 10px));
		border-top: 1px solid var(--color-border);
		background: color-mix(in srgb, var(--color-danger) 10%, transparent);
		color: var(--color-danger);
		font-size: 0.72rem;
		word-break: break-word;
	}

	.queued-bar {
		width: 100%;
		box-sizing: border-box;
		flex-shrink: 0;
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 5px max(10px, calc((100% - var(--chat-max-width)) / 2 + 10px));
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
