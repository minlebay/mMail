import test from 'node:test';
import assert from 'node:assert/strict';
import { monthGrid, startOfWeek, visibleRange, toRfc3339, fromDateKey, toDateKey, onDay, layoutDay, titleLines } from '../src/lib/calendar/dates.js';

test('weeks start on Monday and the month grid covers the whole month', () => {
  const oct = new Date(2026, 9, 17); // суббота
  assert.equal(startOfWeek(oct).getDay(), 1);
  const grid = monthGrid(oct);
  assert.equal(grid.length, 42);
  assert.equal(grid[0].getDay(), 1);
  assert.ok(grid.some((d) => d.getMonth() === 9 && d.getDate() === 1));
  assert.ok(grid.some((d) => d.getMonth() === 9 && d.getDate() === 31));
  const [from, to] = visibleRange('week', oct);
  assert.equal((to - from) / 86400000, 7);
});

test('local dates and times round-trip without UTC shifts', () => {
  const d = new Date(2026, 9, 20, 10, 30);
  assert.equal(new Date(toRfc3339(d)).getTime(), d.getTime());
  assert.equal(toDateKey(fromDateKey('2026-10-20')), '2026-10-20');
});

test('all-day end is exclusive; overlapping events get separate columns', () => {
  const allDay = { allDay: true, start: '2026-10-20', end: '2026-10-21' };
  assert.ok(onDay(allDay, new Date(2026, 9, 20)));
  assert.ok(!onDay(allDay, new Date(2026, 9, 21)));
  const at = (h1, h2) => ({ allDay: false, start: toRfc3339(new Date(2026, 9, 20, h1)), end: toRfc3339(new Date(2026, 9, 20, h2)) });
  const laid = layoutDay([at(9, 11), at(10, 12), at(13, 14)], new Date(2026, 9, 20));
  assert.deepEqual(laid.map((l) => [l.column, l.columns]), [[0, 2], [1, 2], [0, 1]]);
});

test('full event titles share the day cell', () => {
  // Клетка ~190px, как на 1440p: одно событие — почти вся клетка, три — поровну.
  assert.equal(titleLines(190, 1, false), 9);
  assert.equal(titleLines(190, 3, false), 2);
  assert.ok(titleLines(190, 3, true) <= titleLines(190, 3, false), '«ещё N» забирает место');
  assert.equal(titleLines(60, 3, true), 1, 'в тесной клетке — хотя бы одна строка');
  assert.equal(titleLines(0, 0, false), 1);
});
