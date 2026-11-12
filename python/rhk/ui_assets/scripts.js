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
    <div class="scripts-footer"><span id="scripts-state" class="muted">New draft</span><button id="scripts-save" class="primary-button" type="button">Save draft</button></div><p id="scripts-status" role="status" aria-live="polite"></p></section></div>`;
  document.body.append(dialog);
  const $ = id => document.getElementById('scripts-' + id);
  let revision = null, loadedKey = null, dirty = false, busy = false;
  function setBusy(value) { busy = value; for (const name of ['name', 'extension', 'text', 'save']) $(name).disabled = value; }
  const message = (text, error = false) => { $('status').textContent = text; $('status').classList.toggle('error', error); };
  async function api(path, options = {}) {
    const headers = { 'X-RHK-Token': sessionStorage.getItem('rhk_token') || '' };
    if (options.body) headers['Content-Type'] = 'application/json';
    const response = await fetch(path, { ...options, headers, credentials: 'same-origin' });
    const body = await response.json();
    if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
    return body;
  }
  function changed() { dirty = true; $('state').textContent = 'Unsaved changes'; }
  function discard() { return !busy && (!dirty || window.confirm('Discard unsaved changes?')); }
  function reset() {
    if (!discard()) return;
    revision = loadedKey = null; dirty = false;
    $('name').value = ''; $('extension').value = 'py'; $('text').value = '';
    $('state').textContent = 'New draft'; message(''); $('name').focus();
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
      await refresh();
    } catch (error) { message(error.message + ' Your editor contents have been kept.', true); }
    finally { setBusy(false); }
  }
  open.onclick = async () => { dialog.showModal(); try { await refresh(); } catch (error) { message(error.message, true); } };
  $('new').onclick = reset; $('save').onclick = save;
  $('refresh').onclick = () => refresh().catch(error => message(error.message, true));
  $('close').onclick = () => { if (discard()) dialog.close(); };
  dialog.addEventListener('cancel', event => { if (!discard()) event.preventDefault(); });
  for (const name of ['name', 'extension', 'text']) $(name).addEventListener('input', changed);
  $('text').addEventListener('keydown', event => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); save(); } });
})();
