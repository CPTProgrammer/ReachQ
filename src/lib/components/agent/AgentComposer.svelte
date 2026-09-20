<script module lang="ts">
	export interface Props {
		scope: string;
		/** Active tab's connection: tool-execution hint in send options. */
		connectionId?: string;
		threadId: string | null;
	}
</script>

<script lang="ts">
	import type { InstanceModels, ModelMeta } from '$lib/ipc/agent';
	import { agentCancelRun, agentSendMessage, getThreadRuntime } from '$lib/state/agent.svelte';
	import {
		findModel,
		getInstanceModels,
		resolveSelection,
		setScopeSelection,
		type ComposerSelection
	} from '$lib/state/agent-settings.svelte';
	import { getThreads } from '$lib/state/agent-threads.svelte';
	import { t } from '$lib/state/i18n.svelte';
	import { flushDraft, getDraft, setDraft } from '$lib/state/agent-drafts.svelte';
	import { setFollowing } from '$lib/state/agent-chat-follow.svelte';
	import { formatContextLength, formatEffort } from '$lib/utils/formatters';
	import { autogrowTextarea } from '$lib/utils/autogrow';
	import ProviderIcon from './ProviderIcon.svelte';

	let { scope, connectionId, threadId }: Props = $props();

	let runtime = $derived(threadId ? getThreadRuntime(threadId) : null);
	let draftKey = $derived(threadId ?? `__scope:${scope}`);

	// ── Model / thinking / effort selection (design 01 §3.6, 05 §5) ──────────

	let sel = $state<ComposerSelection | null>(null);
	let selKey = $state('');

	$effect(() => {
		const groups = getInstanceModels();
		const summary = threadId ? getThreads().find((x) => x.id === threadId) : undefined;
		const key = `${scope}|${threadId ?? ''}|${groups.length}:${groups.map((g) => g.models?.length ?? 0).join(',')}`;
		if (key === selKey) return;
		selKey = key;
		sel = resolveSelection(scope, summary?.model ?? null);
	});

	let selectedModel = $derived(sel ? findModel(sel.model) : null);
	let modelMeta = $derived(selectedModel?.model ?? null);
	let providerPreset = $derived(selectedModel?.instance.preset ?? null);

	let thinkingOn = $derived(
		modelMeta
			? modelMeta.thinkingMandatory || (modelMeta.supportsThinking && (sel?.thinking ?? false))
			: (sel?.thinking ?? false)
	);

	let efforts = $derived(modelMeta?.thinkingEfforts ?? []);
	let currentEffort = $derived(sel?.effort ?? modelMeta?.defaultEffort ?? 'medium');

	function toggleThinking(): void {
		if (!sel || !modelMeta) return;
		if (!modelMeta.supportsThinking || modelMeta.thinkingMandatory) return;
		sel = { ...sel, thinking: !sel.thinking };
		setScopeSelection(scope, sel);
	}

	function pickEffort(effort: string): void {
		if (!sel) return;
		sel = { ...sel, effort };
		setScopeSelection(scope, sel);
		effortOpen = false;
	}

	function pickModel(group: InstanceModels, m: ModelMeta): void {
		sel = {
			model: `${group.instance.id}/${m.id}`,
			thinking: m.thinkingMandatory ? true : m.supportsThinking ? (sel?.thinking ?? false) : false,
			effort: m.defaultEffort ?? (m.thinkingEfforts.includes('medium') ? 'medium' : (m.thinkingEfforts[0] ?? null))
		};
		setScopeSelection(scope, sel);
		modelOpen = false;
	}

	let modelGroups = $derived(getInstanceModels().filter((g) => (g.models?.length ?? 0) > 0));

	// ── Context usage ring (design 01 §3.3) ──────────────────────────────────

	let usage = $derived(runtime?.lastUsage ?? null);
	let contextLength = $derived(modelMeta?.contextLength ?? 0);
	let usageFraction = $derived(usage && contextLength > 0 ? usage.promptTokens / contextLength : 0);
	let ringColor = $derived(
		usageFraction >= 1
			? 'var(--color-danger)'
			: usageFraction >= 0.7
				? 'var(--color-warning)'
				: 'var(--color-text-secondary)'
	);

	// ── Input + send / stop / queue (design 01 §3.4) ─────────────────────────

	let inputEl: HTMLTextAreaElement | undefined = $state();

	$effect(() => {
		getDraft(draftKey); // subscribe
		autogrowTextarea(inputEl);
	});

	let sendOpts = $derived.by(() => {
		if (!sel) return null;
		return {
			model: sel.model,
			thinking: thinkingOn,
			effort: thinkingOn ? (sel.effort ?? undefined) : undefined,
			connectionHint: connectionId
		};
	});

	let hasText = $derived(getDraft(draftKey).trim().length > 0);
	let running = $derived(runtime?.running ?? false);
	let mode = $derived<'send' | 'stop' | 'queue'>(!running ? 'send' : hasText ? 'queue' : 'stop');

	async function send(): Promise<void> {
		const text = getDraft(draftKey).trim();
		if (!text || !threadId || !sendOpts) return;
		if (running && runtime?.queued) return; // queue full: no-op (design 01 §3.5)
		setDraft(draftKey, '');
		void flushDraft(draftKey); // delete the persisted draft row now, not in 400ms
		setFollowing(true); // sending always jumps the chat to the bottom
		await agentSendMessage(scope, threadId, text, sendOpts);
	}

	function onKeydown(e: KeyboardEvent): void {
		if (e.key !== 'Enter' || e.isComposing) return;
		if (e.ctrlKey || e.shiftKey || e.altKey || e.metaKey) return; // newline
		e.preventDefault();
		if (mode === 'stop') return; // stop is click-only (design 01 §3.4)
		void send();
	}

	function onActionClick(): void {
		if (mode === 'stop') {
			// A queued message goes back to the composer, not into the void.
			if (threadId) void agentCancelRun(threadId);
		} else {
			void send();
		}
	}

	// ── Usage hover card (delayed, non-native tooltip) ────────────────────────

	let usageCardOpen = $state(false);
	let usageTimer: ReturnType<typeof setTimeout> | undefined;

	function scheduleUsageCard(): void {
		clearTimeout(usageTimer);
		usageTimer = setTimeout(() => (usageCardOpen = true), 400);
	}

	function hideUsageCard(): void {
		clearTimeout(usageTimer);
		usageCardOpen = false;
	}

	// ── Menus (click-outside) ────────────────────────────────────────────────

	let modelOpen = $state(false);
	let effortOpen = $state(false);
	let modelEl: HTMLDivElement | undefined = $state();
	let effortEl: HTMLDivElement | undefined = $state();

	$effect(() => {
		if (!modelOpen) return;
		const handler = (e: MouseEvent) => {
			if (modelEl && !modelEl.contains(e.target as Node)) modelOpen = false;
		};
		document.addEventListener('mousedown', handler, true);
		return () => document.removeEventListener('mousedown', handler, true);
	});

	$effect(() => {
		if (!effortOpen) return;
		const handler = (e: MouseEvent) => {
			if (effortEl && !effortEl.contains(e.target as Node)) effortOpen = false;
		};
		document.addEventListener('mousedown', handler, true);
		return () => document.removeEventListener('mousedown', handler, true);
	});
</script>

<div class="composer-area">
	<div class="composer">
		<textarea
			bind:this={inputEl}
			bind:value={() => getDraft(draftKey), (v) => setDraft(draftKey, v)}
			onkeydown={onKeydown}
			placeholder={t('agent.placeholder')}
			rows="4"
			disabled={!threadId}
		></textarea>

		<div class="bar">
			<div class="button-bar thinking-button-bar">
				<!-- Thinking toggle (design 01 §3.1) -->
				<button
					type="button"
					class="bar-btn icon-only"
					class:active={thinkingOn}
					disabled={!modelMeta || !modelMeta.supportsThinking || modelMeta.thinkingMandatory}
					title={!modelMeta || !modelMeta.supportsThinking
						? t('agent.thinking_unavailable')
						: modelMeta.thinkingMandatory
							? t('agent.thinking_locked')
							: t('agent.thinking')}
					aria-label={t('agent.thinking')}
					onclick={toggleThinking}
				>
					<!-- <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5a3 3 0 1 0-6 .1 4 4 0 0 0-2.5 5.8 4 4 0 0 0 .6 6.6A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 6 .1 4 4 0 0 1 2.5 5.8 4 4 0 0 1-.6 6.6A4 4 0 1 1 12 18Z"/><path d="M12 5v13"/></svg> -->
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
						{#if thinkingOn}
							<path d="M 16.19 13.29 A 4 4 0 0 0 15 16.12 V 19 a 1 1 0 0 1 -1 1 H 10 a 1 1 0 0 1 -1 -1 V 16.13 a 4.13 4.13 0 0 0 -1.26 -2.91 a 6 6 0 1 1 8.45 0.07 Z M 9 16 a 0.57 0.57 0 0 1 0 0.13 V 19 a 1 1 0 0 0 1 1 h 4 a 1 1 0 0 0 1 -1 V 16.12 A 0.49 0.49 0 0 1 15 16 Z m 4 4 H 11 v 1 h 2 Z"/>
						{:else}
							<path d="M 4 4 L 20 20 M 15.129 15.133 A 4 4 0 0 0 15 16.12 V 19 a 1 1 0 0 1 -1 1 H 10 a 1 1 0 0 1 -1 -1 V 16.13 a 4.13 4.13 0 0 0 -1.26 -2.91 a 6 6 0 0 1 -1.219 -6.68 M 9.456 3.575 A 6 6 0 0 1 17.522 11.355 M 9 16 a 0.57 0.57 0 0 1 0 0.13 V 19 a 1 1 0 0 0 1 1 h 4 a 1 1 0 0 0 1 -1 V 16.12 A 0.49 0.49 0 0 1 15 16 Z m 4 4 H 11 v 1 h 2 Z"/>
						{/if}
					</svg>
				</button>

				<!-- Reasoning effort (design 01 §3.2) -->
				{#if efforts.length > 0}
					<div class="split"></div>

					<div class="menu-anchor effort-menu-anchor" bind:this={effortEl}>
						<button
							type="button"
							class="bar-btn"
							disabled={!thinkingOn}
							title={t('agent.effort')}
							onclick={() => (effortOpen = !effortOpen)}
						>
							<span>{formatEffort(currentEffort)}</span>
							<svg width="9" height="9" viewBox="0 0 12 12" fill="none"><path d="M3 4.5 6 7.5 9 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
						</button>
						{#if effortOpen}
							<div class="menu">
								{#each efforts as effort (effort)}
									<button type="button" class="menu-item" class:selected={effort === currentEffort} onclick={() => pickEffort(effort)}>
										<span class="menu-check">
											{#if effort === currentEffort}
												<svg width="11" height="11" viewBox="0 0 14 14" fill="none"><path d="M2 7l3.5 3.5L12 3.5" stroke="var(--color-accent)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
											{/if}
										</span>
										<span>{formatEffort(effort)}</span>
									</button>
								{/each}
							</div>
						{/if}
					</div>
				{/if}
			</div>

			<span class="spacer"></span>

			<!-- Context usage ring (design 01 §3.3; hidden without usage) -->
			{#if usage && contextLength > 0}
				<div
					class="menu-anchor usage-anchor"
					onmouseenter={scheduleUsageCard}
					onmouseleave={hideUsageCard}
					role="presentation"
				>
					<span class="usage-ring">
						<svg width="16" height="16" viewBox="0 0 18 18">
							<circle cx="9" cy="9" r="7" fill="none" stroke="var(--color-border)" stroke-width="2.4" />
							<circle
								cx="9" cy="9" r="7" fill="none"
								stroke={ringColor}
								stroke-width="2.4"
								stroke-linecap="round"
								stroke-dasharray={`${Math.min(1, usageFraction) * 43.98} 43.98`}
								transform="rotate(-90 9 9)"
							/>
						</svg>
					</span>
					{#if usageCardOpen}
						<div class="menu usage-card">
							<div class="usage-card-title">{t('agent.context')}</div>
							<div class="usage-card-row">
								<span>
									{formatContextLength(usage.promptTokens)}<span class="usage-card-dim">{' / '}{formatContextLength(contextLength)}</span>
								</span>
								<span>{Math.round(usageFraction * 100)}%</span>
							</div>
						</div>
					{/if}
				</div>
			{/if}

			<!-- Model picker (design 01 §3.6) -->
			<div class="menu-anchor" bind:this={modelEl}>
				<button
					type="button"
					class="bar-btn model-btn"
					title={t('agent.model')}
					onclick={() => (modelOpen = !modelOpen)}
				>
					{#if providerPreset}
						<ProviderIcon preset={providerPreset} size={14} />
					{/if}
					<span class="model-name">{modelMeta?.displayName ?? t('agent.model')}</span>
					<svg width="9" height="9" viewBox="0 0 12 12" fill="none"><path d="M3 4.5 6 7.5 9 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
				</button>
				{#if modelOpen}
					<div class="menu model-menu">
						{#each modelGroups as group (group.instance.id)}
							<div class="menu-group">{group.instance.name}</div>
							{#each group.models ?? [] as m (m.id)}
								<button
									type="button"
									class="menu-item"
									class:selected={sel?.model === `${group.instance.id}/${m.id}`}
									onclick={() => pickModel(group, m)}
								>
									<span class="menu-icon"><ProviderIcon preset={group.instance.preset} size={14} /></span>
									<span class="menu-label">{m.displayName}</span>
									<span class="menu-check">
										{#if sel?.model === `${group.instance.id}/${m.id}`}
											<svg width="11" height="11" viewBox="0 0 14 14" fill="none"><path d="M2 7l3.5 3.5L12 3.5" stroke="var(--color-accent)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
										{/if}
									</span>
									<span class="menu-badge">{formatContextLength(m.contextLength)}</span>
								</button>
							{/each}
						{/each}
						{#if modelGroups.length === 0}
							<div class="menu-empty">{t('agent.configure_models')}</div>
						{/if}
					</div>
				{/if}
			</div>

			<!-- Action button: send / stop / queue (design 01 §3.4) -->
			<button
				type="button"
				class="action-btn"
				class:stop={mode === 'stop'}
				disabled={(mode === 'send' && (!hasText || !threadId || !sendOpts)) ||
					(mode === 'queue' && !!runtime?.queued)}
				title={mode === 'send' ? t('agent.send') : mode === 'stop' ? t('agent.stop') : t('agent.queue')}
				aria-label={mode === 'send' ? t('agent.send') : mode === 'stop' ? t('agent.stop') : t('agent.queue')}
				onclick={onActionClick}
			>
				{#if mode === 'send'}
					<svg width="16" height="16" viewBox="0 0 24 24" stroke="currentColor" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11.5003 12H5.41872M5.24634 12.7972L4.24158 15.7986C3.69128 17.4424 3.41613 18.2643 3.61359 18.7704C3.78506 19.21 4.15335 19.5432 4.6078 19.6701C5.13111 19.8161 5.92151 19.4604 7.50231 18.7491L17.6367 14.1886C19.1797 13.4942 19.9512 13.1471 20.1896 12.6648C20.3968 12.2458 20.3968 11.7541 20.1896 11.3351C19.9512 10.8529 19.1797 10.5057 17.6367 9.81135L7.48483 5.24303C5.90879 4.53382 5.12078 4.17921 4.59799 4.32468C4.14397 4.45101 3.77572 4.78336 3.60365 5.22209C3.40551 5.72728 3.67772 6.54741 4.22215 8.18767L5.24829 11.2793C5.34179 11.561 5.38855 11.7019 5.407 11.8459C5.42338 11.9738 5.42321 12.1032 5.40651 12.231C5.38768 12.375 5.34057 12.5157 5.24634 12.7972Z"></path></svg>
				{:else if mode === 'stop'}
					<svg width="8" height="8" viewBox="0 0 24 24" fill="currentColor"><rect x="0" y="0" width="24" height="24"/></svg>
				{:else}
					<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 7h12"/><path d="M6 12h12"/><path d="M6 17h7"/><path d="M19 16v7"/><path d="M15.5 19.5h7"/></svg>
				{/if}
			</button>
		</div>
	</div>
</div>

<style>
	.composer-area {
		display: flex;
		justify-content: center;
		border-top: 1px solid var(--color-border);
		background: var(--color-bg-elevated);
	}

	.composer {
		flex-shrink: 0;
		flex-grow: 1;
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 8px;
		max-width: var(--chat-max-width);
	}

	textarea {
		box-sizing: border-box;
		border: none;
		background: transparent;
		color: var(--color-text-primary);
		font-family: var(--font-sans);
		font-size: 0.8rem;
		resize: none;
		outline: none;
		max-height: 180px;
		margin: 4px 4px 0px;
		scrollbar-gutter: stable;
	}

	textarea:focus {
		border-radius: 0;
	}

	textarea::placeholder {
		color: var(--color-text-secondary);
		opacity: 0.6;
	}

	textarea:disabled {
		opacity: 0.5;
	}

	.bar {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.spacer {
		flex: 1;
	}

	.bar-btn {
		display: flex;
		align-items: center;
		gap: 4px;
		height: 24px;
		padding: 0 7px;
		border: none;
		border-radius: 6px;
		background: transparent;
		color: var(--color-text-secondary);
		font-size: 0.7rem;
		font-family: var(--font-sans);
		cursor: pointer;
		transition:
			background-color 150ms ease,
			color 150ms ease;
	}

	.bar-btn:hover:not(:disabled) {
		background: color-mix(in srgb, var(--color-contrast) 8%, transparent);
	}

	.bar-btn:hover:not(:disabled):not(.icon-only) {
		color: var(--color-text-primary);
	}

	.bar-btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.bar-btn.icon-only {
		width: 24px;
		padding: 0;
		justify-content: center;
	}

	.bar-btn.active {
		color: var(--color-accent);
	}

	.button-bar {
		display: flex;
	}

	.button-bar > .split {
		width: 1px;
		margin: 3px 1px;
		height: 18px;
		background-color: var(--color-text-primary);
		opacity: 10%;
	}

	.thinking-button-bar > .bar-btn:first-child:not(:last-child) {
		border-top-right-radius: 0;
		border-bottom-right-radius: 0;
	}

	.thinking-button-bar > .menu-anchor > .bar-btn {
		border-top-left-radius: 0;
		border-bottom-left-radius: 0;
	}

	.model-btn {
		max-width: 180px;
	}

	.model-name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		/* Optical alignment: Inter glyphs sit high in the line box (the
		   descender space below the baseline is unused by most model names),
		   so nudge the text down to optically center it against the icon. */
		position: relative;
		top: 0.5px;
	}

	.usage-ring {
		display: flex;
		align-items: center;
		padding: 0 4px;
		cursor: default;
	}

	.usage-anchor {
		display: flex;
		align-items: center;
	}

	/* More specific than .menu so these overrides win regardless of order. */
	.menu.usage-card {
		max-height: none;
		overflow: visible;
		padding: 7px 9px;
		white-space: nowrap;
		cursor: default;
	}

	/* Hover bridge from the card's bottom edge down through the gap to the
	   ring, so the cursor can travel between ring and card without closing it
	   (same technique as .meta-card::after, mirrored: the card is right-aligned
	   to the anchor). Hit-testing follows the clip-path. */
	.menu.usage-card::after {
		content: '';
		position: absolute;
		top: 100%;
		left: 0;
		right: 0;
		height: 22px; /* 6px gap + 16px ring: reaches the anchor's bottom edge */
		clip-path: polygon(50% 0, 100% 0, 100% 100%, calc(100% - 24px) 100%);
	}

	.usage-card-title {
		font-size: 0.62rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--color-text-secondary);
		margin-bottom: 4px;
	}

	.usage-card-row {
		display: flex;
		justify-content: space-between;
		gap: 16px;
		font-size: 0.75rem;
		font-family: var(--font-mono);
		color: var(--color-text-primary);
	}

	.usage-card-dim {
		color: var(--color-text-secondary);
	}

	/* ── pop-up menus ── */

	.menu-anchor {
		position: relative;
	}

	.menu {
		position: absolute;
		bottom: calc(100% + 6px);
		right: 0;
		z-index: 60;
		min-width: 150px;
		max-height: 260px;
		overflow-y: auto;
		padding: 4px;
		background: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		box-shadow: var(--shadow-elevated);
	}

	.menu-anchor:first-child .menu,
	.effort-menu-anchor .menu {
		left: 0;
		right: auto;
	}

	.model-menu {
		min-width: 230px;
	}

	.menu-group {
		padding: 5px 8px 3px;
		font-size: 0.62rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--color-text-secondary);
		border-top: 1px solid var(--color-border);
		margin-top: 3px;
	}

	.menu-group:first-child {
		border-top: none;
		margin-top: 0;
	}

	.menu-item {
		display: flex;
		align-items: center;
		gap: 6px;
		width: 100%;
		padding: 5px 8px;
		border: none;
		border-radius: 5px;
		background: transparent;
		color: var(--color-text-primary);
		font-size: 0.75rem;
		font-family: var(--font-sans);
		cursor: pointer;
		text-align: left;
		white-space: nowrap;
	}

	.menu-item:hover {
		background: color-mix(in srgb, var(--color-contrast) 6%, transparent);
	}

	.menu-check {
		width: 12px;
		display: flex;
		align-items: center;
		flex-shrink: 0;
	}

	.menu-icon {
		width: 14px;
		display: flex;
		align-items: center;
		flex-shrink: 0;
	}

	.menu-item.selected .menu-icon {
		color: var(--color-accent);
	}

	.menu-label {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.menu-badge {
		flex-shrink: 0;
		padding: 1px 5px;
		border-radius: 4px;
		background: var(--color-bg-secondary);
		color: var(--color-text-secondary);
		font-size: 0.62rem;
		font-family: var(--font-mono);
	}

	.menu-empty {
		padding: 8px 10px;
		font-size: 0.72rem;
		color: var(--color-text-secondary);
		opacity: 0.7;
		white-space: nowrap;
	}

	/* ── action button ── */

	.action-btn {
		--action-btn-background-color: var(--color-text-primary);
		display: flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		border: none;
		border-radius: 6px;
		background: color-mix(in srgb, var(--action-btn-background-color) 10%, transparent);
		color: var(--color-accent);
		cursor: pointer;
		flex-shrink: 0;
		transition: opacity 150ms ease;
	}

	.action-btn:not(:disabled):hover {
		background: color-mix(in srgb, var(--action-btn-background-color) 5%, transparent);
	}

	.action-btn:disabled {
		opacity: 0.5;
		cursor: default;
		color: var(--color-text-secondary);
	}

	.action-btn.stop {
		--action-btn-background-color: var(--color-danger);
		color: var(--color-danger);
	}
</style>
