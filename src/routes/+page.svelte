<script lang="ts">
  import { TOOLS } from "$lib/tools";
</script>

<section class="hero">
  <h1 class="title">A real desktop PDF toolkit.</h1>
  <p class="subtitle">
    All the power. None of the docker-compose. Your files never leave this
    computer.
  </p>
</section>

<section class="grid" aria-label="Tools">
  {#each TOOLS as tool (tool.id)}
    {#if tool.available}
      <a class="card" href={tool.href} data-tool={tool.id}>
        <div class="card-body">
          <h2 class="card-title">{tool.title}</h2>
          <p class="card-subtitle">{tool.subtitle}</p>
        </div>
        <span class="card-arrow" aria-hidden="true">→</span>
      </a>
    {:else}
      <div class="card card-disabled" data-tool={tool.id} aria-disabled="true">
        <div class="card-body">
          <h2 class="card-title">{tool.title}</h2>
          <p class="card-subtitle">{tool.subtitle}</p>
        </div>
        <span class="card-soon">Soon</span>
      </div>
    {/if}
  {/each}
</section>

<style>
  .hero {
    max-width: 760px;
    margin: var(--space-7) auto var(--space-6);
    padding: 0 var(--space-6);
    text-align: center;
  }

  .title {
    font-size: var(--text-3xl);
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--color-text);
  }

  .subtitle {
    margin-top: var(--space-3);
    color: var(--color-text-muted);
    font-size: var(--text-base);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-4);
    max-width: 960px;
    margin: 0 auto var(--space-7);
    padding: 0 var(--space-6);
  }

  .card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-5);
    background-color: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    color: var(--color-text);
    transition:
      transform var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out),
      box-shadow var(--duration-fast) var(--ease-out);
  }

  .card:hover {
    transform: translateY(-2px);
    border-color: var(--color-border-strong);
    box-shadow: var(--shadow-md);
  }

  .card-body {
    min-width: 0;
  }

  .card-title {
    font-size: var(--text-lg);
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .card-subtitle {
    margin-top: var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .card-arrow {
    color: var(--color-text-subtle);
    font-size: var(--text-lg);
    transition: transform var(--duration-fast) var(--ease-out);
  }

  .card:hover .card-arrow {
    transform: translateX(2px);
    color: var(--color-accent);
  }

  .card-disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .card-disabled:hover {
    transform: none;
    border-color: var(--color-border);
    box-shadow: none;
  }

  .card-soon {
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--color-text-subtle);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    padding: 2px 6px;
  }
</style>
