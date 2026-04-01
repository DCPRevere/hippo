<script lang="ts">
	import { features } from '$lib/features';
	import FeatureGate from '$lib/components/FeatureGate.svelte';
</script>

<div class="page">
	<FeatureGate feature="billing">
		{#snippet children()}
			<h1>Billing</h1>

			<div class="section">
				<h2>Current Plan</h2>
				<div class="plan-card">
					<div class="plan-name">Free</div>
					<p class="plan-desc">Community plan with basic features.</p>
					<button class="upgrade-btn" disabled>Upgrade</button>
				</div>
			</div>

			<div class="section">
				<h2>Usage This Period</h2>
				<p class="muted">Usage-based billing details will appear here.</p>
			</div>
		{/snippet}
	</FeatureGate>

	{#if !features.features.billing}
		<h1>Billing</h1>
		<p class="muted">Billing is not available in standalone mode.</p>
	{/if}
</div>

<style>
	.page { max-width: 700px; }
	h1 { font-size: 1.5rem; font-weight: 600; margin-bottom: 24px; }
	h2 {
		font-size: 0.85rem; color: #888; text-transform: uppercase;
		letter-spacing: 0.03em; margin-bottom: 12px;
	}
	.section { margin-bottom: 32px; }
	.plan-card {
		background: #161616; border: 1px solid #222;
		border-radius: 8px; padding: 20px;
	}
	.plan-name {
		font-size: 1.3rem; font-weight: 600; color: #fff;
		margin-bottom: 4px;
	}
	.plan-desc {
		font-size: 0.85rem; color: #666; margin-bottom: 16px;
	}
	.upgrade-btn {
		background: #4e9af5; color: #fff; border: none;
		padding: 8px 20px; border-radius: 6px; cursor: pointer;
		font-size: 0.85rem;
	}
	.upgrade-btn:disabled { opacity: 0.5; cursor: not-allowed; }
	.muted { color: #555; font-size: 0.85rem; }
</style>
