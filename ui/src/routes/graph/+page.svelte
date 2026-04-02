<script lang="ts">
	import { onMount } from 'svelte';
	import { getGraph, listGraphs } from '$lib/api';
	import GraphView from '$lib/components/GraphView.svelte';
	import EntityDetail from '$lib/components/EntityDetail.svelte';
	import EdgeDetail from '$lib/components/EdgeDetail.svelte';
	import SearchBar from '$lib/components/SearchBar.svelte';
	import type { GraphDump, Entity, Edge } from '$lib/types';

	let graphData = $state<GraphDump | null>(null);
	let filteredData = $state<GraphDump | null>(null);
	let graphs = $state<string[]>([]);
	let selectedGraph = $state<string>('');
	let selectedEntity = $state<Entity | null>(null);
	let selectedEdge = $state<Edge | null>(null);
	let filterText = $state('');
	let loading = $state(true);
	let error = $state('');

	onMount(() => {
		loadGraphList();
	});

	async function loadGraphList() {
		try {
			const resp = await listGraphs();
			graphs = resp.graphs;
			selectedGraph = resp.default;
			await loadGraph();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
			loading = false;
		}
	}

	async function loadGraph() {
		loading = true;
		error = '';
		try {
			graphData = await getGraph(selectedGraph || undefined);
			applyFilter();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	function applyFilter() {
		if (!graphData) {
			filteredData = null;
			return;
		}
		if (!filterText.trim()) {
			filteredData = graphData;
			return;
		}
		const q = filterText.toLowerCase();
		const matchedEntities = graphData.entities.filter(
			(e) =>
				e.name.toLowerCase().includes(q) ||
				e.entity_type.toLowerCase().includes(q) ||
				e.id.toLowerCase().includes(q)
		);
		const entityIds = new Set(matchedEntities.map((e) => e.id));
		// Also include entities that are connected to matched entities via matching edges
		const matchedEdges = graphData.edges.active.filter(
			(e) =>
				e.fact.toLowerCase().includes(q) ||
				entityIds.has(e.subject_id) ||
				entityIds.has(e.object_id)
		);
		// Add connected entities
		for (const edge of matchedEdges) {
			entityIds.add(edge.subject_id);
			entityIds.add(edge.object_id);
		}
		const allMatchedEntities = graphData.entities.filter((e) => entityIds.has(e.id));
		filteredData = {
			graph: graphData.graph,
			entities: allMatchedEntities,
			edges: {
				active: matchedEdges,
				invalidated: []
			}
		};
	}

	function handleSelectNode(entity: Entity) {
		selectedEdge = null;
		selectedEntity = entity;
	}

	function handleSelectEdge(edge: Edge) {
		selectedEntity = null;
		selectedEdge = edge;
	}

	function handleEntityDelete(_id: string) {
		selectedEntity = null;
		loadGraph();
	}

	function handleGraphChange(e: Event) {
		const target = e.target as HTMLSelectElement;
		selectedGraph = target.value;
		loadGraph();
	}
</script>

<div class="page">
	<div class="page-header">
		<h1>Graph Explorer</h1>
		{#if graphs.length > 1}
			<select class="graph-select" bind:value={selectedGraph} onchange={handleGraphChange}>
				{#each graphs as g}
					<option value={g}>{g}</option>
				{/each}
			</select>
		{/if}
	</div>

	<div class="filter-bar">
		<SearchBar bind:value={filterText} placeholder="Filter by name, type, or fact..." onsubmit={applyFilter} />
	</div>

	{#if loading}
		<p class="muted">Loading graph...</p>
	{:else if error}
		<p class="error">{error}</p>
	{:else if filteredData}
		<div class="graph-layout">
			<div class="graph-main">
				<GraphView
					data={filteredData}
					onSelectNode={handleSelectNode}
					onSelectEdge={handleSelectEdge}
				/>
			</div>
			{#if selectedEntity}
				<div class="detail-sidebar">
					<EntityDetail
						entity={selectedEntity}
						onclose={() => (selectedEntity = null)}
						ondelete={handleEntityDelete}
					/>
				</div>
			{:else if selectedEdge}
				<div class="detail-sidebar">
					<EdgeDetail
						edge={selectedEdge}
						onclose={() => (selectedEdge = null)}
					/>
				</div>
			{/if}
		</div>

		<div class="graph-stats">
			{filteredData.entities.length} entities, {filteredData.edges.active.length} edges
			{#if filterText}
				(filtered)
			{/if}
		</div>
	{/if}
</div>

<style>
	.page { height: calc(100vh - 48px); display: flex; flex-direction: column; }
	.page-header {
		display: flex;
		align-items: center;
		gap: 16px;
		margin-bottom: 12px;
	}
	h1 {
		font-size: 1.5rem;
		font-weight: 600;
	}
	.graph-select {
		background: #161616;
		border: 1px solid #333;
		color: #ccc;
		padding: 6px 10px;
		border-radius: 6px;
		font-size: 0.825rem;
	}
	.filter-bar {
		margin-bottom: 12px;
	}
	.graph-layout {
		flex: 1;
		display: flex;
		gap: 0;
		min-height: 0;
	}
	.graph-main {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
	}
	.detail-sidebar {
		width: 340px;
		min-width: 340px;
	}
	.graph-stats {
		padding: 8px 0;
		font-size: 0.75rem;
		color: #555;
	}
	.muted { color: #555; }
	.error { color: #e55; }
</style>
