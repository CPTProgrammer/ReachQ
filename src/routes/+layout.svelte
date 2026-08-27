<script lang="ts">
	import type { Snippet } from 'svelte';
	import '../app.css';
	import AppShell from '$lib/components/layout/AppShell.svelte';
	import WelcomeScreen from '$lib/components/setup/WelcomeScreen.svelte';
	import { loadSettings, getSettings, syncTraySettings } from '$lib/state/settings.svelte';
	import { agentMigrateLegacySettings } from '$lib/ipc/agent';
	import { loadProviders, loadModels } from '$lib/state/agent-settings.svelte';
	import { initPanelStorageSync } from '$lib/state/agent.svelte';
	import { initShortcuts, cleanupShortcuts } from '$lib/state/shortcuts.svelte';
	import { startupUpdateCheck, startPeriodicChecks, stopPeriodicChecks } from '$lib/state/updater.svelte';
	import { changeLocale } from '$lib/state/i18n.svelte';
	import { loadSnippets } from '$lib/state/snippets.svelte';
	import { vaultState } from '$lib/state/vault.svelte';
	import { onMount } from 'svelte';

	let { children }: { children: Snippet } = $props();

	const isEditorWindow = typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('editor');
	// Detached agent panel window (`?agent=<identity>`, design 01 §1.2).
	const isAgentWindow = typeof window !== 'undefined' && !!new URLSearchParams(window.location.search).get('agent');
	const settings = getSettings();

	onMount(() => {
		initPanelStorageSync();
		loadSettings();
		Promise.all([
			syncTraySettings(),
		]).then(() => {
			const preloader = document.getElementById('preloader');
			if (preloader) {
				preloader.classList.add('hidden');
				setTimeout(() => preloader.remove(), 400);
			}
		});
		initShortcuts();
		// startupUpdateCheck();
		// startPeriodicChecks();

		return () => {
			cleanupShortcuts();
			stopPeriodicChecks();
		};
	});

	$effect(() => {
		changeLocale(settings.locale);
	});

	// Load snippets once vault is unlocked
	$effect(() => {
		if (!vaultState.locked) {
			loadSnippets();
			// Agent bootstrap: one-time legacy AI settings migration (idempotent,
			// decided backend-side), then provider/model lists. Failures are
			// silent (e.g. pure-browser dev mode without Tauri).
			void (async () => {
				try { await agentMigrateLegacySettings(); } catch { /* silent */ }
				try { await loadProviders(); } catch { /* silent */ }
				try { await loadModels(); } catch { /* silent */ }
			})();
		}
	});

	$effect(() => {
		document.documentElement.style.setProperty('--app-font-size', `${settings.fontSize}px`);
	});

	$effect(() => {
		const theme = settings.theme;
		const root = document.documentElement;
		root.classList.remove('dark', 'light');

		if (theme === 'system') {
			const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
			root.classList.add(prefersDark ? 'dark' : 'light');
		} else {
			root.classList.add(theme);
		}
	});
</script>

{#if isEditorWindow || isAgentWindow}
	{@render children()}
{:else}
	{#if !settings.setupComplete}
		<WelcomeScreen />
	{/if}

	<AppShell>
		{@render children()}
	</AppShell>
{/if}
