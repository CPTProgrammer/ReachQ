<script module lang="ts">
	export interface Props {
		/** SSH identity this detached window is bound to (from `?agent=`). */
		identity: string;
	}
</script>

<script lang="ts">
	import { onMount } from 'svelte';
	import { initDetachedWindowCloseHook, updatePanelState } from '$lib/state/agent.svelte';
	import AgentPanel from './AgentPanel.svelte';

	let { identity }: Props = $props();

	onMount(() => {
		// A detached window *is* the panel: force it open here. The opener
		// already wrote this state; repeating it covers direct URL opens too.
		updatePanelState(identity, { open: true, detached: true });
		// Closing this window docks the panel back, closed (design 01 §1.2).
		return initDetachedWindowCloseHook(identity);
	});
</script>

<div class="agent-window">
	<AgentPanel {identity} detached />
</div>

<style>
	.agent-window {
		display: flex;
		width: 100vw;
		height: 100vh;
		overflow: hidden;
		background: var(--color-bg-elevated);
	}
</style>
