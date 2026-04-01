<script lang="ts">
	import { features } from '$lib/features';
	import { health } from '$lib/api';
	import type { HealthResponse } from '$lib/types';

	let healthData = $state<HealthResponse | null>(null);
	let loading = $state(true);
	let error = $state('');

	$effect(() => {
		loadSettings();
	});

	async function loadSettings() {
		loading = true;
		error = '';
		try {
			healthData = await health();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}
</script>

<div class="page">
	<h1>Settings</h1>

	{#if loading}
		<p class="muted">Loading...</p>
	{:else if error}
		<p class="error">{error}</p>
	{:else}
		<div class="section">
			<h2>Instance</h2>
			<div class="settings-grid">
				<div class="setting-row">
					<span class="setting-label">Mode</span>
					<span class="setting-value">{features.mode}</span>
				</div>
				<div class="setting-row">
					<span class="setting-label">API URL</span>
					<span class="setting-value mono">{features.baseUrl || window.location.origin}</span>
				</div>
				<div class="setting-row">
					<span class="setting-label">Status</span>
					<span class="setting-value">{healthData?.status ?? 'unknown'}</span>
				</div>
				<div class="setting-row">
					<span class="setting-label">Graph Backend</span>
					<span class="setting-value">{healthData?.graph ?? '-'}</span>
				</div>
			</div>
		</div>

		<div class="section">
			<h2>Features</h2>
			<div class="settings-grid">
				{#each Object.entries(features.features) as [name, enabled]}
					<div class="setting-row">
						<span class="setting-label">{name}</span>
						<span class="setting-value" class:enabled class:disabled={!enabled}>
							{enabled ? 'Enabled' : 'Disabled'}
						</span>
					</div>
				{/each}
			</div>
		</div>

		{#if features.mode === 'cloud' && features.features.byok}
			<div class="section">
				<h2>BYOK (Bring Your Own Key)</h2>
				<p class="desc">Configure your own LLM API key for this tenant.</p>
				<div class="byok-form">
					<input type="password" placeholder="LLM API key" disabled />
					<button disabled>Save</button>
				</div>
				<p class="muted">Coming soon</p>
			</div>
		{/if}

		{#if features.mode === 'cloud'}
			<div class="section danger-zone">
				<h2>Danger Zone</h2>
				<button class="danger-btn" disabled>Delete Account</button>
				<p class="muted">This action is irreversible.</p>
			</div>
		{/if}
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
	.settings-grid {
		background: #161616; border: 1px solid #222;
		border-radius: 8px; overflow: hidden;
	}
	.setting-row {
		display: flex; justify-content: space-between;
		padding: 10px 16px; border-bottom: 1px solid #1a1a1a;
		font-size: 0.85rem;
	}
	.setting-row:last-child { border-bottom: none; }
	.setting-label { color: #888; }
	.setting-value { color: #ccc; }
	.setting-value.enabled { color: #4ea; }
	.setting-value.disabled { color: #666; }
	.mono { font-family: 'SF Mono', monospace; font-size: 0.8rem; }
	.desc { font-size: 0.85rem; color: #666; margin-bottom: 12px; }
	.byok-form {
		display: flex; gap: 8px; margin-bottom: 8px;
	}
	.byok-form input {
		flex: 1; background: #111; border: 1px solid #333; color: #ccc;
		padding: 8px 12px; border-radius: 6px; font-size: 0.85rem;
	}
	.byok-form button {
		background: #333; color: #ccc; border: 1px solid #444;
		padding: 8px 16px; border-radius: 6px; cursor: pointer;
	}
	.danger-zone {
		border: 1px solid #422; border-radius: 8px;
		padding: 20px; background: #160a0a;
	}
	.danger-btn {
		background: #3a1111; border: 1px solid #622; color: #e88;
		padding: 8px 16px; border-radius: 6px; cursor: pointer;
		font-size: 0.85rem; margin-bottom: 8px;
	}
	.danger-btn:disabled { opacity: 0.5; cursor: not-allowed; }
	.muted { color: #555; font-size: 0.8rem; }
	.error { color: #e55; font-size: 0.85rem; }
</style>
