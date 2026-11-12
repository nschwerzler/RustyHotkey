(() => {
  'use strict';
  const actions = document.querySelector('.top-actions');
  if (!actions) return;
  const open = document.createElement('button');
  open.id = 'scripts-open'; open.type = 'button'; open.className = 'ghost-button'; open.textContent = 'Scripts';
  actions.prepend(open);
  const dialog = document.createElement('dialog');
  dialog.id = 'scripts-dialog'; dialog.className = 'scripts-dialog'; dialog.setAttribute('aria-labelledby', 'scripts-title');
  dialog.innerHTML = `<header class="scripts-heading"><div><p class="eyebrow">SCRIPT WORKSPACE</p><h2 id="scripts-title">Scripts &amp; widgets</h2></div><button id="scripts-close" class="icon-button" type="button" aria-label="Close scripts">×</button></header>
    <p class="muted">Edit Python, Lua and JSON drafts. Saving checks syntax where supported and never runs a script.</p>
    <div class="scripts-body"><aside class="scripts-sidebar"><button id="scripts-new" class="secondary-button" type="button">New draft</button><button id="scripts-refresh" class="ghost-button" type="button">Refresh list</button><div id="scripts-list" class="scripts-list" aria-label="Saved scripts"></div></aside>
    <section class="scripts-editor"><div class="scripts-fields"><label>Name<input id="scripts-name" placeholder="my-script" maxlength="64" autocomplete="off"></label><label>Type<select id="scripts-extension"><option value="py">Python (.py)</option><option value="lua">Lua widget (.lua)</option><option value="json">JSON (.json)</option></select></label></div>
    <label class="sr-only" for="scripts-text">Script content</label><textarea id="scripts-text" spellcheck="false" autocapitalize="off" autocomplete="off" placeholder="Paste a draft or ask Master to create one."></textarea>
    <div class="scripts-footer"><span id="scripts-state" class="muted">New draft</span><button id="scripts-save" class="primary-button" type="button">Save draft</button></div><p id="scripts-status" role="status" aria-live="polite"></p>
    <section class="scripts-runner" aria-labelledby="scripts-runner-title"><div class="scripts-runner-heading"><div><h3 id="scripts-runner-title">Operator run</h3><p class="muted">Runs the saved Python revision on this computer. Drafts never run automatically.</p></div><button id="scripts-run" class="primary-button" type="button" disabled>Run saved Python</button></div><p id="scripts-run-note" class="muted">Load a saved Python draft to enable running.</p><p id="scripts-run-result" role="status" aria-live="polite"></p><div id="scripts-runs" class="scripts-runs" aria-label="Script run history"></div></section></section></div>`;
  document.body.append(dialog);
  const $ = id => document.getElementById('scripts-' + id);
  let revision = null, loadedKey = null, dirty = false, busy = false, runBusy = false, runsLoading = false, pollTimer = null, lastRunsSignature = '';
  function updateRunControl() {
    const currentKey = `${$('name').value.trim()}.${$('extension').value}`;
    const eligible = $('extension').value === 'py' && !!revision && loadedKey === currentKey && !dirty;
    $('run').disabled = !eligible || busy || runBusy;
    $('run-note').textContent = eligible ? 'This saved revision is ready to run. Launches require this operator action.' : 'Only a loaded, saved Python revision with no unsaved changes can run. Save changes first.';
  }
  function setBusy(value) { busy = value; for (const name of ['name', 'extension', 'text', 'save']) $(name).disabled = value; updateRunControl(); }
  const message = (text, error = false) => { $('status').textContent = text; $('status').classList.toggle('error', error); };
  async function api(path, options = {}) {
    const headers = { 'X-RHK-Token': sessionStorage.getItem('rhk_token') || '' };
    if (options.body) headers['Content-Type'] = 'application/json';
    const response = await fetch(path, { ...options, headers, credentials: 'same-origin' });
    const body = await response.json();
    if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
    return body;
  }
  function changed() { dirty = true; $('state').textContent = 'Unsaved changes'; updateRunControl(); }
  function discard() { return !busy && (!dirty || window.confirm('Discard unsaved changes?')); }
  function reset() {
    if (!discard()) return;
    revision = loadedKey = null; dirty = false;
    $('name').value = ''; $('extension').value = 'py'; $('text').value = '';
    $('state').textContent = 'New draft'; message(''); updateRunControl(); $('name').focus();
  }
  async function read(item) {
    if (!discard()) return;
    setBusy(true);
    try {
      const value = await api(`/api/scripts/${encodeURIComponent(item.id)}.${item.extension}`);
      $('name').value = value.id; $('extension').value = value.extension; $('text').value = value.text;
      revision = value.revision; loadedKey = `${value.id}.${value.extension}`; dirty = false;
      $('state').textContent = `Saved · ${revision.slice(0, 8)}`;
      message(value.extension === 'lua' ? 'Lua text saved as a draft. Load it explicitly to validate its widget behavior.' : 'Loaded saved draft.');
      updateRunControl();
    } catch (error) { message(error.message, true); } finally { setBusy(false); }
  }
  async function refresh() {
    const result = await api('/api/scripts'); const list = $('list'); list.replaceChildren();
    for (const item of result.scripts) {
      const button = document.createElement('button'); button.type = 'button'; button.className = 'script-file';
      button.textContent = `${item.id}.${item.extension}`; button.onclick = () => read(item); list.append(button);
    }
    if (!result.scripts.length) { const empty = document.createElement('p'); empty.className = 'muted'; empty.textContent = 'No saved drafts yet.'; list.append(empty); }
  }
  async function save() {
    if (busy) return;
    const id = $('name').value.trim(), extension = $('extension').value, text = $('text').value;
    if (!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(id)) { message('Use a name of 1–64 letters, digits, underscores or hyphens.', true); return; }
    const payload = { id, extension, text };
    if (`${id}.${extension}` === loadedKey && revision) payload.expected_revision = revision;
    setBusy(true);
    try {
      const value = await api('/api/scripts', { method: 'POST', body: JSON.stringify(payload) });
      revision = value.revision; loadedKey = `${value.id}.${value.extension}`; dirty = false;
      $('state').textContent = `Saved · ${revision.slice(0, 8)}`;
      message(extension === 'lua' ? 'Lua draft saved. It has not been loaded or run.' : 'Syntax checked and saved. Nothing was run.');
      updateRunControl();
      await refresh();
    } catch (error) { message(error.message + ' Your editor contents have been kept.', true); }
    finally { setBusy(false); }
  }
  function renderRuns(runs) {
    if (!dialog.open) return;
    const safeRuns = Array.isArray(runs) ? [...runs].sort((a, b) => {
      const activeA = a.state === 'running' || a.state === 'stopping';
      const activeB = b.state === 'running' || b.state === 'stopping';
      if (activeA !== activeB) return activeA ? -1 : 1;
      const startedA = Date.parse(a.started_at || '') || 0;
      const startedB = Date.parse(b.started_at || '') || 0;
      return startedB - startedA;
    }).slice(0, 20) : [];
    const signature = JSON.stringify(safeRuns);
    if (signature === lastRunsSignature) return;
    lastRunsSignature = signature;
    const list = $('runs'); list.replaceChildren();
    if (!safeRuns.length) { const empty = document.createElement('p'); empty.className = 'muted'; empty.textContent = 'No script runs yet.'; list.append(empty); return; }
    for (const run of safeRuns) {
      const card = document.createElement('article'); card.className = 'script-run';
      const heading = document.createElement('div'); heading.className = 'script-run-heading';
      const title = document.createElement('strong'); title.textContent = `${run.script_id || 'Python script'} · ${run.state || 'unknown'}`;
      const runId = document.createElement('span'); runId.className = 'muted'; runId.textContent = `Run ${String(run.id || '').slice(0, 12)}`;
      heading.append(title, runId); card.append(heading);
      const outcome = document.createElement('p'); outcome.className = 'muted';
      outcome.textContent = run.exit_code === null || run.exit_code === undefined ? `Exit code: pending` : `Exit code: ${run.exit_code}`;
      card.append(outcome);
      const lines = Array.isArray(run.output) ? run.output.slice(-12).map(chunk => {
        const stream = chunk && chunk.stream === 'stderr' ? 'stderr' : 'stdout';
        const text = chunk && typeof chunk.text === 'string' ? chunk.text : '';
        return `[${stream}] ${text.slice(0, 2400)}`;
      }) : [];
      if (run.output_truncated) lines.unshift('[output truncated]');
      if (lines.length) { const output = document.createElement('pre'); output.className = 'script-run-output'; output.textContent = lines.join('\n'); card.append(output); }
      if (run.state === 'running' || run.state === 'stopping') {
        const stop = document.createElement('button'); stop.type = 'button'; stop.className = 'secondary-button';
        stop.textContent = run.state === 'stopping' ? 'Stopping…' : 'Stop run'; stop.disabled = run.state === 'stopping';
        stop.setAttribute('aria-label', `Stop run ${String(run.id || '').slice(0, 12)}`);
        stop.onclick = () => stopRun(run.id, stop); card.append(stop);
      }
      list.append(card);
    }
  }
  async function refreshRuns() {
    if (!dialog.open || runsLoading) return;
    runsLoading = true;
    try {
      const result = await api('/api/script-runs');
      if (dialog.open) renderRuns(result.runs);
    } catch (error) {
      if (dialog.open) { lastRunsSignature = ''; $('runs').textContent = ''; const status = document.createElement('p'); status.className = 'error'; status.textContent = `Unable to load script runs: ${error.message}`; $('runs').append(status); }
    } finally { runsLoading = false; }
  }
  async function runSaved() {
    updateRunControl();
    if ($('run').disabled) return;
    runBusy = true; updateRunControl();
    $('run-result').textContent = 'Starting saved revision…'; $('run-result').classList.remove('error');
    try {
      const receipt = await api('/api/script-runs', { method: 'POST', body: JSON.stringify({ id: $('name').value.trim(), expected_revision: revision }) });
      $('run-result').textContent = `Run started · ${String(receipt.id || '').slice(0, 12)}. The editor remains a saved draft.`;
      await refreshRuns();
    } catch (error) { $('run-result').textContent = `Run could not start: ${error.message}`; $('run-result').classList.add('error'); }
    finally { runBusy = false; updateRunControl(); }
  }
  async function stopRun(runId, button) {
    if (!runId || button.disabled) return;
    button.disabled = true; button.textContent = 'Stopping…';
    try { await api(`/api/script-runs/${encodeURIComponent(runId)}/stop`, { method: 'POST', body: '{}' }); await refreshRuns(); }
    catch (error) { button.disabled = false; button.textContent = 'Stop run'; $('run-result').textContent = `Could not stop run: ${error.message}`; $('run-result').classList.add('error'); }
  }
  function startPolling() { stopPolling(); refreshRuns(); pollTimer = window.setInterval(refreshRuns, 2000); }
  function stopPolling() { if (pollTimer !== null) { window.clearInterval(pollTimer); pollTimer = null; } }
  open.onclick = async () => { dialog.showModal(); startPolling(); try { await refresh(); } catch (error) { message(error.message, true); } };
  $('new').onclick = reset; $('save').onclick = save;
  $('run').onclick = runSaved;
  $('refresh').onclick = () => refresh().catch(error => message(error.message, true));
  $('close').onclick = () => { if (discard()) dialog.close(); };
  dialog.addEventListener('close', stopPolling);
  dialog.addEventListener('cancel', event => { if (!discard()) event.preventDefault(); });
  for (const name of ['name', 'extension', 'text']) $(name).addEventListener('input', changed);
  $('extension').addEventListener('change', updateRunControl);
  $('text').addEventListener('keydown', event => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); save(); } });
})();
