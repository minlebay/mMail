// Получатели из ядра: [{ name, addr }].
export const names = (list) => list.map((a) => a.name || a.addr).join(', ');
export const addrs = (list) => list.map((a) => a.addr).join(', ');
// «Имя <адрес>» — как в поле «Кому» письма.
export const full = (a) => (a.name ? `${a.name} <${a.addr}>` : a.addr);
