<script lang="ts">
	import type { Entity, Edge } from '$lib/types';
	import { getEntityEdges, deleteEntity } from '$lib/api';
	import EdgeTable from './EdgeTable.svelte';

	let { entity, onclose, ondelete }: {
		entity: Entity;
		onclose: () => void;
		ondelete?: (id: string) => void;
	} = $props();

	let edges = $state<Edge[]>([]);
	let loading = $state(true);
	let error = $state('');

	$effect(() => {
		loadEdges(entity.id);
	});

	async function loadEdges(id: string) {
		loading = true;
		error = '';
		try {
			edges = await getEntityEdges(id);
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	async function handleDelete() {
		if (!confirm(`Delete entity "${entity.name}" and invalidate all its edges?`)) return;
		try {
			await deleteEntity(entity.id);
			ondelete?.(entity.id);
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
	}
</script>

<div class="detail-panel">
	<div class="panel-header">
		<h3>{entity.name}</h3>
		<button class="close-btn" onclick={onclose}>x</button>
	</div>

	<div class="meta-grid">
		<div class="meta-item">
			<span class="meta-label">Type</span>
			<span class="meta-value">{entity.entity_type}</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">ID</span>
			<span class="meta-value mono">{entity.id}</span>
		</div>
		<div class="meta-item">
			<span class="meta-label">Resolved</span>
			<span class="meta-value">{entity.resolved ? 'Yes' : 'No'}</span>
		</div>
		{#if entity.hint}
			<div class="meta-item">
				<span class="meta-label">Hint</span>
				<span class="meta-value">{entity.hint}</span>
			</div>
		{/if}
		<div class="meta-item">
			<span class="meta-label">Created</span>
			<span class="meta-value">{new Date(entity.created_at).toLocaleString()}</span>
		</div>
	</div>

	<h4>Edges</h4>
	{#if loading}
		<p class="muted">Loading edges...</p>
	{:else if error}
		<p class="error">{error}</p>
	{:else}
		<EdgeTable {edges} />
	{/if}

	<div class="actions">
		<button class="danger-btn" onclick={handleDelete}>Delete Entity</button>
	</div>
</div>

<style>
	.detail-panel {
		background: #111;
		border-left: 1px solid #222;
		padding: 20px;
		height: 100%;
		overflow-y: auto;
	}
	.panel-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin-bottom: 16px;
	}
	h3 {
		font-size: 1.1rem;
		font-weight: 600;
	}
	h4 {
		font-size: 0.85rem;
		color: #888;
		text-transform: uppercase;
		letter-spacing: 0.03em;
		margin: 20px 0 8px;
	}
	.close-btn {
		background: none;
		border: none;
		color: #666;
		font-size: 1.2rem;
		cursor: pointer;
		padding: 4px 8px;
	}
	.close-btn:hover { color: #ccc; }
	.meta-grid {
		display: grid;
		gap: 8px;
	}
	.meta-item {
		display: flex;
		gap: 8px;
	}
	.meta-label {
		font-size: 0.75rem;
		color: #666;
		min-width: 60px;
	}
	.meta-value {
		font-size: 0.825rem;
		color: #ccc;
	}
	.mono {
		font-family: 'SF Mono', 'Fira Code', monospace;
		font-size: 0.75rem;
	}
	.muted { color: #555; font-size: 0.85rem; }
	.error { color: #e55; font-size: 0.85rem; }
	.actions {
		margin-top: 24px;
		padding-top: 16px;
		border-top: 1px solid #222;
	}
	.danger-btn {
		background: #3a1111;
		border: 1px solid #622;
		color: #e88;
		padding: 6px 14px;
		border-radius: 6px;
		cursor: pointer;
		font-size: 0.8rem;
	}
	.danger-btn:hover {
		background: #4a1515;
	}
</style>
