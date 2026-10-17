import test from 'node:test';
import assert from 'node:assert/strict';
import { get } from 'svelte/store';
import { contextMenu, showContextMenu, closeContextMenu, menuPosition } from '../src/lib/contextMenu.js';

test('menu stays within each viewport edge, including a small window', () => {
  assert.deepEqual(menuPosition(990, 790, 220, 240, 1000, 800), { x: 772, y: 552 });
  assert.deepEqual(menuPosition(-4, -9, 220, 240, 1000, 800), { x: 8, y: 8 });
  assert.deepEqual(menuPosition(20, 30, 220, 240, 150, 180), { x: 8, y: 8 });
  assert.deepEqual(menuPosition(100, 120, 220, 240, 1000, 800), { x: 100, y: 120 });
});
test('opening only captures the clicked target; no action runs before selection', () => {
  let calls = 0;
  let prevented = false;
  let stopped = false;
  const anchor = { getBoundingClientRect: () => ({ left: 40, top: 50 }) };
  const items = [{ label: 'Ответить', run: () => calls++ }];
  showContextMenu({ currentTarget: anchor, clientX: 100, clientY: 120,
    preventDefault: () => { prevented = true; }, stopPropagation: () => { stopped = true; } }, items);
  assert.equal(get(contextMenu).anchor, anchor);
  assert.equal(get(contextMenu).items, items);
  assert.equal(calls, 0);
  assert.ok(prevented && stopped);
  closeContextMenu();
  assert.equal(get(contextMenu), null);
  assert.equal(calls, 0);
});
test('keyboard invocation positions the menu at the focused target', () => {
  showContextMenu({ currentTarget: { getBoundingClientRect: () => ({ left: 40, top: 50 }) },
    preventDefault() {}, stopPropagation() {} }, []);
  assert.equal(get(contextMenu).x, 56);
  assert.equal(get(contextMenu).y, 66);
  closeContextMenu();
});
