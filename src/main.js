import { getAppearance } from './lib/api.js';
import { setLanguage } from './lib/i18n.js';
import { listen } from '@tauri-apps/api/event';
import { mount } from 'svelte';
import { getCurrentWindow } from '@tauri-apps/api/window';
import App from './App.svelte';
import ComposeWindow from './ComposeWindow.svelte';
import SettingsWindow from './SettingsWindow.svelte';
import './app.css';

// Один фронтенд на все окна: окна писем создаются ядром с меткой compose-N, настройки — "settings".
const label = getCurrentWindow().label;
const Root = label.startsWith('compose-') ? ComposeWindow : label === 'settings' ? SettingsWindow : App;
// Subscribe before reading preferences so every window follows language changes.
let languageUpdated = false;
await listen('appearance-changed', ({ payload }) => {
  languageUpdated = true;
  setLanguage(payload.language);
});
try {
  const appearance = await getAppearance();
  if (!languageUpdated) setLanguage(appearance.language);
} catch (error) {
  console.error('Could not load interface language:', error);
}
mount(Root, { target: document.getElementById('app') });
