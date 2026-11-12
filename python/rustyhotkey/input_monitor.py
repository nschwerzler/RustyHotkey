"""Bounded, in-memory input telemetry. Publishing never sends native input."""
from __future__ import annotations

from collections import deque
import re
import threading
import time
from typing import Any


def key_code(vk: int) -> str:
    """Convert a Windows virtual key to a keyboard graphic's DOM code."""
    if 65 <= vk <= 90: return 'Key' + chr(vk)
    if 48 <= vk <= 57: return 'Digit' + chr(vk)
    if 112 <= vk <= 135: return f'F{vk - 111}'
    if 96 <= vk <= 105: return f'Numpad{vk - 96}'
    return {8: 'Backspace', 9: 'Tab', 13: 'Enter', 16: 'ShiftLeft',
            17: 'ControlLeft', 18: 'AltLeft', 19: 'Pause', 20: 'CapsLock',
            27: 'Escape', 32: 'Space', 33: 'PageUp', 34: 'PageDown',
            35: 'End', 36: 'Home', 37: 'ArrowLeft', 38: 'ArrowUp',
            39: 'ArrowRight', 40: 'ArrowDown', 44: 'PrintScreen',
            45: 'Insert', 46: 'Delete', 91: 'MetaLeft', 92: 'MetaRight',
            93: 'ContextMenu', 106: 'NumpadMultiply', 107: 'NumpadAdd',
            109: 'NumpadSubtract', 110: 'NumpadDecimal', 111: 'NumpadDivide',
            144: 'NumLock', 145: 'ScrollLock', 160: 'ShiftLeft',
            161: 'ShiftRight', 162: 'ControlLeft', 163: 'ControlRight',
            164: 'AltLeft', 165: 'AltRight', 186: 'Semicolon', 187: 'Equal',
            188: 'Comma', 189: 'Minus', 190: 'Period', 191: 'Slash',
            192: 'Backquote', 219: 'BracketLeft', 220: 'Backslash',
            221: 'BracketRight', 222: 'Quote'}.get(vk, f'VK_{vk:02X}')


class InputMonitor:
    """Shared event ring for the UI and script clients; no global keyboard hook.

    Sources keep separate held-key sets so a release in one script cannot clear
    another script's key. Data is deliberately transient and text is omitted.
    """
    LIMIT = 512

    def __init__(self):
        self._events: deque[dict[str, Any]] = deque(maxlen=self.LIMIT)
        self._held: set[tuple[str, str]] = set()
        self._sequence = 0
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._workers: dict[str, threading.Thread] = {}
        self._closed = False

    def publish(self, source: str, events: list[dict[str, Any]]) -> dict[str, Any]:
        if not isinstance(source, str) or not re.fullmatch(r'[A-Za-z0-9_.:-]{1,64}', source):
            raise ValueError('source must be a short script identifier')
        if not isinstance(events, list) or len(events) > 256:
            raise ValueError('events must contain at most 256 entries')
        clean = []
        for event in events:
            if not isinstance(event, dict) or set(event) - {'kind', 'key'}:
                raise ValueError('input events accept only kind and key')
            kind = event.get('kind'); key = event.get('key', '')
            if not isinstance(kind, str) or kind not in {'down', 'up', 'reset'}:
                raise ValueError('event kind must be down, up or reset')
            if not isinstance(key, str) or (kind != 'reset' and not re.fullmatch(r'[A-Za-z0-9_]{1,32}', key)):
                raise ValueError('key must be a keyboard code')
            clean.append((kind, '' if kind == 'reset' else key))
        with self._lock:
            if self._closed: raise ValueError('input monitor is closed')
            sources = {s for s, _ in self._held} | set(self._workers)
            if source not in sources and len(sources) >= 32:
                raise ValueError('too many active input sources')
            # Reject a whole batch rather than leaving a partially updated state.
            held = set(self._held)
            for kind, key in clean:
                if kind == 'reset': held = {(s, k) for s, k in held if s != source}
                elif kind == 'down': held.add((source, key))
                else: held.discard((source, key))
                if len(held) > 512: raise ValueError('too many held keys')
            self._held = held
            for kind, key in clean:
                self._sequence += 1
                self._events.append({'sequence': self._sequence, 'source': source,
                                     'kind': kind, 'key': key, 'at_ms': time.time_ns() // 1_000_000})
            return {'accepted': len(clean), 'last_sequence': self._sequence}

    def snapshot(self, after: int = 0) -> dict[str, Any]:
        if isinstance(after, bool) or not isinstance(after, int) or after < 0:
            raise ValueError('after must be a nonnegative sequence')
        with self._lock:
            oldest = self._events[0]['sequence'] if self._events else self._sequence + 1
            return {'events': [dict(e) for e in self._events if e['sequence'] > after],
                    'held': [{'source': s, 'key': k} for s, k in sorted(self._held)],
                    'last_sequence': self._sequence, 'oldest_sequence': oldest,
                    'truncated': after < oldest - 1 or after > self._sequence, 'limit': self.LIMIT}

    def actions(self, source: str, actions: list[dict[str, Any]]) -> dict[str, Any]:
        """Visualize intended actions without executing them; text is omitted."""
        if not isinstance(actions, list) or len(actions) > 128 or any(not isinstance(a, dict) for a in actions):
            raise ValueError('actions must contain at most 128 action objects')
        events = []
        for action in actions:
            kind = action.get('type')
            if kind in {'tap', 'key'}:
                vk = action.get('key')
                if isinstance(vk, bool) or not isinstance(vk, int) or not 1 <= vk <= 255:
                    raise ValueError('key must be a virtual key in 1..255')
                key = key_code(vk)
            elif kind in {'click', 'button'}:
                button = action.get('button')
                if isinstance(button, bool) or not isinstance(button, int) or not 1 <= button <= 5:
                    raise ValueError('button must be in 1..5')
                key = f'Mouse{button}'
            else: continue
            if kind in {'tap', 'click'}:
                events.extend([{'kind': 'down', 'key': key}, {'kind': 'up', 'key': key}])
            else:
                if not isinstance(action.get('down'), bool): raise ValueError('down must be boolean')
                events.append({'kind': 'down' if action['down'] else 'up', 'key': key})
        return self.publish('sim:' + source, events)

    def attach(self, client: Any, source: str) -> None:
        """Poll successful native emissions, including safety releases."""
        source = 'live:' + source
        self.publish(source, [])  # validate before allocating a worker
        with self._lock:
            if source in self._workers: return
            if len(self._workers) >= 16: raise ValueError('too many monitored clients')
            def poll():
                after = 0
                try:
                    while not self._stop.is_set():
                        if getattr(client, '_closed', False): break
                        try:
                            snapshot = client.input_events(after)
                        except Exception:
                            if getattr(client, '_closed', False) or getattr(client, '_error', None): break
                            # Long workflows may keep the serial engine busy. Retry
                            # telemetry without cancelling the running workflow.
                            if self._stop.wait(0.5): break
                            continue
                        if snapshot.get('truncated'):
                            self.publish(source, [{'kind': 'reset'}])
                        batch = [{'kind': e['kind'], 'key': e.get('key', '')} for e in snapshot['events']]
                        # Native rings can contain 512 entries, split at publish's bound.
                        for offset in range(0, len(batch), 256): self.publish(source, batch[offset:offset + 256])
                        after = snapshot['last_sequence']
                        if snapshot.get('truncated'):
                            self.publish(source, [{'kind': 'reset'}])
                            held = snapshot.get('held', [])
                            held = [h['key'] if isinstance(h, dict) else h for h in held]
                            for offset in range(0, len(held), 256):
                                self.publish(source, [{'kind': 'down', 'key': k} for k in held[offset:offset + 256]])
                        self._stop.wait(0.1)
                except Exception:
                    # Telemetry must never prevent engine cancellation or shutdown.
                    pass
                finally:
                    try: self.publish(source, [{'kind': 'reset'}])
                    except ValueError: pass
                    with self._lock: self._workers.pop(source, None)
            worker = threading.Thread(target=poll, name='rhk-input-monitor', daemon=True)
            self._workers[source] = worker
        worker.start()

    def simulation(self, result: dict[str, Any], source: str = 'scenario') -> None:
        """Display a mock engine trace, omitting text contents and pointer moves."""
        self.publish('sim:' + source, [{'kind': 'reset'}])
        batch = []
        for item in result.get('events', []):
            event = item.get('event', {})
            if event.get('type') == 'key': key = key_code(event['key'])
            elif event.get('type') == 'button': key = f"Mouse{event['button']}"
            else: continue
            batch.append({'kind': 'down' if event['down'] else 'up', 'key': key})
            if len(batch) == 256:
                self.publish('sim:' + source, batch); batch = []
        if batch: self.publish('sim:' + source, batch)

    def close(self) -> None:
        self._stop.set()
        with self._lock:
            self._closed = True
            workers = list(self._workers.values())
            self._held.clear()
        for worker in workers: worker.join(timeout=0.25)
