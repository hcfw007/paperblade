<script lang="ts">
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { invoke } from "@tauri-apps/api/core";

  let input = $state<string | null>(null);
  let locked = $state<boolean | null>(null);
  let password = $state("");
  let confirmation = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);
  let resultPath = $state<string | null>(null);

  // The file decides the job: a plain PDF gets locked, a locked one gets opened.
  const mode = $derived(locked === null ? null : locked ? "decrypt" : "encrypt");
  const needsConfirmation = $derived(mode === "encrypt");
  const mismatch = $derived(
    needsConfirmation && confirmation.length > 0 && password !== confirmation,
  );
  const ready = $derived(
    Boolean(input) &&
      password.length > 0 &&
      (!needsConfirmation || password === confirmation) &&
      !busy,
  );

  function basename(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  function suggestedName(): string {
    if (!input) return "output.pdf";
    const name = basename(input).replace(/\.pdf$/i, "");
    return mode === "encrypt" ? `${name}-locked.pdf` : `${name}-unlocked.pdf`;
  }

  async function pickInput() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (typeof picked !== "string") return;

    error = null;
    resultPath = null;
    password = "";
    confirmation = "";
    input = picked;
    locked = null;
    try {
      locked = await invoke<boolean>("is_encrypted", { input: picked });
    } catch (e) {
      error = typeof e === "string" ? e : "Could not read that PDF.";
      input = null;
    }
  }

  async function run() {
    if (!input || !mode) return;
    error = null;
    resultPath = null;

    const output = await save({
      defaultPath: suggestedName(),
      filters: [{ name: "PDF", extensions: ["pdf"] }],
    });
    if (!output) return;

    busy = true;
    try {
      const command = mode === "encrypt" ? "encrypt_pdf" : "decrypt_pdf";
      resultPath = await invoke<string>(command, { input, output, password });
      password = "";
      confirmation = "";
    } catch (e) {
      error = typeof e === "string" ? e : "Something went wrong. Please try again.";
    } finally {
      busy = false;
    }
  }
</script>

<section class="page">
  <header>
    <h1>Encrypt &amp; Decrypt</h1>
    <p>Add or remove a PDF password. Files stay on this computer.</p>
  </header>

  <div class="toolbar">
    <button class="btn" onclick={pickInput} disabled={busy}>Choose PDF</button>
  </div>

  {#if !input}
    <div class="empty">No file yet. Choose a PDF to lock or unlock.</div>
  {:else}
    <div class="source">
      <span class="name" title={input}>{basename(input)}</span>
      <span class="badge" data-locked={locked}>
        {locked ? "Password-protected" : "Not protected"}
      </span>
    </div>

    <div class="fields">
      <label class="field">
        <span class="label">
          {mode === "encrypt" ? "New password" : "Password"}
        </span>
        <input
          class="text"
          type="password"
          bind:value={password}
          oninput={() => (error = null)}
          disabled={busy}
          autocomplete="off"
        />
      </label>

      {#if needsConfirmation}
        <label class="field">
          <span class="label">Confirm password</span>
          <input
            class="text"
            type="password"
            bind:value={confirmation}
            oninput={() => (error = null)}
            disabled={busy}
            autocomplete="off"
            aria-invalid={mismatch}
          />
        </label>
        {#if mismatch}
          <p class="hint warn">The two passwords do not match.</p>
        {:else}
          <p class="hint">
            There is no way to recover this password. Keep it somewhere safe.
          </p>
        {/if}
      {/if}
    </div>
  {/if}

  <div class="actions">
    <button class="btn btn-primary" onclick={run} disabled={!ready}>
      {#if busy}
        {mode === "encrypt" ? "Locking…" : "Unlocking…"}
      {:else}
        {mode === "decrypt" ? "Remove password" : "Add password"}
      {/if}
    </button>
  </div>

  {#if error}
    <p class="status error" role="alert">{error}</p>
  {/if}
  {#if resultPath}
    <p class="status success">Saved to {resultPath}</p>
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
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-5);
    padding: var(--space-3) var(--space-4);
    background-color: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm);
  }

  .badge {
    flex: none;
    padding: 0.15rem var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
  }

  .badge[data-locked="true"] {
    border-color: var(--color-accent);
    color: var(--color-accent);
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

  .text[aria-invalid="true"] {
    border-color: var(--color-danger);
  }

  .hint {
    margin-top: calc(-1 * var(--space-2));
    color: var(--color-text-subtle);
    font-size: var(--text-sm);
  }

  .warn {
    color: var(--color-danger);
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
</style>
