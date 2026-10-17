<script>
  import Settings from './lib/Settings.svelte';
  import { applyTheme, DEFAULT_THEME, DEFAULT_OPACITY } from './lib/themes.js';
  import { getAppearance } from './lib/api.js';

  // Форма строится по сохранённым настройкам, поэтому сначала читаем их.
  let appearance = $state(null);
  getAppearance().then((a) => {
    appearance = { ...a, theme: a.theme || DEFAULT_THEME, opacity: a.opacity ?? DEFAULT_OPACITY };
    applyTheme(appearance.theme, appearance.opacity);
  });
</script>

{#if appearance}
  <Settings {appearance} />
{/if}
