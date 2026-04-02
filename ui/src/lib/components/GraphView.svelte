<script lang="ts">
	import { untrack } from 'svelte';
	import cytoscape from 'cytoscape';
	import type { Core, LayoutOptions } from 'cytoscape';
	import type { GraphDump, Entity, Edge } from '$lib/types';

	let { data, onSelectNode, onSelectEdge }: {
		data: GraphDump;
		onSelectNode?: (entity: Entity) => void;
		onSelectEdge?: (edge: Edge) => void;
	} = $props();

	let container: HTMLDivElement;
	let cy: Core | null = null;
	let currentLayout = $state<string>('cose');

	const entityTypeColors: Record<string, string> = {
		person: '#4e9af5',
		organization: '#f59e4e',
		place: '#4ef5a2',
		event: '#f54e9a',
		concept: '#a24ef5',
		thing: '#f5e44e'
	};

	function colorForType(type: string): string {
		const key = type.toLowerCase();
		return entityTypeColors[key] || '#888';
	}

	function buildElements(graphData: GraphDump) {
		const nodes = graphData.entities.map((e) => ({
			data: {
				id: e.id,
				label: e.name,
				entityType: e.entity_type,
				color: colorForType(e.entity_type),
				entity: e
			}
		}));

		const entityIds = new Set(graphData.entities.map((e) => e.id));
		const edges = graphData.edges.active
			.filter((e) => entityIds.has(e.subject_id) && entityIds.has(e.object_id))
			.map((e) => ({
				data: {
					id: `e-${e.edge_id}`,
					source: e.subject_id,
					target: e.object_id,
					label: e.relation_type,
					confidence: e.confidence,
					edge: e
				}
			}));

		return [...nodes, ...edges];
	}

	function getLayoutOptions(name: string): LayoutOptions {
		switch (name) {
			case 'circle':
				return { name: 'circle', padding: 40 };
			case 'grid':
				return { name: 'grid', padding: 40 };
			case 'concentric':
				return { name: 'concentric', padding: 40 };
			default:
				return {
					name: 'cose',
					animate: false,
					padding: 40,
					nodeRepulsion: () => 8000,
					idealEdgeLength: () => 120,
					nodeOverlap: 20
				} as LayoutOptions;
		}
	}

	function initCytoscape(graphData: GraphDump) {
		if (cy) cy.destroy();

		cy = cytoscape({
			container,
			elements: buildElements(graphData),
			style: [
				{
					selector: 'node',
					style: {
						'background-color': 'data(color)',
						label: 'data(label)',
						color: '#ccc',
						'font-size': '11px',
						'text-valign': 'bottom',
						'text-margin-y': 6,
						width: 28,
						height: 28,
						'border-width': 2,
						'border-color': '#333'
					}
				},
				{
					selector: 'node:selected',
					style: {
						'border-color': '#fff',
						'border-width': 3
					}
				},
				{
					selector: 'edge',
					style: {
						width: 'mapData(confidence, 0, 1, 1, 5)',
						'line-color': '#444',
						'target-arrow-color': '#444',
						'target-arrow-shape': 'triangle',
						'curve-style': 'bezier',
						label: 'data(label)',
						color: '#666',
						'font-size': '9px',
						'text-rotation': 'autorotate',
						'text-margin-y': -8
					} as any
				},
				{
					selector: 'edge:selected',
					style: {
						'line-color': '#888',
						'target-arrow-color': '#888'
					}
				}
			],
			layout: getLayoutOptions(currentLayout),
			wheelSensitivity: 0.3
		});

		cy.on('tap', 'node', (evt) => {
			const entity = evt.target.data('entity') as Entity;
			if (entity && onSelectNode) onSelectNode(entity);
		});

		cy.on('tap', 'edge', (evt) => {
			const edge = evt.target.data('edge') as Edge;
			if (edge && onSelectEdge) onSelectEdge(edge);
		});
	}

	function applyLayout(name: string) {
		currentLayout = name;
		if (cy) {
			cy.layout(getLayoutOptions(name)).run();
		}
	}

	function fitGraph() {
		cy?.fit(undefined, 40);
	}

	$effect(() => {
		if (container && data) {
			untrack(() => initCytoscape(data));
		}
		return () => {
			cy?.destroy();
			cy = null;
		};
	});
</script>

<div class="graph-controls">
	<div class="layout-buttons">
		<button class:active={currentLayout === 'cose'} onclick={() => applyLayout('cose')}>Force</button>
		<button class:active={currentLayout === 'circle'} onclick={() => applyLayout('circle')}>Circle</button>
		<button class:active={currentLayout === 'grid'} onclick={() => applyLayout('grid')}>Grid</button>
		<button class:active={currentLayout === 'concentric'} onclick={() => applyLayout('concentric')}>Concentric</button>
	</div>
	<button class="fit-btn" onclick={fitGraph}>Fit</button>
</div>
<div class="graph-container" bind:this={container}></div>

<style>
	.graph-container {
		width: 100%;
		height: 100%;
		min-height: 400px;
		background: #0d0d0d;
		border: 1px solid #222;
		border-radius: 8px;
	}
	.graph-controls {
		display: flex;
		justify-content: space-between;
		align-items: center;
		margin-bottom: 8px;
	}
	.layout-buttons {
		display: flex;
		gap: 4px;
	}
	.layout-buttons button, .fit-btn {
		background: #1a1a1a;
		border: 1px solid #333;
		color: #999;
		padding: 4px 12px;
		border-radius: 4px;
		font-size: 0.75rem;
		cursor: pointer;
	}
	.layout-buttons button:hover, .fit-btn:hover {
		background: #252525;
		color: #ccc;
	}
	.layout-buttons button.active {
		background: #2a2a2a;
		border-color: #555;
		color: #eee;
	}
</style>
