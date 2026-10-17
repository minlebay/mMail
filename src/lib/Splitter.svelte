<script>
  // Вертикальный разделитель между панелями: тянется мышью, двойной клик — ширина по умолчанию.
  let { value, min, max, initial, onchange } = $props();

  let start = null;

  function onpointerdown(e) {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    start = { x: e.clientX, value };
  }

  function onpointermove(e) {
    if (!start) return;
    onchange(Math.round(Math.min(max, Math.max(min, start.value + e.clientX - start.x))));
  }

  function onpointerup() {
    start = null;
  }
</script>

<div
  class="splitter"
  role="separator"
  aria-orientation="vertical"
  aria-valuenow={value}
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  ondblclick={() => onchange(initial)}
></div>

<style>
  .splitter {
    position: relative;
    cursor: col-resize;
    touch-action: none;
  }
  .splitter::after {
    content: '';
    position: absolute;
    top: 12px;
    bottom: 12px;
    left: 50%;
    width: 2px;
    margin-left: -1px;
    border-radius: 1px;
    background: transparent;
    transition: background 0.15s;
  }
  .splitter:hover::after,
  .splitter:active::after {
    background: var(--accent);
  }
</style>
