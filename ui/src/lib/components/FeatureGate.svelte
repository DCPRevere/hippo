<script lang="ts">
	import { features } from '$lib/features';
	import type { Snippet } from 'svelte';

	let { feature, children }: {
		feature: keyof typeof features.features;
		children: Snippet;
	} = $props();
</script>

{#if features.features[feature]}
	{@render children()}
{/if}
