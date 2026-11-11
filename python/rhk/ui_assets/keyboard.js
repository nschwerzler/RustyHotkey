(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const dialog = $('keyboard-dialog');
  const layout = $('keyboard-layout');
  const surface = $('keyboard-surface');
  const keys = new Map();
  const localHeld = new Set();
  let remoteHeld = [], cursor = 0, gapCount = 0, timer = null, polling = false, demoRunning = false;
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
  function sourceLabel(source) { source = String(source || 'unknown'); if (source === 'local') return 'local page'; if (source === 'demo') return 'demo (simulated)'; if (source.startsWith('sim:')) return source.slice(4) + ' (simulated)'; if (source.startsWith('live:')) return source.slice(5) + ' (live script)'; return source; }
  function allHeld() { return [...remoteHeld, ...Array.from(localHeld, key => ({source:'local',key}))]; }
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
  async function poll() {
    if (polling || !dialog.open) return; polling = true;
    try {
      const data = await request(`/api/input-events?after=${cursor}`); const events = Array.isArray(data.events) ? data.events : [];
      if (data.truncated || (events.length && events[0].sequence > cursor + 1)) { gapCount++; $('keyboard-drop-count').textContent = `${gapCount} event gap${gapCount === 1 ? '' : 's'}`; }
      if (data.truncated && cursor > data.last_sequence) { cursor = 0; return; }
      for (const event of events) { if (event.sequence > cursor) { timeline(event); cursor = event.sequence; } }
      cursor = Math.max(cursor, Number(data.last_sequence) || 0); remoteHeld = Array.isArray(data.held) ? data.held : []; paint();
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
  $('keyboard-open').addEventListener('click', () => { dialog.showModal(); surface.focus(); poll(); timer = setInterval(poll, 300); });
  $('keyboard-close').addEventListener('click', () => dialog.close());
  dialog.addEventListener('close', () => { clearInterval(timer); timer = null; releaseLocal(); $('keyboard-capture').checked = false; });
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


