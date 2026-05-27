<script lang="ts">
  import "../styles/global.css";
  import { page } from "$app/stores";

  let { children } = $props();

  const isHome = $derived($page.url.pathname === "/");
</script>

<div class="app">
  <header class="topbar" data-home={isHome}>
    <a class="brand" href="/" aria-label="PaperBlade home">
      <span class="brand-mark" aria-hidden="true"></span>
      <span class="brand-name">PaperBlade</span>
    </a>
    {#if !isHome}
      <a class="back" href="/">← All tools</a>
    {/if}
  </header>
  <main class="main">
    {@render children()}
  </main>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background-color: var(--color-surface);
  }

  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-6);
    border-bottom: 1px solid transparent;
    transition: border-color var(--duration-normal) var(--ease-out);
  }

  .topbar:not([data-home="true"]) {
    border-bottom-color: var(--color-border);
  }

  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-text);
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .brand-mark {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    background: linear-gradient(
      135deg,
      var(--color-accent) 0%,
      var(--color-accent-strong) 100%
    );
    transform: rotate(-12deg);
  }

  .brand-name {
    font-size: var(--text-base);
  }

  .back {
    font-size: var(--text-sm);
    color: var(--color-text-muted);
  }

  .back:hover {
    color: var(--color-text);
  }

  .main {
    flex: 1;
    overflow-y: auto;
  }
</style>
