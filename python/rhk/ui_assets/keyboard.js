(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const dialog = $('keyboard-dialog');
  const layout = $('keyboard-layout');
  const surface = $('keyboard-surface');
  const sampleScenario = {
    schema_version: 1,
    profile: {
      schema_version: 1,
      target: {window_class: 'ExampleWindow', window_title: null},
      controls: {abort: 112, reset: 113, pause: 114, resume: 115},
      tick_ms: 20,
      rules: []
    },
    frames: [{
      at_ms: 0,
      observation: {target_present: true, focused: true, keys_down: [], pixels: [], clipboard: '', cursor: {x: 0, y: 0}},
      operations: [{op: 'arm'}, {op: 'execute', actions: [{type: 'tap', key: 65}]}]
    }]
  };
  const keys = new Map();
  const localHeld = new Set();
  const playbackHeld = new Set();
  const MAX_PLAYBACK_ROWS = 600;
  const MIN_EVENT_DWELL_MS = 120;
  let remoteHeld = [], cursor = 0, gapCount = 0, timer = null, polling = false, demoRunning = false, simulationRunning = false;
  const playback = {items: [], index: 0, timer: null, playing: false, speed: 1, dueAt: 0, scheduledAt: 0, scheduledVirtual: 0, scheduledFloor: 0, delay: 0, remainingVirtual: 0, remainingFloor: 0, currentAt: 0, currentRow: null, truncated: false, focus: null};
  const rows = [
    [['Escape','Esc'],...Array.from({length:12},(_,i)=>[`F${i+1}`,`F${i+1}`]),['PrintScreen','PrtSc'],['ScrollLock','ScrLk'],['Pause','Pause']],
    [['Backquote','`'],...Array.from('1234567890',(x)=>[`Digit${x}`,x]),['Minus','−'],['Equal','='],['Backspace','Backspace',2],['Insert','Ins'],['Home','Home'],['PageUp','PgUp']],
    [['Tab','Tab',1.5],...Array.from('QWERTYUIOP',x=>[`Key${x}`,x]),['BracketLeft','['],['BracketRight',']'],['Backslash','\\',1.5],['Delete','Del'],['End','End'],['PageDown','PgDn']],
    [['CapsLock','Caps',1.8],...Array.from('ASDFGHJKL',x=>[`Key${x}`,x]),['Semicolon',';'],['Quote',"'"],['Enter','Enter',2.2]],
    [['ShiftLeft','Shift',2.3],...Array.from('ZXCVBNM',x=>[`Key${x}`,x]),['Comma',','],['Period','.'],['Slash','/'],['ShiftRight','Shift',2.3],['ArrowUp','↑']],
    [['ControlLeft','Ctrl',1.4],['MetaLeft','Win',1.2],['AltLeft','Alt',1.2],['Space','Space',6],['AltRight','Alt',1.2],['MetaRight','Win',1.2],['ContextMenu','Menu',1.2],['ControlRight','Ctrl',1.4],['ArrowLeft','←'],['ArrowDown','↓'],['ArrowRight','→']]
  ];
  function cap(code, label, weight, parent) {
    const el = document.createElement('span'); el.className = 'keycap'; el.dataset.key = code; el.textContent = label; el.title = code;
    if (weight) el.style.setProperty('--weight', weight); parent.append(el); keys.set(code, el);
  }
  for (const row of rows) { const line = document.createElement('div'); line.className = 'keyboard-row'; for (const [code, label, weight] of row) cap(code, label, weight, line); layout.append(line); }
  const numpad = $('keyboard-numpad');
  for (const [code,label] of [['NumLock','Num'],['NumpadDivide','÷'],['NumpadMultiply','×'],['NumpadSubtract','−'],['Numpad7','7'],['Numpad8','8'],['Numpad9','9'],['NumpadAdd','+'],['Numpad4','4'],['Numpad5','5'],['Numpad6','6'],['NumpadEnter','Enter'],['Numpad1','1'],['Numpad2','2'],['Numpad3','3'],['Numpad0','0'],['NumpadDecimal','.']]) cap(code,label,0,numpad);
  const mouse = $('keyboard-mouse'); const heading = document.createElement('p'); heading.className = 'mouse-title'; heading.textContent = 'MOUSE'; mouse.append(heading);
  for (const [code, label] of [['Mouse1','Left'],['Mouse2','Right'],['Mouse3','Middle'],['Mouse4','Back'],['Mouse5','Forward']]) cap(code, label, 0, mouse);
  function sourceLabel(source) { source = String(source || 'unknown'); if (source === 'local') return 'local page'; if (source === 'demo') return 'demo (simulated)'; if (source === 'scenario') return 'scenario playback (simulated)'; if (source.startsWith('sim:')) return source.slice(4) + ' (simulated)'; if (source.startsWith('live:')) return source.slice(5) + ' (live script)'; return source; }
  function allHeld() { return [...remoteHeld, ...Array.from(localHeld, key => ({source:'local',key})), ...Array.from(playbackHeld, key => ({source:'scenario',key}))]; }
  function paint() {
    const held = allHeld(); const active = new Set(held.map(x => String(x.key)));
    for (const [code, el] of keys) el.dataset.held = String(active.has(code));
    $('keyboard-held-count').textContent = held.length;
    $('keyboard-held').textContent = held.length ? held.map(x => `${x.key} · ${sourceLabel(x.source)}`).join(', ') : 'No keys held';
  }
  function flash(key) { const el = keys.get(String(key)); if (!el) return; el.classList.remove('released'); void el.offsetWidth; el.classList.add('released'); setTimeout(() => el.classList.remove('released'), 650); }
  function timeline(event) {
    const item = document.createElement('li');
    for (const [value, cls] of [[event.kind, 'event-kind'], [event.key || '—', ''], [sourceLabel(event.source), ''], [event.at_ms ? new Date(event.at_ms).toLocaleTimeString() : new Date().toLocaleTimeString(), 'event-time']]) { const span = document.createElement('span'); span.textContent = String(value); if (cls) span.className = cls; item.append(span); }
    const list = $('keyboard-timeline'); list.prepend(item); while (list.children.length > 150) list.lastElementChild.remove();
    if (event.kind === 'up') flash(event.key);
  }
  async function request(url, options = {}) {
    const headers = {'X-RHK-Token': sessionStorage.getItem('rhk_token') || ''}; if (options.body) headers['Content-Type'] = 'application/json';
    const response = await fetch(url, {...options, headers, credentials:'same-origin'}); let body = {}; try { body = await response.json(); } catch {}
    if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`); return body;
  }
  function playbackKey(event) {
    if (event.type === 'button' && Number.isInteger(event.button) && event.button >= 1 && event.button <= 5) return `Mouse${event.button}`;
    if (event.type !== 'key' || !Number.isInteger(event.key)) return null;
    const key = event.key;
    if (key >= 65 && key <= 90) return `Key${String.fromCharCode(key)}`;
    if (key >= 48 && key <= 57) return `Digit${String.fromCharCode(key)}`;
    if (key >= 112 && key <= 135) return `F${key - 111}`;
    if (key >= 96 && key <= 105) return `Numpad${key - 96}`;
    return ({8:'Backspace',9:'Tab',13:'Enter',16:'ShiftLeft',17:'ControlLeft',18:'AltLeft',19:'Pause',20:'CapsLock',27:'Escape',32:'Space',33:'PageUp',34:'PageDown',35:'End',36:'Home',37:'ArrowLeft',38:'ArrowUp',39:'ArrowRight',40:'ArrowDown',44:'PrintScreen',45:'Insert',46:'Delete',91:'MetaLeft',92:'MetaRight',93:'ContextMenu',106:'NumpadMultiply',107:'NumpadAdd',109:'NumpadSubtract',110:'NumpadDecimal',111:'NumpadDivide',144:'NumLock',145:'ScrollLock',160:'ShiftLeft',161:'ShiftRight',162:'ControlLeft',163:'ControlRight',164:'AltLeft',165:'AltRight',186:'Semicolon',187:'Equal',188:'Comma',189:'Minus',190:'Period',191:'Slash',192:'Backquote',219:'BracketLeft',220:'Backslash',221:'BracketRight',222:'Quote'})[key] || `VK_${key.toString(16).toUpperCase().padStart(2,'0')}`;
  }
  function playbackEventLabel(event) {
    if (event.type === 'key' || event.type === 'button') return `${playbackKey(event) || 'Input'} ${event.down ? 'down' : 'up'}`;
    if (event.type === 'wheel') return `Wheel ${event.horizontal ? 'horizontal' : 'vertical'} event`;
    if (event.type === 'text') return 'Text action · contents omitted';
    if (event.type === 'move') return 'Pointer move · not displayed';
    return `${String(event.type || 'Unknown')} event`;
  }
  function playbackFocusLabel(focus) {
    if (!focus) return 'Focus unknown';
    return `${focus.focused ? 'Focused' : 'Unfocused'} · target ${focus.target_present ? 'present' : 'absent'}`;
  }
  function updatePlaybackButton() {
    $('simulation-play-toggle').textContent = playback.playing ? 'Pause playback' : (playback.index >= playback.items.length && playback.items.length ? 'Replay scenario' : 'Play scenario');
  }
  function prefersReducedMotion() { return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches; }
  function resetPlayback(clearRows = true) {
    if (playback.timer !== null) clearTimeout(playback.timer);
    playback.timer = null; playback.playing = false; playback.index = 0; playback.currentAt = playback.items[0]?.at_ms || 0;
    playback.remainingVirtual = 0; playback.remainingFloor = 0; playback.focus = null; playback.currentRow = null;
    playbackHeld.clear(); paint();
    if (clearRows) $('simulation-play-timeline').replaceChildren();
    $('simulation-play-status').textContent = playback.items.length ? 'Ready to play the returned trace.' : 'This scenario has no timeline rows to play.';
    updatePlaybackButton();
  }
  function finishPlayback() {
    if (playback.timer !== null) clearTimeout(playback.timer);
    playback.timer = null; playback.playing = false;
    playbackHeld.clear(); paint();
    $('simulation-play-status').textContent = playback.truncated
      ? `Playback complete. All simulated keys and buttons were released. The display was limited to the first ${MAX_PLAYBACK_ROWS} timeline rows.`
      : 'Playback complete. All simulated keys and buttons were released.';
    updatePlaybackButton();
  }
  function renderPlaybackItem(item) {
    const row = document.createElement('li'); row.dataset.kind = item.kind;
    if (item.kind === 'frame') {
      row.className = `frame-marker${item.focus.focused ? ' focused' : ' unfocused'}`;
      row.textContent = `Frame ${item.frame} · T+${item.at_ms} ms · ${playbackFocusLabel(item.focus)}`;
      playback.focus = {...item.focus, frame:item.frame};
    } else {
      const key = playbackKey(item.event);
      if (key && item.event.down) playbackHeld.add(key);
      else if (key && !item.event.down) { playbackHeld.delete(key); flash(key); }
      row.className = `event-marker${item.event.down ? ' down' : item.event.down === false ? ' up' : ''}`;
      row.textContent = `Event frame ${item.frame} · T+${item.at_ms} ms · ${playbackEventLabel(item.event)} · Observation frame ${playback.focus?.frame ?? 'unknown'} · ${playbackFocusLabel(playback.focus)}`;
      paint();
    }
    if (playback.currentRow) playback.currentRow.removeAttribute('aria-current');
    row.setAttribute('aria-current','step'); playback.currentRow = row;
    const list = $('simulation-play-timeline'); list.append(row);
    while (list.children.length > MAX_PLAYBACK_ROWS) list.firstElementChild.remove();
    list.scrollTop = list.scrollHeight;
    playback.currentAt = item.at_ms;
    $('simulation-play-status').textContent = `${playback.playing ? 'Playing' : 'Paused'} · ${playback.speed}× · T+${item.at_ms} ms · ${item.kind === 'event' ? `Event frame ${item.frame}` : `Frame ${item.frame}`} · Observation frame ${playback.focus?.frame ?? 'unknown'} · ${playbackFocusLabel(playback.focus)}`;
  }
  function schedulePlayback(virtualRemaining, floorRemaining = 0) {
    if (!playback.playing || playback.index >= playback.items.length) { finishPlayback(); return; }
    const speed = Number($('simulation-play-speed').value) || 1;
    playback.speed = speed; playback.remainingVirtual = virtualRemaining; playback.remainingFloor = floorRemaining;
    playback.scheduledVirtual = virtualRemaining; playback.scheduledFloor = floorRemaining;
    playback.scheduledAt = Date.now(); playback.delay = Math.max(virtualRemaining / speed, floorRemaining); playback.dueAt = playback.scheduledAt + playback.delay;
    playback.timer = setTimeout(() => {
      playback.timer = null; playback.remainingVirtual = 0; playback.remainingFloor = 0;
      const item = playback.items[playback.index++]; renderPlaybackItem(item);
      if (playback.index >= playback.items.length) finishPlayback();
      else {
        const next = playback.items[playback.index];
        const dwell = item.kind === 'event' ? MIN_EVENT_DWELL_MS : 0;
        schedulePlayback(Math.max(0, next.at_ms - item.at_ms), dwell);
      }
    }, playback.delay);
    updatePlaybackButton();
  }
  function playPlayback() {
    if (!playback.items.length) return;
    if (playback.index >= playback.items.length) resetPlayback();
    playback.playing = true;
    schedulePlayback(playback.remainingVirtual, playback.remainingFloor);
  }
  function pausePlayback() {
    if (!playback.playing) return;
    const elapsed = Math.max(0, Date.now() - playback.scheduledAt);
    const remainingReal = Math.max(0, playback.dueAt - Date.now());
    const consumedVirtual = Math.min(playback.scheduledVirtual, elapsed * playback.speed);
    playback.remainingVirtual = Math.max(0, playback.scheduledVirtual - consumedVirtual);
    playback.remainingFloor = Math.max(0, playback.scheduledFloor - elapsed);
    if (playback.timer !== null) clearTimeout(playback.timer);
    playback.timer = null; playback.playing = false;
    if (remainingReal === 0) playback.remainingVirtual = playback.remainingFloor = 0;
    $('simulation-play-status').textContent = `Paused at T+${playback.currentAt} ms · ${playback.focus ? `Observation frame ${playback.focus.frame} · ${playbackFocusLabel(playback.focus)}` : 'waiting for first frame'}`;
    updatePlaybackButton();
  }
  function preparePlayback(result, scenario) {
    const items = [];
    for (const [index, frame] of (Array.isArray(scenario.frames) ? scenario.frames : []).entries()) {
      items.push({kind:'frame', at_ms:Number(frame.at_ms) || 0, frame:index + 1, focus:{focused:frame.observation?.focused === true, target_present:frame.observation?.target_present === true}, order:index * 2});
    }
    for (const [index, observed] of (Array.isArray(result.events) ? result.events : []).entries()) {
      if (!observed || !observed.event) continue;
      items.push({kind:'event', at_ms:Number(observed.at_ms) || 0, frame:Number(observed.frame) || 1, event:observed.event, order:index * 2 + 1});
    }
    items.sort((a,b) => a.at_ms - b.at_ms || a.order - b.order);
    playback.truncated = items.length > MAX_PLAYBACK_ROWS;
    playback.items = items.slice(0, MAX_PLAYBACK_ROWS);
    $('simulation-playback').hidden = false;
    resetPlayback();
    if (playback.truncated) $('simulation-play-status').textContent = `Ready. Playback is limited to the first ${MAX_PLAYBACK_ROWS} timeline rows.`;
  }
  function showSimulationResult(result, scenario) {
    const frames = Array.isArray(result.frames) ? result.frames : [];
    const steps = frames.flatMap((frame, frameIndex) => (Array.isArray(frame.operations) ? frame.operations : []).map((operation, operationIndex) => ({frame: frameIndex + 1, index: operationIndex + 1, ...operation})));
    const failures = steps.filter(step => step.ok === false);
    $('simulation-result-counts').textContent = `${steps.length} step${steps.length === 1 ? '' : 's'} · ${failures.length} failure${failures.length === 1 ? '' : 's'} · ${(Array.isArray(result.events) ? result.events.length : 0)} simulated event${result.events?.length === 1 ? '' : 's'}`;
    const list = $('simulation-steps'); list.replaceChildren();
    for (const step of steps) {
      const item = document.createElement('li');
      if (step.ok === false) item.className = 'failed';
      const label = document.createElement('strong'); label.textContent = `Frame ${step.frame}, step ${step.index}: ${step.op || 'operation'} · ${step.ok === false ? 'failed' : 'ok'}`; item.append(label);
      if (step.error) { const error = document.createElement('span'); error.textContent = String(step.error); item.append(error); }
      list.append(item);
    }
    const lastStep = steps.at(-1);
    $('simulation-last-state').textContent = JSON.stringify(lastStep?.status ?? result.final_status ?? {}, null, 2);
    $('simulation-result').hidden = false;
    preparePlayback(result, scenario);
    $('simulation-status').textContent = failures.length ? `What-if complete with ${failures.length} failed step${failures.length === 1 ? '' : 's'}. Review the trace below.` : 'What-if complete. Review the trace below.';
    if (prefersReducedMotion()) $('simulation-play-status').textContent = 'Ready. Reduced motion is enabled; choose Play scenario to start the trace.';
    else playPlayback();
  }
  async function runSimulation() {
    if (simulationRunning) return;
    const input = $('simulation-scenario').value;
    if (new TextEncoder().encode(input).byteLength > 512 * 1024) { $('simulation-status').textContent = 'Scenario exceeds the 512 KiB editor limit.'; return; }
    let scenario;
    try { scenario = JSON.parse(input); }
    catch (error) { $('simulation-status').textContent = `Invalid JSON: ${error.message}`; return; }
    if (!scenario || typeof scenario !== 'object' || Array.isArray(scenario)) { $('simulation-status').textContent = 'Scenario JSON must be an object.'; return; }
    resetPlayback();
    simulationRunning = true;
    $('simulation-run').disabled = true;
    $('simulation-sample').disabled = true;
    $('simulation-status').textContent = 'Running mock simulation…';
    try {
      const result = await request('/api/simulate', {method: 'POST', body: JSON.stringify({scenario})});
      showSimulationResult(result, scenario);
      simulationRunning = false;
      await poll();
    } catch (error) {
      $('simulation-status').textContent = `Simulation failed: ${error.message}`;
    } finally {
      simulationRunning = false;
      $('simulation-run').disabled = false;
      $('simulation-sample').disabled = false;
    }
  }
  async function poll() {
    if (polling || simulationRunning || !dialog.open) return; polling = true;
    try {
      const data = await request(`/api/input-events?after=${cursor}`); const events = Array.isArray(data.events) ? data.events : [];
      if (data.truncated || (events.length && events[0].sequence > cursor + 1)) { gapCount++; $('keyboard-drop-count').textContent = `${gapCount} event gap${gapCount === 1 ? '' : 's'}`; }
      if (data.truncated && cursor > data.last_sequence) { cursor = 0; return; }
      for (const event of events) { if (event.sequence > cursor) { if (event.source !== 'sim:scenario') timeline(event); cursor = event.sequence; } }
      cursor = Math.max(cursor, Number(data.last_sequence) || 0); remoteHeld = Array.isArray(data.held) ? data.held.filter(item => item.source !== 'sim:scenario') : []; paint();
      $('keyboard-status').textContent = 'Script feed connected · local capture ' + ($('keyboard-capture').checked ? 'on' : 'off');
    } catch (e) { $('keyboard-status').textContent = `Script feed unavailable: ${e.message}`; }
    finally { polling = false; }
  }
  function localEvent(kind, key) {
    if (kind === 'down') { if (localHeld.has(key)) return; localHeld.add(key); }
    else { if (!localHeld.delete(key)) return; }
    timeline({kind, key, source:'local', at_ms:Date.now()}); paint();
  }
  function releaseLocal() { for (const key of [...localHeld]) localEvent('up', key); }
  function excluded(target) { return target instanceof Element && !!target.closest('input,textarea,select,button,[contenteditable]:not([contenteditable="false"])'); }
  document.addEventListener('keydown', e => { if (dialog.open && $('keyboard-capture').checked && !e.repeat && !excluded(e.target)) localEvent('down', e.code); });
  document.addEventListener('keyup', e => { if (localHeld.has(e.code)) localEvent('up', e.code); });
  document.addEventListener('mousedown', e => { if (dialog.open && $('keyboard-capture').checked && surface.contains(e.target) && !excluded(e.target)) { const names = ['Mouse1','Mouse3','Mouse2','Mouse4','Mouse5']; if (names[e.button]) localEvent('down', names[e.button]); } });
  document.addEventListener('mouseup', e => { const names = ['Mouse1','Mouse3','Mouse2','Mouse4','Mouse5']; if (names[e.button]) localEvent('up', names[e.button]); });
  window.addEventListener('blur', releaseLocal); document.addEventListener('visibilitychange', () => { if (document.hidden) releaseLocal(); });
  $('keyboard-capture').addEventListener('change', e => { if (!e.target.checked) releaseLocal(); else surface.focus(); poll(); });
  $('simulation-scenario').value = JSON.stringify(sampleScenario, null, 2);
  $('simulation-sample').addEventListener('click', () => { $('simulation-scenario').value = JSON.stringify(sampleScenario, null, 2); $('simulation-status').textContent = 'Generic sample loaded. The scenario has not been run.'; });
  $('simulation-run').addEventListener('click', runSimulation);
  $('simulation-play-toggle').addEventListener('click', () => playback.playing ? pausePlayback() : playPlayback());
  $('simulation-play-restart').addEventListener('click', () => { resetPlayback(); playPlayback(); });
  $('simulation-play-speed').addEventListener('change', e => {
    const nextSpeed = Number(e.target.value) || 1;
    if (!playback.playing || playback.timer === null) { playback.speed = nextSpeed; return; }
    const elapsed = Math.max(0, Date.now() - playback.scheduledAt);
    const virtualRemaining = Math.max(0, playback.scheduledVirtual - Math.min(playback.scheduledVirtual, elapsed * playback.speed));
    const floorRemaining = Math.max(0, playback.scheduledFloor - elapsed);
    clearTimeout(playback.timer); playback.timer = null; playback.speed = nextSpeed;
    schedulePlayback(virtualRemaining, floorRemaining);
  });
  $('keyboard-open').addEventListener('click', () => { dialog.showModal(); surface.focus(); poll(); timer = setInterval(poll, 300); });
  $('keyboard-close').addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => { clearInterval(timer); timer = null; resetPlayback(); releaseLocal(); $('keyboard-capture').checked = false; });
  $('keyboard-clear').addEventListener('click', () => { $('keyboard-timeline').replaceChildren(); gapCount = 0; $('keyboard-drop-count').textContent = 'No gaps'; });
  $('keyboard-reset').addEventListener('click', async () => {
    releaseLocal(); const button = $('keyboard-reset'); button.disabled = true;
    try { await request('/api/input-events', {method:'POST', body:JSON.stringify({source:'demo',events:[{kind:'reset'}]})}); await poll(); }
    catch (e) { $('keyboard-status').textContent = `Reset unavailable: ${e.message}`; } finally { button.disabled = false; }
  });
  $('keyboard-demo').addEventListener('click', async () => {
    if (demoRunning) return; demoRunning = true; const button = $('keyboard-demo'); button.disabled = true; $('keyboard-reset').disabled = true; let down = null;
    const send = events => request('/api/input-events', {method:'POST', body:JSON.stringify({source:'demo',events})});
    try { for (const key of ['KeyA','Space','Mouse1']) { down = key; await send([{kind:'down',key}]); await poll(); await new Promise(resolve => setTimeout(resolve, 450)); await send([{kind:'up',key}]); down = null; await poll(); await new Promise(resolve => setTimeout(resolve, 150)); } }
    catch (e) { $('keyboard-status').textContent = `Demo unavailable: ${e.message}`; }
    finally { if (down) { try { await send([{kind:'up',key:down}]); } catch {} } button.disabled = false; $('keyboard-reset').disabled = false; demoRunning = false; }
  });
})();


