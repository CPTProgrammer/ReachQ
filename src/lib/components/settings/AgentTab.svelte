<script lang="ts">
	import { tick } from 'svelte';
	import Button from '$lib/components/shared/Button.svelte';
	import Dropdown from '$lib/components/shared/Dropdown.svelte';
	import Input from '$lib/components/shared/Input.svelte';
	import Toggle from '$lib/components/shared/Toggle.svelte';
	import ProviderIcon from '$lib/components/agent/ProviderIcon.svelte';
	import { getAgentBackend } from '$lib/state/agent.svelte';
	import {
		findModel,
		getInstanceModels,
		getProviderInstances,
		getTitleModel,
		getToolSettings,
		loadModels,
		loadProviders,
		loadTitleModel,
		loadToolSettings,
		setTitleModel,
		setToolConfig
	} from '$lib/state/agent-settings.svelte';
	import {
		archiveThread,
		deleteThread,
		getAllThreads,
		loadAllThreads,
		reassignThread,
		relativeTime,
		renameThread,
		threadDisplayTitle,
		unarchiveThread
	} from '$lib/state/agent-threads.svelte';
	import { sessionList, type SessionConfig } from '$lib/ipc/sessions';
	import { isLocked } from '$lib/state/vault.svelte';
	import { t, tOr } from '$lib/state/i18n.svelte';
	import { agentComputeIdentity, type ProviderInstance, type ThreadSummary, type ToolSettingsEntry } from '$lib/ipc/agent';

	type ToolOption = ToolSettingsEntry['options'][number];

	// ---------------------------------------------------------------------------
	// View switch: main settings <-> "all threads" sub-view (design 04 §4.5)
	// ---------------------------------------------------------------------------

	let view = $state<'main' | 'threads'>('main');

	// ---------------------------------------------------------------------------
	// Data loading (provider config is vault-backed; reload on unlock)
	// ---------------------------------------------------------------------------

	let presets = $state<[string, string, string][]>([]);

	$effect(() => {
		if (!isLocked()) {
			void refreshProviders().catch(() => {});
			void loadModels().catch(() => {});
			void loadTitleModel().catch(() => {});
		}
	});

	$effect(() => {
		void loadToolSettings().catch(() => {});
	});

	$effect(() => {
		void getAgentBackend()
			.providerPresets()
			.then((p) => (presets = p))
			.catch(() => {});
	});

	function errText(e: unknown): string {
		return e instanceof Error ? e.message : String(e);
	}

	// ---------------------------------------------------------------------------
	// Provider instances (design 04 §4.1)
	// ---------------------------------------------------------------------------

	let editingId = $state<string | null>(null);
	let editName = $state('');
	let editKey = $state('');
	let editUrl = $state('');
	let editBusy = $state(false);
	let validating = $state<Record<string, boolean>>({});
	let validateMsg = $state<Record<string, { ok: boolean; text: string } | undefined>>({});
	let confirmDeleteId = $state<string | null>(null);

	async function refreshProviders(): Promise<void> {
		await loadProviders();
		const ids = new Set(getProviderInstances().map((i) => i.id));
		for (const id of Object.keys(validating)) {
			if (!ids.has(id)) {
				delete validating[id];
				delete validateMsg[id];
			}
		}
	}

	function presetLabel(presetId: string): string {
		return presets.find(([id]) => id === presetId)?.[1] ?? presetId;
	}

	function presetDefaultUrl(presetId: string): string {
		return presets.find(([id]) => id === presetId)?.[2] ?? '';
	}

	async function startEdit(inst: ProviderInstance) {
		confirmDeleteId = null;
		editingId = inst.id;
		editName = inst.name;
		editUrl = inst.baseUrl ?? '';
		editKey = '';
		const key = await getAgentBackend()
			.providerGetApiKey(inst.id)
			.catch(() => '');
		if (editingId === inst.id) editKey = key;
	}

	function discardEdit() {
		editingId = null;
	}

	async function saveEdit(inst: ProviderInstance) {
		if (editBusy) return;
		editBusy = true;
		try {
			await getAgentBackend().providerUpdate(inst.id, editName.trim(), editUrl.trim());
			await getAgentBackend().providerSetApiKey(inst.id, editKey.trim());
			validateMsg[inst.id] = undefined;
			editingId = null;
			await refreshProviders().catch(() => {});
		} catch (e) {
			// Stay in edit mode so the user can retry or discard.
			validateMsg[inst.id] = { ok: false, text: errText(e) };
		} finally {
			editBusy = false;
		}
	}

	async function onValidate(inst: ProviderInstance) {
		validating[inst.id] = true;
		validateMsg[inst.id] = undefined;
		try {
			const models = await getAgentBackend().providerValidate(inst.id);
			validateMsg[inst.id] = {
				ok: true,
				text: t('agent.settings_validate_ok', { count: models.length })
			};
			await loadModels().catch(() => {});
		} catch (e) {
			validateMsg[inst.id] = { ok: false, text: errText(e) };
		} finally {
			validating[inst.id] = false;
		}
	}

	async function confirmDeleteInstance(id: string) {
		confirmDeleteId = null;
		try {
			await getAgentBackend().providerDelete(id);
		} catch {
			/* fall through to refresh */
		}
		await refreshProviders().catch(() => {});
		await loadModels().catch(() => {});
	}

	// --- Add provider (preset -> key form; default naming happens backend-side)

	let addOpen = $state(false);
	let addPreset = $state('');
	let addName = $state('');
	let addKey = $state('');
	let addBaseUrl = $state('');
	let addBusy = $state(false);
	let addError = $state<string | null>(null);

	// Mirrors the backend's default naming in agent_provider_add: preset
	// display name, then "Name 2", "Name 3"... for repeat instances.
	function defaultInstanceName(presetId: string): string {
		const label = presetLabel(presetId);
		const count = getProviderInstances().filter((i) => i.preset === presetId).length;
		return count === 0 ? label : `${label} ${count + 1}`;
	}

	function openAddForm() {
		addOpen = true;
		addError = null;
		addKey = '';
		addPreset = presets[0]?.[0] ?? '';
		addName = defaultInstanceName(addPreset);
		addBaseUrl = presetDefaultUrl(addPreset);
	}

	function selectAddPreset(id: string) {
		if (id === addPreset) return;
		// Refresh auto-filled defaults for the new preset, but keep fields the
		// user has customized (non-empty and no longer matching the old default).
		if (!addName.trim() || addName === defaultInstanceName(addPreset)) {
			addName = defaultInstanceName(id);
		}
		if (!addBaseUrl.trim() || addBaseUrl === presetDefaultUrl(addPreset)) {
			addBaseUrl = presetDefaultUrl(id);
		}
		addPreset = id;
	}

	async function submitAdd() {
		if (!addPreset || !addKey.trim() || addBusy) return;
		addBusy = true;
		addError = null;
		try {
			await getAgentBackend().providerAdd(
				addPreset,
				addKey.trim(),
				addName.trim() || undefined,
				addBaseUrl.trim() || undefined
			);
			addOpen = false;
			await refreshProviders().catch(() => {});
			await loadModels().catch(() => {});
		} catch (e) {
			addError = errText(e);
		} finally {
			addBusy = false;
		}
	}

	function formatContext(length: number): string {
		if (!length) return '';
		if (length >= 1_000_000) return `${Math.round(length / 1_000_000)}M`;
		if (length >= 1000) return `${Math.round(length / 1000)}K`;
		return String(length);
	}

	// ---------------------------------------------------------------------------
	// Tool settings (design 04 §4.2/§4.3, reverse-registered option controls)
	// ---------------------------------------------------------------------------

	function saveTool(
		tool: ToolSettingsEntry,
		enabled: boolean,
		requireApproval: boolean,
		options: Record<string, unknown>
	) {
		void setToolConfig(tool.name, enabled, requireApproval, options).catch(() => {});
	}

	function setToolOption(tool: ToolSettingsEntry, key: string, value: unknown) {
		saveTool(tool, tool.enabled, tool.requireApproval, { ...tool.values, [key]: value });
	}

	function effectiveValue(tool: ToolSettingsEntry, opt: ToolOption): unknown {
		const v = tool.values[opt.key];
		return v === undefined || v === null ? opt.default : v;
	}

	function optString(v: unknown): string {
		return typeof v === 'string' ? v : '';
	}

	function optNumber(v: unknown): number | '' {
		return typeof v === 'number' ? v : '';
	}

	function optBool(v: unknown): boolean {
		return v === true;
	}

	function optList(v: unknown): string[] {
		return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
	}

	function onNumberOption(
		tool: ToolSettingsEntry,
		key: string,
		e: Event & { currentTarget: HTMLInputElement }
	) {
		const raw = e.currentTarget.value;
		if (raw === '') return;
		const num = Number(raw);
		if (Number.isNaN(num)) return;
		setToolOption(tool, key, num);
	}

	let listDrafts = $state<Record<string, string>>({});

	function listKey(tool: string, key: string): string {
		return `${tool}:${key}`;
	}

	function listAddItem(tool: ToolSettingsEntry, opt: ToolOption) {
		const dk = listKey(tool.name, opt.key);
		const value = (listDrafts[dk] ?? '').trim();
		if (!value) return;
		listDrafts[dk] = '';
		setToolOption(tool, opt.key, [...optList(effectiveValue(tool, opt)), value]);
	}

	function listRemoveItem(tool: ToolSettingsEntry, opt: ToolOption, index: number) {
		setToolOption(
			tool,
			opt.key,
			optList(effectiveValue(tool, opt)).filter((_, i) => i !== index)
		);
	}

	// ---------------------------------------------------------------------------
	// Title generation model (design 04 §4.4)
	// ---------------------------------------------------------------------------

	let titleOpen = $state(false);
	let titleDdEl = $state<HTMLDivElement | undefined>(undefined);
	let titleDropUp = $state(false);

	// NOTE: this grouped dropdown re-implements the shared Dropdown.svelte
	// behavior (including the drop-up flip) because Dropdown only supports flat
	// option lists. Prefer extending the shared component with option groups and
	// migrating to it rather than growing this parallel implementation.
	function toggleTitleDropdown() {
		if (!titleOpen && titleDdEl) {
			const spaceBelow = window.innerHeight - titleDdEl.getBoundingClientRect().bottom;
			titleDropUp = spaceBelow < 264; // list max-height (240px) + gap + margin
		}
		titleOpen = !titleOpen;
	}

	$effect(() => {
		if (!titleOpen) return;
		function onDocClick(e: MouseEvent) {
			if (titleDdEl && !titleDdEl.contains(e.target as Node)) titleOpen = false;
		}
		document.addEventListener('click', onDocClick, true);
		return () => document.removeEventListener('click', onDocClick, true);
	});

	const titleGroups = $derived(
		getInstanceModels().filter((g) => (g.models?.length ?? 0) > 0)
	);

	const titleLabel = $derived.by(() => {
		const sel = getTitleModel();
		if (!sel) return t('agent.settings_title_model_none');
		const found = findModel(sel);
		return found ? `${found.instance.name} / ${found.model.displayName || found.model.id}` : sel;
	});

	function selectTitleModel(value: string | null) {
		titleOpen = false;
		void setTitleModel(value).catch(() => {});
	}

	// ---------------------------------------------------------------------------
	// All threads sub-view (design 04 §4.5, 01 §2.1)
	// ---------------------------------------------------------------------------

	let editingThreadId = $state<string | null>(null);
	let editingTitle = $state('');
	let renameInputEl = $state<HTMLInputElement | undefined>(undefined);
	let confirmDeleteThreadId = $state<string | null>(null);
	let reassignThreadId = $state<string | null>(null);
	let sessions = $state<SessionConfig[]>([]);

	const sortedThreads = $derived(
		[...getAllThreads()].sort((a, b) => b.updatedAt - a.updatedAt)
	);

	function openThreadsView() {
		view = 'threads';
		void loadAllThreads().catch(() => {});
		void sessionList().then((list) => (sessions = list)).catch(() => {});
	}

	/** Owner label: session name (or deleted marker) for session scopes,
	 * the embedded link identity for link scopes. */
	function ownerLabel(thread: ThreadSummary): string {
		if (thread.ownerKey.startsWith('session:')) {
			const id = thread.ownerKey.slice('session:'.length);
			return sessions.find((s) => s.id === id)?.name ?? t('agent.thread_owner_deleted_session');
		}
		if (thread.ownerKey.startsWith('link:')) {
			return thread.ownerKey.slice('link:'.length);
		}
		return thread.ownerKey;
	}

	/** Whether the thread's owning session still exists (move-to-link target
	 * can be computed from its current config). */
	function owningSession(thread: ThreadSummary): SessionConfig | undefined {
		if (!thread.ownerKey.startsWith('session:')) return undefined;
		const id = thread.ownerKey.slice('session:'.length);
		return sessions.find((s) => s.id === id);
	}

	async function commitReassign(threadId: string, ownerKey: string) {
		reassignThreadId = null;
		await reassignThread(threadId, ownerKey).catch(() => {});
	}

	/** Move a session-owned thread to the quick-connect link its session
	 * currently resolves to. */
	async function moveThreadToLink(thread: ThreadSummary) {
		const s = owningSession(thread);
		if (!s) return;
		try {
			const identity = await agentComputeIdentity({
				username: s.username,
				host: s.host,
				port: s.port,
				jumpChain: s.jump_chain,
				proxy: s.proxy
			});
			await commitReassign(thread.id, `link:${identity}`);
		} catch {
			/* identity computation failed — leave the thread where it is */
		}
	}

	function startRename(thread: ThreadSummary) {
		editingThreadId = thread.id;
		editingTitle = thread.title;
		void tick().then(() => renameInputEl?.focus());
	}

	async function commitRename() {
		const id = editingThreadId;
		if (!id) return;
		editingThreadId = null;
		const title = editingTitle.trim();
		if (title) await renameThread(id, title).catch(() => {});
	}

	function onRenameKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			e.preventDefault();
			void commitRename();
		} else if (e.key === 'Escape') {
			editingThreadId = null;
		}
	}

	async function confirmDeleteThread(id: string) {
		confirmDeleteThreadId = null;
		await deleteThread(id).catch(() => {});
	}
</script>

{#if view === 'threads'}
	<!-- All threads sub-view (design 04 §4.5) -->
	<div class="tab-content">
		<div class="threads-header">
			<button class="back-btn" onclick={() => (view = 'main')}>
				<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<path d="M15 18l-6-6 6-6" />
				</svg>
				{t('agent.back')}
			</button>
			<span class="setting-label">{t('agent.all_threads')}</span>
		</div>

		<div class="thread-list">
			{#if sortedThreads.length === 0}
				<div class="empty-hint">{t('agent.no_threads')}</div>
			{/if}
			{#each sortedThreads as thread (thread.id)}
				<div class="thread-item" class:confirming={confirmDeleteThreadId === thread.id} class:moving={reassignThreadId === thread.id}>
					<div class="thread-text">
						{#if editingThreadId === thread.id}
							<input
								bind:this={renameInputEl}
								class="title-edit"
								type="text"
								bind:value={editingTitle}
								onkeydown={onRenameKeydown}
								onblur={() => void commitRename()}
							/>
						{:else}
							<span class="thread-title" class:archived={thread.archived} class:untitled={!thread.title}>{threadDisplayTitle(thread)}</span>
						{/if}
						<div class="thread-meta">
							<span class="owner-label" class:owner-deleted={thread.ownerKey.startsWith('session:') && !sessions.some((s) => thread.ownerKey === `session:${s.id}`)}>{ownerLabel(thread)}</span>
							<span>{relativeTime(thread.updatedAt)}</span>
						</div>
					</div>
					{#if editingThreadId !== thread.id && confirmDeleteThreadId !== thread.id && reassignThreadId !== thread.id}
						<div class="thread-actions">
							{#if thread.archived}
								<button
									class="icon-btn"
									title={t('agent.unarchive')}
									onclick={() => void unarchiveThread(thread.id)}
								>
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
										<path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
										<path d="M3 3v5h5" />
									</svg>
								</button>
								<button
									class="icon-btn"
									title={t('agent.thread_move')}
									onclick={() => (reassignThreadId = thread.id)}
								>
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
										<path d="M5 12h14M13 6l6 6-6 6" />
										<path d="M3 4v16" />
									</svg>
								</button>
								<button
									class="icon-btn danger"
									title={t('agent.delete_thread')}
									onclick={() => (confirmDeleteThreadId = thread.id)}
								>
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
										<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
									</svg>
								</button>
							{:else}
								<button
									class="icon-btn"
									title={t('agent.edit_title')}
									onclick={() => startRename(thread)}
								>
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
										<path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z" />
									</svg>
								</button>
								<button
									class="icon-btn"
									title={t('agent.thread_move')}
									onclick={() => (reassignThreadId = thread.id)}
								>
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
										<path d="M5 12h14M13 6l6 6-6 6" />
										<path d="M3 4v16" />
									</svg>
								</button>
								<button
									class="icon-btn"
									title={t('agent.archive')}
									onclick={() => void archiveThread(thread.id)}
								>
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
										<path d="M21 8v13H3V8M1 3h22v5H1zM10 12h4" />
									</svg>
								</button>
							{/if}
						</div>
					{/if}
					{#if reassignThreadId === thread.id}
						<div class="confirm-bar reassign-bar">
							<span class="confirm-text">{t('agent.thread_move_to')}</span>
							<div class="reassign-options">
								{#each sessions as s (s.id)}
									<button
										class="confirm-btn"
										class:active={thread.ownerKey === `session:${s.id}`}
										onclick={() => void commitReassign(thread.id, `session:${s.id}`)}
									>{s.name}</button>
								{/each}
								{#if !thread.ownerKey.startsWith('link:') && owningSession(thread)}
									<button
										class="confirm-btn"
										onclick={() => void moveThreadToLink(thread)}
									>{t('agent.thread_move_to_link')}</button>
								{/if}
								<button class="confirm-btn" onclick={() => (reassignThreadId = null)}>
									{t('common.cancel')}
								</button>
							</div>
						</div>
					{/if}
					{#if confirmDeleteThreadId === thread.id}
						<div class="confirm-bar">
							<span class="confirm-text">{t('agent.delete_thread_confirm')}</span>
							<button class="confirm-btn danger" onclick={() => void confirmDeleteThread(thread.id)}>
								{t('common.confirm')}
							</button>
							<button class="confirm-btn" onclick={() => (confirmDeleteThreadId = null)}>
								{t('common.cancel')}
							</button>
						</div>
					{/if}
				</div>
			{/each}
		</div>
	</div>
{:else}
	<div class="tab-content">
		<!-- Providers (design 04 §4.1) -->
		<div class="section-header">
			<div class="setting-info">
				<span class="setting-label">{t('agent.settings_providers')}</span>
				<span class="setting-description">{t('agent.settings_providers_desc')}</span>
			</div>
			{#if !isLocked()}
				<Button variant="secondary" size="sm" onclick={openAddForm} disabled={addOpen}>
					{t('agent.settings_add_provider')}
				</Button>
			{/if}
		</div>

		{#if isLocked()}
			<div class="locked-hint">{t('agent.settings_unlock_vault')}</div>
		{:else}
			{#if getProviderInstances().length === 0 && !addOpen}
				<div class="empty-hint">{t('agent.settings_no_providers')}</div>
			{/if}

			{#each getProviderInstances() as inst (inst.id)}
				{@const group = getInstanceModels().find((g) => g.instance.id === inst.id)}
				{@const models = group?.models ?? []}
				<div class="provider-card">
					<div class="provider-head">
						{#if editingId === inst.id}
							<input
								class="name-input"
								type="text"
								bind:value={editName}
								aria-label={t('agent.settings_provider_name')}
							/>
							<span class="preset-badge"><ProviderIcon preset={inst.preset} size={11} />{presetLabel(inst.preset)}</span>
							<button class="icon-btn" title={t('common.cancel')} onclick={discardEdit}>
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
									<path d="M18 6L6 18M6 6l12 12" />
								</svg>
							</button>
							<button
								class="icon-btn save"
								title={t('common.save')}
								disabled={editBusy}
								onclick={() => void saveEdit(inst)}
							>
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
									<path d="M20 6L9 17l-5-5" />
								</svg>
							</button>
						{:else}
							<span class="name-text">{inst.name}</span>
							<span class="preset-badge"><ProviderIcon preset={inst.preset} size={11} />{presetLabel(inst.preset)}</span>
							<button class="icon-btn" title={t('common.edit')} onclick={() => void startEdit(inst)}>
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
									<path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z" />
								</svg>
							</button>
							<button
								class="icon-btn danger"
								title={t('agent.settings_delete_provider')}
								onclick={() => (confirmDeleteId = confirmDeleteId === inst.id ? null : inst.id)}
							>
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
									<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
								</svg>
							</button>
						{/if}
					</div>

					{#if confirmDeleteId === inst.id}
						<div class="confirm-bar">
							<span class="confirm-text">{t('agent.settings_delete_provider_confirm')}</span>
							<button class="confirm-btn danger" onclick={() => void confirmDeleteInstance(inst.id)}>
								{t('common.confirm')}
							</button>
							<button class="confirm-btn" onclick={() => (confirmDeleteId = null)}>
								{t('common.cancel')}
							</button>
						</div>
					{/if}

					{#if editingId === inst.id}
						<div class="field-row">
							<span class="field-label">{t('agent.settings_api_key')}</span>
							<Input
								type="password"
								bind:value={editKey}
								placeholder={t('agent.settings_api_key')}
							/>
						</div>

						<div class="field-row">
							<span class="field-label">{t('agent.settings_base_url')}</span>
							<Input
								type="text"
								bind:value={editUrl}
								placeholder={presetDefaultUrl(inst.preset)}
							/>
							<span class="field-desc">{t('agent.settings_base_url_desc')}</span>
						</div>

						<div class="validate-row">
							<Button
								variant="secondary"
								size="sm"
								disabled={!!validating[inst.id]}
								onclick={() => void onValidate(inst)}
							>
								{validating[inst.id]
									? t('agent.settings_validating')
									: t('agent.settings_validate')}
							</Button>
							{#if validateMsg[inst.id]}
								{@const msg = validateMsg[inst.id]!}
								<span class="validate-msg" class:ok={msg.ok} class:err={!msg.ok}>{msg.text}</span>
							{/if}
						</div>

						{#if models.length > 0}
							<div class="models-preview">
								{#each models as model (model.id)}
									<div class="model-line">
										<span class="model-name">{model.displayName || model.id}</span>
										{#if formatContext(model.contextLength)}
											<span class="model-ctx">{formatContext(model.contextLength)}</span>
										{/if}
									</div>
								{/each}
							</div>
						{:else if group?.error}
							<div class="validate-msg err models-error">{group.error}</div>
						{/if}
					{/if}
				</div>
			{/each}

			{#if addOpen}
				<div class="provider-card add-form">
					<div class="preset-row">
						{#each presets as [id, label] (id)}
							<button
								class="preset-chip"
								class:selected={addPreset === id}
								onclick={() => selectAddPreset(id)}
							>
								<ProviderIcon preset={id} size={12} />
								{label}
							</button>
						{/each}
					</div>
					<div class="field-row">
						<span class="field-label">{t('agent.settings_provider_name')}</span>
						<Input type="text" bind:value={addName} placeholder={defaultInstanceName(addPreset)} />
					</div>
					<div class="field-row">
						<span class="field-label">{t('agent.settings_api_key')}</span>
						<Input type="password" bind:value={addKey} placeholder={t('agent.settings_api_key')} />
					</div>
					<div class="field-row">
						<span class="field-label">{t('agent.settings_base_url')}</span>
						<Input type="text" bind:value={addBaseUrl} placeholder={presetDefaultUrl(addPreset)} />
					</div>
					{#if addError}
						<span class="validate-msg err">{addError}</span>
					{/if}
					<div class="add-actions">
						<Button variant="ghost" size="sm" onclick={() => (addOpen = false)}>
							{t('common.cancel')}
						</Button>
						<Button
							variant="primary"
							size="sm"
							disabled={!addPreset || !addKey.trim() || addBusy}
							onclick={() => void submitAdd()}
						>
							{t('agent.settings_add_provider')}
						</Button>
					</div>
				</div>
			{/if}
		{/if}

		<!-- Tools (design 04 §4.2/§4.3) -->
		<div class="section-header tools-header">
			<div class="setting-info">
				<span class="setting-label">{t('agent.settings_tools')}</span>
				<span class="setting-description">{t('agent.settings_tools_desc')}</span>
			</div>
			<div class="tool-col-headers">
				<span class="col-header">{t('agent.settings_tool_enabled')}</span>
				<span class="col-header">{t('agent.settings_tool_approval')}</span>
			</div>
		</div>

		{#each getToolSettings() as tool (tool.name)}
			<div class="tool-card">
				<div class="tool-row">
					<div class="setting-info">
						<span class="setting-label mono">{tool.name}</span>
						<span class="setting-description">{tOr(`agent.tool_desc_${tool.name}`, tool.description)}</span>
					</div>
					<div class="tool-toggles">
						<div class="toggle-cell">
							<Toggle
								checked={tool.enabled}
								onchange={(c) => saveTool(tool, c, tool.requireApproval, { ...tool.values })}
							/>
						</div>
						<div class="toggle-cell">
							<Toggle
								checked={tool.requireApproval}
								onchange={(c) => saveTool(tool, tool.enabled, c, { ...tool.values })}
							/>
						</div>
					</div>
				</div>

				{#if tool.options.length > 0}
					<div class="tool-options">
						{#each tool.options as opt (opt.key)}
							<div class="option-row">
								<div class="setting-info">
									<span class="option-name mono">{opt.key}</span>
									<span class="setting-description">{tOr(`agent.tool_opt_${tool.name}_${opt.key}`, opt.description)}</span>
								</div>
								<div class="option-control">
									{#if opt.type === 'boolean'}
										<Toggle
											checked={optBool(effectiveValue(tool, opt))}
											onchange={(c) => setToolOption(tool, opt.key, c)}
										/>
									{:else if opt.type === 'number'}
										<input
											class="opt-input"
											type="number"
											value={optNumber(effectiveValue(tool, opt))}
											min={opt.min}
											max={opt.max}
											onchange={(e) => onNumberOption(tool, opt.key, e)}
										/>
									{:else if opt.type === 'enum'}
										<Dropdown
											options={opt.values.map((v) => ({ label: v, value: v }))}
											selected={optString(effectiveValue(tool, opt))}
											onchange={(v) => setToolOption(tool, opt.key, v)}
										/>
									{:else if opt.type === 'string_list'}
										{@const items = optList(effectiveValue(tool, opt))}
										<div class="str-list">
											{#each items as item, i (i)}
												<div class="str-list-row">
													<span class="str-list-value">{item}</span>
													<button
														class="icon-btn small"
														aria-label={t('common.delete')}
														onclick={() => listRemoveItem(tool, opt, i)}
													>
														<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
															<path d="M18 6L6 18M6 6l12 12" />
														</svg>
													</button>
												</div>
											{/each}
											<div class="str-list-add">
												<input
													class="opt-input"
													type="text"
													value={listDrafts[listKey(tool.name, opt.key)] ?? ''}
													oninput={(e) =>
														(listDrafts[listKey(tool.name, opt.key)] = e.currentTarget.value)}
													onkeydown={(e) => {
														if (e.key === 'Enter') {
															e.preventDefault();
															listAddItem(tool, opt);
														}
													}}
												/>
												<button
													class="icon-btn small add"
													aria-label={t('agent.settings_add_provider')}
													onclick={() => listAddItem(tool, opt)}
												>
													<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
														<path d="M12 5v14M5 12h14" />
													</svg>
												</button>
											</div>
										</div>
									{:else}
										<input
											class="opt-input"
											type="text"
											value={optString(effectiveValue(tool, opt))}
											oninput={(e) => setToolOption(tool, opt.key, e.currentTarget.value)}
										/>
									{/if}
								</div>
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/each}

		<!-- Title generation model (design 04 §4.4) -->
		<div class="setting-row">
			<div class="setting-info">
				<span class="setting-label">{t('agent.settings_title_model')}</span>
				<span class="setting-description">{t('agent.settings_title_model_desc')}</span>
			</div>
			<div class="setting-control">
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div
					class="title-dropdown"
					bind:this={titleDdEl}
					onkeydown={(e) => {
						if (e.key === 'Escape') titleOpen = false;
					}}
				>
					<button
						class="dropdown-trigger"
						class:open={titleOpen}
						class:has-value={!!getTitleModel()}
						onclick={toggleTitleDropdown}
						aria-haspopup="listbox"
						aria-expanded={titleOpen}
					>
						<span class="dropdown-text">{titleLabel}</span>
						<svg class="dropdown-chevron" class:open={titleOpen} width="12" height="12" viewBox="0 0 12 12" fill="none">
							<path d="M3 4.5L6 7.5L9 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
						</svg>
					</button>

					{#if titleOpen}
						<ul class="dropdown-list" class:drop-up={titleDropUp} role="listbox">
							<li role="option" aria-selected={!getTitleModel()}>
								<button
									class="dropdown-item-btn"
									class:selected={!getTitleModel()}
									onclick={() => selectTitleModel(null)}
								>
									<span class="item-label">{t('agent.settings_title_model_none')}</span>
									{#if !getTitleModel()}
										<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
											<path d="M2 7L5.5 10.5L12 3.5" stroke="var(--color-accent)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
										</svg>
									{/if}
								</button>
							</li>
							{#each titleGroups as group (group.instance.id)}
								<li class="group-header" aria-hidden="true">{group.instance.name}</li>
								{#each group.models ?? [] as model (model.id)}
									{@const value = `${group.instance.id}/${model.id}`}
									<li role="option" aria-selected={getTitleModel() === value}>
										<button
											class="dropdown-item-btn"
											class:selected={getTitleModel() === value}
											onclick={() => selectTitleModel(value)}
										>
											<span class="item-label">{model.displayName || model.id}</span>
											{#if formatContext(model.contextLength)}
												<span class="ctx-badge">{formatContext(model.contextLength)}</span>
											{/if}
											{#if getTitleModel() === value}
												<svg width="14" height="14" viewBox="0 0 14 14" fill="none">
													<path d="M2 7L5.5 10.5L12 3.5" stroke="var(--color-accent)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
												</svg>
											{/if}
										</button>
									</li>
								{/each}
							{/each}
						</ul>
					{/if}
				</div>
			</div>
		</div>

		<!-- All threads (design 04 §4.5) -->
		<div class="setting-row">
			<div class="setting-info">
				<span class="setting-label">{t('agent.threads')}</span>
			</div>
			<div class="setting-control">
				<Button variant="secondary" size="sm" onclick={openThreadsView}>
					{t('agent.view_all_threads')}
				</Button>
			</div>
		</div>
	</div>
{/if}

<style>
	.tab-content {
		display: flex;
		flex-direction: column;
	}

	.setting-row {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 12px 0;
		border-bottom: 1px solid var(--color-border);
		gap: 24px;
	}

	.setting-row:last-child {
		border-bottom: none;
	}

	.setting-info {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.setting-label {
		font-size: 0.875rem;
		font-weight: 500;
		color: var(--color-text-primary);
	}

	.setting-description {
		font-size: 0.75rem;
		color: var(--color-text-secondary);
	}

	.setting-control {
		flex-shrink: 0;
	}

	.mono {
		font-family: var(--font-mono);
	}

	/* --- Sections --- */

	.section-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 16px;
		padding: 14px 0 8px;
	}

	.section-header:first-child {
		padding-top: 0;
	}

	.locked-hint,
	.empty-hint {
		padding: 12px 0;
		font-size: 0.8125rem;
		color: var(--color-text-secondary);
	}

	/* --- Provider cards --- */

	.provider-card {
		display: flex;
		flex-direction: column;
		padding: 12px;
		margin-bottom: 10px;
		background-color: var(--color-bg-secondary);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-card);
	}

	.provider-head {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.name-input {
		flex: 1;
		min-width: 0;
		padding: 4px 8px;
		font-family: var(--font-sans);
		font-size: 0.875rem;
		font-weight: 500;
		color: var(--color-text-primary);
		background: transparent;
		border: 1px solid transparent;
		border-radius: 6px;
		outline: none;
		transition:
			border-color var(--duration-default) var(--ease-default),
			background-color var(--duration-default) var(--ease-default);
	}

	.name-input:hover {
		border-color: var(--color-border);
	}

	.name-input:focus {
		border-color: var(--color-accent);
		background-color: var(--color-bg-elevated);
	}

	/* Mirrors .name-input metrics so the header doesn't shift entering edit mode. */
	.name-text {
		flex: 1;
		min-width: 0;
		padding: 4px 8px;
		font-size: 0.875rem;
		font-weight: 500;
		color: var(--color-text-primary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		border: 1px solid transparent;
	}

	.preset-badge {
		flex-shrink: 0;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 2px 8px;
		font-size: 0.6875rem;
		font-weight: 500;
		color: var(--color-text-secondary);
		border: 1px solid var(--color-border);
		border-radius: 999px;
	}

	.field-row {
		display: flex;
		flex-direction: column;
		gap: 4px;
		margin-top: 10px;
	}

	.field-label {
		font-size: 0.6875rem;
		font-weight: 500;
		color: var(--color-text-secondary);
	}

	.field-desc {
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}

	.validate-row {
		display: flex;
		align-items: center;
		gap: 10px;
		margin-top: 10px;
	}

	.validate-msg {
		font-size: 0.75rem;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.validate-msg.ok {
		color: var(--color-success);
	}

	.validate-msg.err {
		color: var(--color-danger);
	}

	.models-error {
		margin-top: 8px;
	}

	.models-preview {
		margin-top: 10px;
		max-height: 140px;
		overflow-y: auto;
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
	}

	.model-line {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
		padding: 4px 10px;
		border-bottom: 1px solid var(--color-border);
	}

	.model-line:last-child {
		border-bottom: none;
	}

	.model-name {
		font-size: 0.75rem;
		color: var(--color-text-primary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.model-ctx {
		flex-shrink: 0;
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}

	/* --- Add provider form --- */

	.preset-row {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.preset-chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 4px 12px;
		font-family: var(--font-sans);
		font-size: 0.75rem;
		font-weight: 500;
		color: var(--color-text-secondary);
		background: transparent;
		border: 1px solid var(--color-border);
		border-radius: 999px;
		cursor: pointer;
		transition:
			color var(--duration-default) var(--ease-default),
			border-color var(--duration-default) var(--ease-default),
			background-color var(--duration-default) var(--ease-default);
	}

	.preset-chip:hover {
		color: var(--color-text-primary);
	}

	.preset-chip.selected {
		color: var(--color-accent);
		border-color: var(--color-accent);
		background-color: color-mix(in srgb, var(--color-accent) 12%, transparent);
	}

	.add-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 12px;
	}

	/* --- Icon buttons --- */

	.icon-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		flex-shrink: 0;
		padding: 0;
		color: var(--color-text-secondary);
		background: transparent;
		border: none;
		border-radius: 6px;
		cursor: pointer;
		transition:
			color var(--duration-default) var(--ease-default),
			background-color var(--duration-default) var(--ease-default);
	}

	.icon-btn:hover {
		color: var(--color-text-primary);
		background-color: rgba(255, 255, 255, 0.08);
	}

	.icon-btn.danger:hover {
		color: var(--color-danger);
	}

	.icon-btn.small {
		width: 24px;
		height: 24px;
	}

	.icon-btn.add:hover {
		color: var(--color-accent);
	}

	.icon-btn.save:hover {
		color: var(--color-accent);
	}

	.icon-btn:disabled {
		opacity: 0.4;
		cursor: default;
	}

	/* --- Inline confirm bar --- */

	.confirm-bar {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.confirm-text {
		flex: 1;
		min-width: 0;
		font-size: 0.75rem;
		color: var(--color-text-primary);
	}

	.confirm-btn {
		flex-shrink: 0;
		padding: 4px 10px;
		font-family: var(--font-sans);
		font-size: 0.75rem;
		font-weight: 500;
		color: var(--color-text-primary);
		background: transparent;
		border: 1px solid var(--color-border);
		border-radius: 6px;
		cursor: pointer;
		transition:
			background-color var(--duration-default) var(--ease-default),
			opacity var(--duration-default) var(--ease-default);
	}

	.confirm-btn:hover {
		background-color: rgba(255, 255, 255, 0.08);
	}

	.confirm-btn.danger {
		color: #fff;
		background-color: var(--color-danger);
		border-color: transparent;
	}

	.confirm-btn.danger:hover {
		opacity: 0.85;
	}

	/* --- Tools --- */

	.tools-header {
		border-bottom: 1px solid var(--color-border);
	}

	.tool-col-headers {
		display: flex;
		gap: 16px;
		flex-shrink: 0;
	}

	.col-header {
		width: 44px;
		text-align: center;
		font-size: 0.6875rem;
		font-weight: 500;
		color: var(--color-text-secondary);
	}

	.tool-card {
		padding: 10px 0;
		border-bottom: 1px solid var(--color-border);
	}

	.tool-row {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 24px;
	}

	.tool-toggles {
		display: flex;
		gap: 16px;
		flex-shrink: 0;
	}

	.toggle-cell {
		width: 44px;
		display: flex;
		justify-content: center;
	}

	.tool-options {
		margin-top: 6px;
		padding-left: 8px;
		border-left: 2px solid var(--color-border);
	}

	.option-row {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 16px;
		padding: 6px 0;
	}

	.option-name {
		font-size: 0.8125rem;
		font-weight: 500;
		color: var(--color-text-primary);
	}

	.option-control {
		flex-shrink: 0;
		width: 220px;
		display: flex;
		justify-content: flex-end;
	}

	.opt-input {
		width: 100%;
		padding: 6px 10px;
		font-family: var(--font-sans);
		font-size: 0.8125rem;
		color: var(--color-text-primary);
		background-color: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		outline: none;
		box-sizing: border-box;
		transition: border-color var(--duration-default) var(--ease-default);
	}

	.opt-input:focus {
		border-color: var(--color-accent);
	}

	.opt-input[type='number']::-webkit-inner-spin-button,
	.opt-input[type='number']::-webkit-outer-spin-button {
		-webkit-appearance: none;
		margin: 0;
	}

	.opt-input[type='number'] {
		-moz-appearance: textfield;
		appearance: textfield;
	}

	/* string_list control */
	.str-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
		width: 100%;
	}

	.str-list-row {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.str-list-value {
		flex: 1;
		min-width: 0;
		padding: 5px 10px;
		font-family: var(--font-mono);
		font-size: 0.75rem;
		color: var(--color-text-primary);
		background-color: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.str-list-add {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	/* --- Title model dropdown (grouped, mirrors the composer picker) --- */

	.title-dropdown {
		position: relative;
		width: 240px;
	}

	.dropdown-trigger {
		display: flex;
		align-items: center;
		justify-content: space-between;
		width: 100%;
		padding: 8px 12px;
		font-family: var(--font-sans);
		font-size: 0.8125rem;
		color: var(--color-text-secondary);
		background-color: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		cursor: pointer;
		transition:
			border-color var(--duration-default) var(--ease-default),
			box-shadow var(--duration-default) var(--ease-default);
	}

	.dropdown-trigger.has-value {
		color: var(--color-text-primary);
	}

	.dropdown-trigger.open {
		border-color: var(--color-accent);
		box-shadow: 0 0 0 1px var(--color-accent);
	}

	.dropdown-text {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.dropdown-chevron {
		flex-shrink: 0;
		color: var(--color-text-secondary);
		transition: transform var(--duration-default) var(--ease-default);
	}

	.dropdown-chevron.open {
		transform: rotate(180deg);
	}

	.dropdown-list {
		position: absolute;
		top: calc(100% + 4px);
		left: 0;
		right: 0;
		z-index: 50;
		margin: 0;
		padding: 4px;
		list-style: none;
		background-color: var(--color-bg-elevated);
		border: 1px solid var(--color-border);
		border-radius: var(--radius-btn);
		box-shadow: var(--shadow-elevated);
		max-height: 240px;
		overflow-y: auto;
	}

	.dropdown-list.drop-up {
		top: auto;
		bottom: calc(100% + 4px);
		animation: dropdownInUp var(--duration-default) var(--ease-default);
	}

	@keyframes dropdownInUp {
		from {
			opacity: 0;
			transform: translateY(4px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	.group-header {
		padding: 4px 10px 2px;
		font-size: 0.6875rem;
		font-weight: 600;
		color: var(--color-text-secondary);
		margin-top: 4px;
		border-top: 1px solid var(--color-border);
	}

	.dropdown-item-btn {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 7px 10px;
		font-family: var(--font-sans);
		font-size: 0.8125rem;
		color: var(--color-text-primary);
		background: transparent;
		border: none;
		border-radius: 4px;
		cursor: pointer;
		transition: background-color var(--duration-default) var(--ease-default);
	}

	.dropdown-item-btn:hover {
		background-color: rgba(255, 255, 255, 0.06);
	}

	.dropdown-item-btn.selected {
		color: var(--color-accent);
	}

	.item-label {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		text-align: left;
	}

	.ctx-badge {
		flex-shrink: 0;
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}

	/* --- All threads sub-view --- */

	.threads-header {
		display: flex;
		align-items: center;
		gap: 10px;
		padding-bottom: 10px;
		border-bottom: 1px solid var(--color-border);
	}

	.back-btn {
		display: flex;
		align-items: center;
		gap: 4px;
		padding: 4px 8px;
		margin-left: -8px;
		font-family: var(--font-sans);
		font-size: 0.8125rem;
		font-weight: 500;
		color: var(--color-text-secondary);
		background: transparent;
		border: none;
		border-radius: 6px;
		cursor: pointer;
		transition:
			color var(--duration-default) var(--ease-default),
			background-color var(--duration-default) var(--ease-default);
	}

	.back-btn:hover {
		color: var(--color-text-primary);
		background-color: rgba(255, 255, 255, 0.06);
	}

	.thread-list {
		display: flex;
		flex-direction: column;
	}

	.thread-item {
		position: relative;
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		padding: 8px 6px;
		border-radius: 6px;
		transition: background-color var(--duration-default) var(--ease-default);
	}

	.thread-item::after {
		content: '';
		position: absolute;
		left: 6px;
		right: 6px;
		bottom: 0;
		height: 1px;
		background: var(--color-border);
	}

	.thread-item:last-child::after {
		display: none;
	}

	.thread-item:not(.confirming):hover {
		background-color: rgba(255, 255, 255, 0.04);
	}

	.thread-item.confirming {
		background-color: color-mix(in srgb, var(--color-danger) 10%, transparent);
		box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-danger) 35%, transparent);
	}

	.thread-item .confirm-bar {
		flex-basis: 100%;
		margin-top: 0;
		padding: 8px 2px 0 4px;
		background: transparent;
		border: none;
		border-top: 1px solid color-mix(in srgb, var(--color-danger) 20%, transparent);
	}

	/* Reassign (move-to) bar: accent tint instead of the delete danger tint. */

	.thread-item.moving {
		background-color: color-mix(in srgb, var(--color-accent) 8%, transparent);
		box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 30%, transparent);
	}

	.thread-item .confirm-bar.reassign-bar {
		border-top-color: color-mix(in srgb, var(--color-accent) 20%, transparent);
		flex-wrap: wrap;
		row-gap: 6px;
	}

	.reassign-options {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.confirm-btn.active {
		border-color: var(--color-accent);
		color: var(--color-accent);
	}

	.owner-label.owner-deleted {
		color: var(--color-warning);
	}

	.thread-text {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.thread-title {
		font-size: 0.8125rem;
		font-weight: 500;
		color: var(--color-text-primary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.thread-title.archived {
		color: var(--color-text-secondary);
	}

	.thread-title.untitled {
		color: var(--color-text-secondary);
		font-weight: 400;
	}

	.thread-meta {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 0.6875rem;
		color: var(--color-text-secondary);
	}

	.thread-actions {
		position: absolute;
		right: 6px;
		top: 50%;
		transform: translateY(-50%);
		display: none;
		gap: 2px;
		padding: 2px;
		background-color: var(--color-bg-elevated);
		border-radius: 8px;
	}

	.thread-item:hover .thread-actions {
		display: flex;
	}

	.title-edit {
		width: 100%;
		padding: 3px 8px;
		font-family: var(--font-sans);
		font-size: 0.8125rem;
		font-weight: 500;
		color: var(--color-text-primary);
		background-color: var(--color-bg-elevated);
		border: 1px solid var(--color-accent);
		border-radius: 6px;
		outline: none;
		box-sizing: border-box;
	}
</style>
