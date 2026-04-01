<script lang="ts">
	import { features } from '$lib/features';
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';

	let { children }: { children: Snippet } = $props();

	let sidebarOpen = $state(true);

	const navItems = $derived([
		{ href: '/', label: 'Overview', show: true },
		{ href: '/graph', label: 'Graph', show: true },
		{ href: '/ask', label: 'Ask', show: true },
		{ href: '/users', label: 'Users', show: true },
		{ href: '/usage', label: 'Usage', show: features.features.usage },
		{ href: '/settings', label: 'Settings', show: true },
		{ href: '/billing', label: 'Billing', show: features.features.billing }
	]);

	function isActive(href: string): boolean {
		if (href === '/') return page.url.pathname === '/';
		return page.url.pathname.startsWith(href);
	}

	function logout() {
		localStorage.removeItem('hippo_api_key');
		window.location.href = '/login';
	}

	function toggleSidebar() {
		sidebarOpen = !sidebarOpen;
	}
</script>

{#if page.url.pathname === '/login'}
	{@render children()}
{:else}
	<div class="app-layout" class:sidebar-collapsed={!sidebarOpen}>
		<button class="mobile-toggle" onclick={toggleSidebar}>
			{sidebarOpen ? 'Close' : 'Menu'}
		</button>

		<aside class="sidebar" class:open={sidebarOpen}>
			<div class="sidebar-header">
				<a href="/" class="logo">hippo</a>
			</div>
			<nav class="sidebar-nav">
				{#each navItems as item}
					{#if item.show}
						<a
							href={item.href}
							class="nav-item"
							class:active={isActive(item.href)}
						>
							{item.label}
						</a>
					{/if}
				{/each}
			</nav>
			<div class="sidebar-footer">
				<button class="nav-item logout-btn" onclick={logout}>Logout</button>
			</div>
		</aside>

		<main class="main-content">
			{@render children()}
		</main>
	</div>
{/if}

<style>
	.app-layout {
		display: flex;
		min-height: 100vh;
	}
	.sidebar {
		width: 200px;
		min-width: 200px;
		background: #0f0f0f;
		border-right: 1px solid #1a1a1a;
		display: flex;
		flex-direction: column;
		height: 100vh;
		position: sticky;
		top: 0;
	}
	.sidebar-header {
		padding: 20px 16px;
		border-bottom: 1px solid #1a1a1a;
	}
	.logo {
		font-size: 1.3rem;
		font-weight: 700;
		color: #fff;
		letter-spacing: -0.02em;
	}
	.sidebar-nav {
		flex: 1;
		padding: 8px 0;
		display: flex;
		flex-direction: column;
	}
	.nav-item {
		display: block;
		padding: 8px 16px;
		font-size: 0.85rem;
		color: #888;
		border: none;
		background: none;
		text-align: left;
		cursor: pointer;
		font-family: inherit;
	}
	.nav-item:hover {
		color: #ccc;
		background: #161616;
	}
	.nav-item.active {
		color: #fff;
		background: #1a1a1a;
		border-right: 2px solid #4e9af5;
	}
	.sidebar-footer {
		border-top: 1px solid #1a1a1a;
		padding: 8px 0;
	}
	.logout-btn {
		color: #666;
		width: 100%;
	}
	.logout-btn:hover {
		color: #e88;
	}
	.main-content {
		flex: 1;
		padding: 24px 32px;
		overflow-y: auto;
		min-height: 100vh;
	}
	.mobile-toggle {
		display: none;
		position: fixed;
		top: 12px;
		left: 12px;
		z-index: 100;
		background: #222;
		border: 1px solid #333;
		color: #ccc;
		padding: 6px 12px;
		border-radius: 4px;
		cursor: pointer;
		font-size: 0.8rem;
	}

	@media (max-width: 768px) {
		.mobile-toggle {
			display: block;
		}
		.sidebar {
			position: fixed;
			left: -200px;
			z-index: 50;
			transition: left 0.2s ease;
		}
		.sidebar.open {
			left: 0;
		}
		.main-content {
			padding: 24px 16px;
			padding-top: 48px;
		}
	}
</style>
