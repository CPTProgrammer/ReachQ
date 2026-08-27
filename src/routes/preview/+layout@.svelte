<script lang="ts">
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';
	import '../../app.css';

	let { children }: { children: Snippet } = $props();

	onMount(() => {
		document.documentElement.classList.add('dark');
		return () => document.documentElement.classList.remove('dark');
	});
</script>

<div class="preview-shell">
	<header class="preview-header">
		<span class="brand">Reach · Component Preview</span>
		<nav class="preview-nav">
			<a href="/preview">Index</a>
			<a href="/preview/icons">Distro Icons</a>
			<a href="/preview/agent">AI Agent</a>
		</nav>
	</header>
	<main class="preview-main">
		{@render children()}
	</main>
</div>

<style>
	.preview-shell {
		height: 100%;
		display: flex;
		flex-direction: column;
		background-color: var(--color-bg-primary);
		color: var(--color-text-primary);
	}

	.preview-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 12px 24px;
		border-bottom: 1px solid var(--color-border);
		background-color: var(--color-bg-secondary);
		flex-shrink: 0;
	}

	.brand {
		font-size: 0.8125rem;
		font-weight: 600;
	}

	.preview-nav {
		display: flex;
		gap: 16px;
	}

	.preview-nav a {
		font-size: 0.8125rem;
		color: var(--color-text-secondary);
		text-decoration: none;
	}

	.preview-nav a:hover {
		color: var(--color-text-primary);
	}

	.preview-main {
		flex: 1;
		overflow-y: auto;
		padding: 24px;
	}
</style>
