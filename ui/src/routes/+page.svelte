<script lang="ts">
	import { health, getGraph } from '$lib/api';
	import StatCard from '$lib/components/StatCard.svelte';
	import type { HealthResponse, GraphDump } from '$lib/types';
	import { features } from '$lib/features';

	let healthData = $state<HealthResponse | null>(null);
	let graphData = $state<GraphDump | null>(null);
	let error = $state('');
	let loading = $state(true);

	$effect(() => {
		loadData();
	});

	async function loadData() {
		loading = true;
		error = '';
		try {
			const [h, g] = await Promise.all([health(), getGraph()]);
			healthData = h;
			graphData = g;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	let entityCount = $derived(graphData?.entities.length ?? 0);
	let activeEdgeCount = $derived(graphData?.edges.active.length ?? 0);
	let invalidatedEdgeCount = $derived(graphData?.edges.invalidated.length ?? 0);
</script>

<div class="page">
	<h1>Overview</h1>

	{#if loading}
		<p class="muted">Loading...</p>
	{:else if error}
		<p class="error">{error}</p>
	{:else}
		<div class="stats-row">
			<StatCard label="Status" value={healthData?.status ?? 'unknown'} />
			<StatCard label="Graph" value={healthData?.graph ?? '-'} />
			<StatCard label="Entities" value={entityCount} />
			<StatCard label="Active Edges" value={activeEdgeCount} />
			<StatCard label="Invalidated" value={invalidatedEdgeCount} />
		</div>

		<div class="section">
			<h2>API Endpoint</h2>
			<code class="endpoint">{features.baseUrl || window.location.origin}</code>
		</div>

		{#if graphData && graphData.entities.length > 0}
			<div class="section">
				<h2>Entity Types</h2>
				<div class="type-list">
					{#each Object.entries(
						graphData.entities.reduce<Record<string, number>>((acc, e) => {
							acc[e.entity_type] = (acc[e.entity_type] || 0) + 1;
							return acc;
						}, {})
					) as [type, count]}
						<div class="type-row">
							<span class="type-name">{type}</span>
							<span class="type-count">{count}</span>
						</div>
					{/each}
				</div>
			</div>
		{/if}
	{/if}
</div>

<style>
	.page { max-width: 900px; }
	h1 {
		font-size: 1.5rem;
		font-weight: 600;
		margin-bottom: 24px;
	}
	h2 {
		font-size: 0.85rem;
		color: #888;
		text-transform: uppercase;
		letter-spacing: 0.03em;
		margin-bottom: 12px;
	}
	.stats-row {
		display: flex;
		gap: 12px;
		flex-wrap: wrap;
		margin-bottom: 32px;
	}
	.section {
		margin-bottom: 32px;
	}
	.endpoint {
		display: inline-block;
		background: #161616;
		border: 1px solid #222;
		border-radius: 6px;
		padding: 8px 14px;
		font-family: 'SF Mono', 'Fira Code', monospace;
		font-size: 0.85rem;
		color: #aaa;
	}
	.type-list {
		background: #161616;
		border: 1px solid #222;
		border-radius: 8px;
		overflow: hidden;
	}
	.type-row {
		display: flex;
		justify-content: space-between;
		padding: 8px 14px;
		border-bottom: 1px solid #1a1a1a;
		font-size: 0.85rem;
	}
	.type-row:last-child { border-bottom: none; }
	.type-name { color: #ccc; }
	.type-count { color: #666; font-variant-numeric: tabular-nums; }
	.muted { color: #555; }
	.error { color: #e55; }
</style>
