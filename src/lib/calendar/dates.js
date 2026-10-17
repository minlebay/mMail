// Даты календаря: неделя с понедельника, местный часовой пояс. Без зависимостей — проверяется тестами.

export const DAY = 24 * 60 * 60 * 1000;

export const startOfDay = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
export const addDays = (d, n) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n, d.getHours(), d.getMinutes());
export const addMonths = (d, n) => new Date(d.getFullYear(), d.getMonth() + n, 1);
export const sameDay = (a, b) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** Понедельник недели, в которую попадает дата. */
export function startOfWeek(d) {
  const day = startOfDay(d);
  return addDays(day, -((day.getDay() + 6) % 7));
}

/** 42 дня (6 недель) сетки месяца, начиная с понедельника. */
export function monthGrid(d) {
  const first = startOfWeek(new Date(d.getFullYear(), d.getMonth(), 1));
  return Array.from({ length: 42 }, (_, i) => addDays(first, i));
}

/** Видимый промежуток [from, to) для вида: month | week | day. */
export function visibleRange(view, d) {
  if (view === 'month') {
    const grid = monthGrid(d);
    return [grid[0], addDays(grid[41], 1)];
  }
  if (view === 'week') {
    const start = startOfWeek(d);
    return [start, addDays(start, 7)];
  }
  const start = startOfDay(d);
  return [start, addDays(start, 1)];
}

/** RFC 3339 в местном поясе: 2026-10-20T10:00:00+02:00. */
export function toRfc3339(d) {
  const pad = (n) => String(Math.abs(n)).padStart(2, '0');
  const offset = -d.getTimezoneOffset();
  const sign = offset >= 0 ? '+' : '-';
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:00` +
    `${sign}${pad(Math.trunc(offset / 60))}:${pad(offset % 60)}`
  );
}

/** YYYY-MM-DD в местном поясе. */
export function toDateKey(d) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** Дата из YYYY-MM-DD — полночь по местному времени (а не UTC, как new Date('2026-10-20')). */
export function fromDateKey(key) {
  const [y, m, d] = key.split('-').map(Number);
  return new Date(y, m - 1, d);
}

/** Начало и конец события как Date; у событий на весь день конец не включается (как в Google). */
export function eventBounds(e) {
  return e.allDay ? [fromDateKey(e.start), fromDateKey(e.end)] : [new Date(e.start), new Date(e.end)];
}

/** Попадает ли событие на день (хотя бы минутой). */
export function onDay(e, day) {
  const [start, end] = eventBounds(e);
  const dayStart = startOfDay(day);
  const dayEnd = addDays(dayStart, 1);
  return start < dayEnd && end > dayStart;
}

/** Положение события со временем в колонке дня: доли суток [top, height]. */
export function dayPosition(e, day) {
  const [start, end] = eventBounds(e);
  const dayStart = startOfDay(day).getTime();
  const from = Math.max(start.getTime(), dayStart);
  const to = Math.min(end.getTime(), dayStart + DAY);
  return [(from - dayStart) / DAY, Math.max(to - from, 15 * 60 * 1000) / DAY];
}

/** Раскладка пересекающихся событий дня по колонкам: [{ event, column, columns }]. */
export function layoutDay(events, day) {
  const items = events
    .map((event) => ({ event, pos: dayPosition(event, day) }))
    .sort((a, b) => a.pos[0] - b.pos[0] || b.pos[1] - a.pos[1]);
  const out = [];
  let group = [];
  let groupEnd = -1;
  const flush = () => {
    const columns = Math.max(1, ...group.map((g) => g.column + 1));
    for (const g of group) out.push({ ...g, columns });
    group = [];
  };
  for (const item of items) {
    if (item.pos[0] >= groupEnd) {
      flush();
      groupEnd = -1;
    }
    const busy = new Set(group.filter((g) => g.pos[0] + g.pos[1] > item.pos[0]).map((g) => g.column));
    let column = 0;
    while (busy.has(column)) column++;
    group.push({ ...item, column });
    groupEnd = Math.max(groupEnd, item.pos[0] + item.pos[1]);
  }
  flush();
  return out;
}
