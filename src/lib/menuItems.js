// Пункты меню страницы → описание для родного меню ядра. Отдельно от Tauri, чтобы проверять тестами.
// items: [{ label, run, danger?, disabled?, separator? }]; translate(label) → текст на языке интерфейса.
export function toNative(items, translate) {
  const runs = [];
  const native = items.map((item) => {
    if (item.separator) return { id: null, separator: true };
    runs.push(item.run);
    return { id: runs.length - 1, label: translate(item.label), danger: !!item.danger, disabled: !!item.disabled };
  });
  return { native, runs };
}
