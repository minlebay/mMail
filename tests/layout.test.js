import test from 'node:test';
import assert from 'node:assert/strict';
import { wideLayoutEnabled } from '../src/lib/layout.js';

test('wide layout requires opt-in and enough viewport width', () => {
  assert.equal(wideLayoutEnabled(false, 2560, 210, 300), false);
  assert.equal(wideLayoutEnabled(undefined, 2560, 210, 300), false);
  assert.equal(wideLayoutEnabled(true, 2559, 210, 300), false);
  assert.equal(wideLayoutEnabled(true, 2560, 210, 300), true);
});
test('expanded panels reserve space for both message and calendar', () => {
  assert.equal(wideLayoutEnabled(true, 2560, 900, 900), false);
  assert.equal(wideLayoutEnabled(true, 3000, 900, 900), true);
});
test('resizing or zooming back below the threshold restores normal layout', () => {
  assert.deepEqual([2560, 2560 / 1.5, 2560].map((width) => wideLayoutEnabled(true, width, 210, 300)), [true, false, true]);
});
