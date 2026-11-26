<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { invoke } from "@tauri-apps/api/core";

  type Position = "center" | "diagonal" | "tiled";

  type Report = {
    output: string;
    pages: number;
    beforeBytes: number;
    afterBytes: number;
  };

  const POSITIONS: { id: Position; title: string; hint: string }[] = [
    { id: "center", title: "Centered", hint: "One stamp at the middle of each page" },
    { id: "diagonal", title: "Diagonal", hint: "Corner to corner at 45°" },
    { id: "tiled", title: "Tiled", hint: "Repeated across the whole page" },
  ];

  let input = $state<string | null>(null);
  let text = $state("CONFIDENTIAL");
  let position = $state<Position>("center");
  let rotation = $state(0);
  let fontSize = $state(42);
  let opacity = $state(0.2);
  let color = $state("#888888");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let report = $state<Report | null>(null);

  // Diagonal is the 45° stamp its name promises, so the angle input stands
  // down in that mode.
  const diagonal = $derived(position === "diagonal");
  const canRun = $derived(Boolean(input) && text.trim().length > 0 && !busy);

  function basename(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB"];
    let value = bytes / 1024;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${value.toFixed(1)} ${units[unit]}`;
  }

  async function pickInput() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof picked !== "string") return;
    input = picked;
    error = null;
    report = null;
  }

  async function run() {
    if (!input) return;
    error = null;
    report = null;

    const name = basename(input).replace(/\.pdf$/i, "");
    const output = await save({
      defaultPath: `${name}-watermarked.pdf`,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!output) return;

    busy = true;
    try {
      report = await invoke<Report>("watermark_pdf", {
        input,
        output,
        options: {
          text: text.trim(),
          opacity,
          rotation: diagonal ? 45 : rotation,
          position,
          fontSize,
          color,
        },
      });
    } catch (e) {
      error = typeof e === "string" ? e : "Watermarking failed. Please try again.";
    } finally {
      busy = false;
    }
  }
</script>

<section class="page">
  <header>
    <h1>Watermark</h1>
    <p>Stamp text onto every page. Files stay on this computer.</p>
  </header>

  <div class="toolbar">
    <button class="btn" onclick={pickInput} disabled={busy}>Choose PDF</button>
  </div>

  {#if !input}
    <div class="empty">No file yet. Choose a PDF to stamp.</div>
  {:else}
    <div class="source">
      <span class="name" title={input}>{basename(input)}</span>
    </div>

    <div class="fields">
      <label class="field">
        <span class="label">Watermark text</span>
        <input
          class="text"
          type="text"
          bind:value={text}
          oninput={() => (error = null)}
          disabled={busy}
        />
        <span class="hint">
          Chinese text is drawn with a non-embedded font — spacing can vary
          slightly between PDF readers.
        </span>
      </label>

      <fieldset class="positions">
        <legend>Position</legend>
        {#each POSITIONS as option (option.id)}
          <label class="option">
            <input
              type="radio"
              bind:group={position}
              value={option.id}
              disabled={busy}
            />
            <span class="option-body">
              <span class="option-title">{option.title}</span>
              <span class="option-hint">{option.hint}</span>
            </span>
          </label>
        {/each}
      </fieldset>

      <div class="row">
        <label class="field">
          <span class="label">Angle (degrees)</span>
          <input
            class="text"
            type="number"
            min="-360"
            max="360"
            step="5"
            bind:value={rotation}
            disabled={busy || diagonal}
          />
          {#if diagonal}
            <span class="hint">Diagonal runs corner to corner at 45°.</span>
          {/if}
        </label>

        <label class="field">
          <span class="label">Font size (points)</span>
          <input
            class="text"
            type="number"
            min="1"
            max="200"
            step="1"
            bind:value={fontSize}
            disabled={busy}
          />
        </label>
      </div>

      <label class="field">
        <span class="label">Opacity — {Math.round(opacity * 100)}%</span>
        <input
          type="range"
          min="0.05"
          max="1"
          step="0.05"
          bind:value={opacity}
          disabled={busy}
        />
      </label>

      <label class="field">
        <span class="label">Color</span>
        <input class="swatch" type="color" bind:value={color} disabled={busy} />
      </label>
    </div>

    <p class="hint note">
      Stamping rewrites the file container: digital signatures will no longer
      be valid, and the file grows a little. That is expected.
    </p>
  {/if}

  <div class="actions">
    <button class="btn btn-primary" onclick={run} disabled={!canRun}>
      {busy ? "Stamping…" : "Add watermark"}
    </button>
  </div>

  {#if error}
    <p class="status error" role="alert">{error}</p>
  {/if}

  {#if report}
    <div class="status success">
      <p>
        {formatSize(report.beforeBytes)} → {formatSize(report.afterBytes)}
        across {report.pages} {report.pages === 1 ? "page" : "pages"} — the
        stamp adds a little data to every page.
      </p>
      <p class="path" title={report.output}>Saved to {report.output}</p>
    </div>
  {/if}
</section>

<style>
  .page {
    max-width: 720px;
    margin: var(--space-6) auto;
    padding: 0 var(--space-6);
  }

  h1 {
    font-size: var(--text-2xl);
    font-weight: 600;
    letter-spacing: -0.02em;
  }

  header p {
    margin-top: var(--space-2);
    color: var(--color-text-muted);
  }

  .toolbar {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-6);
  }

  .btn {
    padding: var(--space-2) var(--space-4);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--color-surface-raised);
    color: var(--color-text);
    font-size: var(--text-sm);
    font-weight: 500;
    transition: border-color var(--duration-fast) var(--ease-out);
  }

  .btn:hover:not(:disabled) {
    border-color: var(--color-accent);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-primary {
    border-color: transparent;
    background-color: var(--color-accent);
    color: oklch(100% 0 0);
  }

  .btn-primary:hover:not(:disabled) {
    background-color: var(--color-accent-strong);
  }

  .empty {
    margin-top: var(--space-5);
    padding: var(--space-7);
    border: 1px dashed var(--color-border);
    border-radius: var(--radius-lg);
    text-align: center;
    color: var(--color-text-subtle);
  }

  .source {
    margin-top: var(--space-5);
    padding: var(--space-3) var(--space-4);
    background-color: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .name {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }

  .fields {
    margin-top: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1;
  }

  .label {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .text {
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--color-surface-raised);
    color: var(--color-text);
    font-size: var(--text-sm);
  }

  .text:focus-visible {
    outline: none;
    border-color: var(--color-accent);
  }

  .row {
    display: flex;
    gap: var(--space-4);
  }

  .swatch {
    width: 4rem;
    height: 2.25rem;
    padding: 0;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background-color: var(--color-surface-raised);
    cursor: pointer;
  }

  .positions {
    margin-top: var(--space-1);
    border: none;
  }

  legend {
    padding: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .option {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    margin-top: var(--space-3);
    cursor: pointer;
  }

  .option-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .option-title {
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .option-hint {
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
  }

  .hint {
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
    line-height: 1.6;
  }

  .note {
    margin-top: var(--space-4);
  }

  .actions {
    margin-top: var(--space-6);
  }

  .status {
    margin-top: var(--space-4);
    font-size: var(--text-sm);
    word-break: break-all;
  }

  .error {
    color: var(--color-danger);
  }

  .success {
    color: var(--color-success);
  }

  .path {
    margin-top: var(--space-2);
    color: var(--color-text-subtle);
  }
</style>
