(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const state = { bots: [], providers: [], threads: {}, active_bot_id: null, tasks: [], submitting: false, editingBot: null };
  const accents = ['#73b8ff', '#c896ff', '#ff9d70', '#5ed6ba', '#f2c766', '#ff7195'];
  const portraitPositions = ['0% 0%', '50% 0%', '100% 0%', '0% 100%', '50% 100%', '100% 100%'];
  function token() { const match = location.hash.match(/(?:^#|&)token=([^&]+)/); if (match) { sessionStorage.setItem('rhk_token', decodeURIComponent(match[1])); history.replaceState(null, '', location.pathname + location.search); } return sessionStorage.getItem('rhk_token') || ''; }
  function headers(json = false) { const h = {}; if (json) h['Content-Type'] = 'application/json'; const t = token(); if (t) h['X-RHK-Token'] = t; return h; }
  async function api(url, options = {}) { const response = await fetch(url, { ...options, headers: { ...headers(Boolean(options.body)), ...(options.headers || {}) }, credentials: 'same-origin' }); let body = null; try { body = await response.json(); } catch {} if (!response.ok) throw new Error(body?.error || `Request failed (${response.status})`); return body; }
  function showError(message) { const el = $('notice'); el.textContent = message; el.hidden = false; setTimeout(() => { el.hidden = true; }, 6000); }
  function activeBot() { return state.bots.find(bot => bot.id === state.active_bot_id) || state.bots[0]; }
  function renderBots() { const list = $('bot-list'); list.replaceChildren(); for (const bot of state.bots) { const item = document.createElement('button'); item.className = `bot-item${bot.id === state.active_bot_id ? ' active' : ''}`; item.type = 'button'; item.onclick = () => { state.active_bot_id = bot.id; render(); }; const avatar = document.createElement('span'); avatar.className = 'bot-avatar'; avatar.style.backgroundImage = "url('portraits.png')"; avatar.style.backgroundSize = '300% 200%'; avatar.style.backgroundPosition = portraitPositions[bot.avatar_id ?? 0]; avatar.style.borderColor = bot.accent_color || accents[0]; const meta = document.createElement('span'); meta.className = 'bot-meta'; const name = document.createElement('strong'); name.textContent = bot.name; const role = document.createElement('span'); role.textContent = bot.role; meta.append(name, role); item.append(avatar, meta); list.append(item); } }
  function renderTranscript() { const transcript = $('transcript'); transcript.replaceChildren(); const messages = state.threads[state.active_bot_id] || []; if (!messages.length) { const empty = document.createElement('div'); empty.className = 'empty-state'; empty.textContent = 'Start a conversation with this bot.'; transcript.append(empty); return; } for (const message of messages) { const row = document.createElement('article'); row.className = `message ${message.role === 'user' ? 'user' : ''}`; const avatar = document.createElement('span'); avatar.className = 'message-avatar'; avatar.textContent = message.role === 'user' ? 'You' : 'R'; const body = document.createElement('div'); body.className = 'message-body'; const header = document.createElement('div'); header.className = 'message-header'; header.textContent = message.role === 'user' ? 'You' : (activeBot()?.name || 'Bot'); const text = document.createElement('div'); text.className = 'message-text'; text.textContent = message.content || ''; body.append(header, text); row.append(avatar, body); transcript.append(row); } transcript.scrollTop = transcript.scrollHeight; }
  function renderContext() {
    const bot = activeBot();
    $('active-bot-name').textContent = bot?.name || 'Welcome';
    $('active-bot-label').textContent = (bot?.name || 'SETUP').toUpperCase();
    $('context-bot-name').textContent = bot?.name || 'No bot selected';
    $('context-bot-role').textContent = bot?.role || 'Add a provider and bot to begin.';
    $('task-count').textContent = state.tasks.length;
    const list = $('task-list'); list.replaceChildren();
    if (!state.tasks.length) { const empty = document.createElement('p'); empty.className = 'empty-state'; empty.textContent = 'No active tasks'; list.append(empty); }
    for (const task of state.tasks) { const row = document.createElement('div'); row.className = 'task'; const status = document.createElement('span'); status.className = 'task-status'; status.textContent = task.status === 'done' ? '✓' : task.status === 'error' ? '!' : '•'; const label = document.createElement('span'); label.textContent = task.status === 'done' ? 'Response complete' : task.status === 'error' ? 'Response failed' : 'Response in progress'; row.append(status, label); list.append(row); }
  }
  function render() { renderBots(); renderTranscript(); renderContext(); }
  function setSubmitting(value) { state.submitting = value; $('send-button').disabled = value; $('message').disabled = value; $('run-status').className = `run-status ${value ? 'running' : 'idle'}`; $('run-status').lastChild.textContent = value ? 'Running' : 'Idle'; }
  async function pollTask(id, botId) {
    const task = { id, status: 'queued' }; state.tasks.push(task); renderContext();
    for (let i = 0; i < 240; i++) {
      await new Promise(resolve => setTimeout(resolve, 500));
      const result = await api('/api/tasks/' + encodeURIComponent(id)); Object.assign(task, result); renderContext();
      if (result.status === 'done') { state.threads[botId] ||= []; state.threads[botId].push({role: 'assistant', content: result.response || ''}); try { const latest = await api('/api/state'); if (latest.threads && !Array.isArray(latest.threads)) state.threads = latest.threads; } catch {} renderTranscript(); return; }
      if (result.status === 'error' || result.status === 'missing') throw new Error(result.error || 'Task is unavailable');
    }
    throw new Error('Response is taking too long. Please try again.');
  }
  async function send(event) {
    event.preventDefault(); const input = $('message'); const message = input.value.trim(); const bot = activeBot();
    if (!message || state.submitting || !bot) return;
    state.threads[bot.id] ||= []; state.threads[bot.id].push({ role: 'user', content: message }); input.value = ''; renderTranscript(); setSubmitting(true);
    try { const result = await api('/api/chat', {method: 'POST', body: JSON.stringify({bot_id: bot.id, message})}); if (!result.task_id) throw new Error('No task returned'); await pollTask(result.task_id, bot.id); }
    catch (e) { showError(e.message); } finally { setSubmitting(false); }
  }
  function renderGallery(bot) { const portraits = $('portrait-gallery'); portraits.replaceChildren(); for (let i = 0; i < 6; i++) { const wrap = document.createElement('div'); wrap.className = 'portrait-option'; const input = document.createElement('input'); input.type = 'radio'; input.name = 'avatar_id'; input.id = `avatar-${i}`; input.value = i; input.checked = (bot.avatar_id ?? 0) === i; const label = document.createElement('label'); label.htmlFor = input.id; label.setAttribute('aria-label', `Portrait ${i + 1}`); const image = document.createElement('span'); image.className = 'portrait'; image.style.setProperty('--pos', portraitPositions[i]); label.append(image); wrap.append(input, label); portraits.append(wrap); } const colors = $('accent-gallery'); colors.replaceChildren(); accents.forEach((color, i) => { const wrap = document.createElement('div'); wrap.className = 'accent-option'; const input = document.createElement('input'); input.type = 'radio'; input.name = 'accent_color'; input.id = `accent-${i}`; input.value = color; input.checked = (bot.accent_color || accents[0]) === color; const label = document.createElement('label'); label.htmlFor = input.id; label.title = `Accent ${i + 1}`; label.setAttribute('aria-label', `Accent color ${i + 1}`); label.style.setProperty('--swatch', color); wrap.append(input, label); colors.append(wrap); }); }
  function settingsError(message = '') { $('settings-error').textContent = message; $('settings-error').hidden = !message; }
  function renderProviders(selected) {
    const select = $('bot-provider'); const current = selected ?? select.value; select.replaceChildren();
    if (!state.providers.length) { const o = document.createElement('option'); o.value = ''; o.textContent = 'Add a provider first'; select.append(o); }
    for (const provider of state.providers) { const option = document.createElement('option'); option.value = provider.id; option.textContent = provider.name; select.append(option); }
    if (current) select.value = current;
  }
  function renderProviderRows() {
    const list = $('provider-list'); list.replaceChildren();
    for (const provider of state.providers) addProviderRow(provider, false);
  }
  function addProviderRow(provider = {}, draft = true) {
    const row = document.createElement('div'); row.className = 'provider-row';
    const field = (label, value, type = 'text') => { const wrap = document.createElement('label'); wrap.textContent = label; const input = document.createElement('input'); input.type = type; input.value = value || ''; wrap.append(input); row.append(wrap); return input; };
    const name = field('Name', provider.name);
    const kindLabel = document.createElement('label'); kindLabel.textContent = 'Provider kind'; const kind = document.createElement('select');
    for (const [value, text] of [['custom','Custom'], ['openai','OpenAI'], ['anthropic','Anthropic']]) { const option = document.createElement('option'); option.value = value; option.textContent = text; kind.append(option); }
    kind.value = provider.kind || 'custom'; kindLabel.append(kind); row.append(kindLabel);
    const endpoint = field('Base URL', provider.endpoint || provider.base_url); endpoint.type = 'url'; endpoint.placeholder = 'https://…';
    const model = field('Default model', provider.model);
    const key = field('API key', '', 'password'); key.autocomplete = 'new-password'; key.placeholder = provider.configured ? 'Configured · leave blank to keep' : 'Write-only';
    const checkLabel = document.createElement('label'); checkLabel.className = 'checkbox-label'; const allow = document.createElement('input'); allow.type = 'checkbox'; allow.checked = !!provider.allow_insecure_http; checkLabel.append(allow, document.createTextNode('Allow HTTP on local networks')); row.append(checkLabel);
    const actions = document.createElement('div'); actions.className = 'settings-actions'; const save = document.createElement('button'); save.type = 'button'; save.className = 'primary-button'; save.textContent = draft ? 'Add provider' : 'Save provider';
    const remove = document.createElement('button'); remove.type = 'button'; remove.className = 'danger-button'; remove.textContent = draft ? 'Cancel' : 'Remove'; actions.append(save, remove); row.append(actions);
    save.onclick = async () => {
      settingsError(); if (!name.value.trim() || !endpoint.value.trim()) { settingsError('Provider name and base URL are required.'); return; }
      const data = {name: name.value.trim(), kind: kind.value, endpoint: endpoint.value.trim(), model: model.value.trim(), allow_insecure_http: allow.checked};
      if (!draft) data.id = provider.id; if (key.value) data.key = key.value;
      save.disabled = true; remove.disabled = true; save.textContent = 'Saving…';
      try { const saved = await api('/api/providers', {method: 'POST', body: JSON.stringify(data)}); key.value = ''; const index = state.providers.findIndex(p => p.id === saved.id); if (index < 0) state.providers.push(saved); else state.providers[index] = saved; renderProviderRows(); renderProviders(saved.id); }
      catch (e) { settingsError(e.message); save.disabled = false; remove.disabled = false; save.textContent = draft ? 'Add provider' : 'Save provider'; }
    };
    remove.onclick = async () => { if (draft) { row.remove(); return; } settingsError(); remove.disabled = true; save.disabled = true; try { await api('/api/providers/' + encodeURIComponent(provider.id), {method:'DELETE'}); state.providers = state.providers.filter(p => p.id !== provider.id); renderProviderRows(); renderProviders(); } catch(e) { settingsError(e.message); remove.disabled = false; save.disabled = false; } };
    $('provider-list').append(row);
  }
  function openSettings(bot = activeBot()) {
    settingsError(); state.editingBot = bot ? {...bot} : null;
    $('bot-settings').hidden = !bot; $('delete-bot').hidden = !bot || !state.bots.some(b => b.id === bot.id);
    if (bot) { $('bot-name').value = bot.name || ''; $('bot-role').value = bot.role || ''; $('bot-model').value = bot.model || ''; $('bot-allow-mutating').checked = !!bot.allow_mutating; renderProviders(bot.provider_id); renderGallery(bot); }
    renderProviderRows(); $('settings-dialog').showModal();
  }
  async function saveBot() {
    const bot = state.editingBot; if (!bot) return;
    const payload = {...bot, name: $('bot-name').value.trim(), role: $('bot-role').value.trim(), model: $('bot-model').value.trim(), provider_id: $('bot-provider').value, allow_mutating: $('bot-allow-mutating').checked, avatar_id: Number(document.querySelector('input[name="avatar_id"]:checked')?.value || 0), accent_color: document.querySelector('input[name="accent_color"]:checked')?.value || accents[0]};
    if (!payload.name || !payload.role || !payload.provider_id || !payload.model) { settingsError('Complete the bot name, role, provider and model.'); return; }
    settingsError(); const button = $('save-bot'); button.disabled = true; button.textContent = 'Saving…';
    try { const result = await api('/api/bots', {method:'POST', body:JSON.stringify(payload)}); const saved = result.bot || result; const index = state.bots.findIndex(b => b.id === saved.id); if (index < 0) state.bots.push(saved); else state.bots[index] = saved; state.active_bot_id = saved.id; $('settings-dialog').close(); render(); }
    catch(e) { settingsError(e.message); } finally { button.disabled = false; button.textContent = 'Save bot'; }
  }
  async function deleteBot() {
    const bot = state.editingBot; if (!bot) return; const button = $('delete-bot'); button.disabled = true; settingsError();
    try { await api('/api/bots/' + encodeURIComponent(bot.id), {method:'DELETE'}); state.bots = state.bots.filter(b => b.id !== bot.id); state.active_bot_id = state.bots[0]?.id || null; $('settings-dialog').close(); render(); }
    catch(e) { settingsError(e.message); } finally { button.disabled = false; }
  }
  async function boot() {
    try { const initial = await api('/api/state'); state.bots = initial.bots || []; state.providers = initial.providers || []; state.threads = initial.threads && !Array.isArray(initial.threads) ? initial.threads : {}; state.active_bot_id = initial.active_bot_id || state.bots[0]?.id || null; render(); $('connection-status').lastChild.textContent = 'Connected'; }
    catch(e) { $('connection-status').lastChild.textContent = 'Offline'; showError(e.message); }
  }
  $('composer').addEventListener('submit', send); $('settings-button').onclick = () => openSettings(); $('save-bot').onclick = saveBot; $('delete-bot').onclick = deleteBot; $('add-provider').onclick = () => addProviderRow();
  $('context-toggle').onclick = () => { const panel = $('context-panel'); const open = panel.classList.toggle('open'); $('context-toggle').setAttribute('aria-expanded', String(open)); };
  $('close-context').onclick = () => { $('context-panel').classList.remove('open'); $('context-toggle').setAttribute('aria-expanded', 'false'); };
  $('message').addEventListener('keydown', event => { if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); $('composer').requestSubmit(); } });
  $('add-bot').onclick = () => openSettings({name: '', role: 'RHK scripting assistant', provider_id: state.providers[0]?.id || '', model: state.providers[0]?.model || '', avatar_id: 0, accent_color: accents[0], allow_mutating: false});
  $('bot-provider').addEventListener('change', () => { const provider = state.providers.find(p => p.id === $('bot-provider').value); if (provider && !$('bot-model').value) $('bot-model').value = provider.model || ''; });

    boot();
})();
