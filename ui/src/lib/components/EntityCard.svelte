<script lang="ts">
	import type { Entity } from '$lib/types';

	let { entity, onclick }: {
		entity: Entity;
		onclick?: () => void;
	} = $props();
</script>

<button class="entity-card" onclick={onclick} type="button">
	<div class="header">
		<span class="name">{entity.name}</span>
		<span class="type-badge">{entity.entity_type}</span>
	</div>
	<div class="meta">
		<span class="id">{entity.id.slice(0, 8)}</span>
		{#if entity.hint}
			<span class="hint">{entity.hint}</span>
		{/if}
	</div>
</button>

<style>
	.entity-card {
		display: block;
		width: 100%;
		text-align: left;
		background: #161616;
		border: 1px solid #222;
		border-radius: 8px;
		padding: 12px 16px;
		cursor: pointer;
		color: #e0e0e0;
		font-family: inherit;
		font-size: inherit;
	}
	.entity-card:hover {
		border-color: #444;
		background: #1a1a1a;
	}
	.header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 8px;
	}
	.name {
		font-weight: 600;
		font-size: 0.9rem;
	}
	.type-badge {
		font-size: 0.7rem;
		padding: 2px 8px;
		border-radius: 12px;
		background: #2a2a3a;
		color: #aab;
		white-space: nowrap;
	}
	.meta {
		margin-top: 6px;
		font-size: 0.75rem;
		color: #666;
		display: flex;
		gap: 12px;
	}
</style>
