import test from 'node:test';
import assert from 'node:assert/strict';
import { toNative } from '../src/lib/menuItems.js';

test('page items become native entries; actions stay in the page', () => {
  let calls = 0;
  const { native, runs } = toNative(
    [
      { label: 'Ответить', run: () => calls++ },
      { separator: true },
      { label: 'Удалить', danger: true, run: () => calls++ },
      { label: 'Сохранить вложение', disabled: true, run: () => calls++ },
    ],
    (label) => `[${label}]`,
  );
  assert.deepEqual(native, [
    { id: 0, label: '[Ответить]', danger: false, disabled: false },
    { id: null, separator: true },
    { id: 1, label: '[Удалить]', danger: true, disabled: false },
    { id: 2, label: '[Сохранить вложение]', danger: false, disabled: true },
  ]);
  // Ядру уходят только тексты; функции остаются здесь и вызываются по номеру.
  assert.equal(JSON.stringify(native).includes('=>'), false);
  assert.equal(calls, 0);
  runs[1]();
  assert.equal(calls, 1);
});
