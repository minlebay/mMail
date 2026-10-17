import { mount } from 'svelte';
import { getCurrentWindow } from '@tauri-apps/api/window';
import App from './App.svelte';
import ComposeWindow from './ComposeWindow.svelte';
import SettingsWindow from './SettingsWindow.svelte';
import './app.css';

// Один фронтенд на все окна: окна писем создаются ядром с меткой compose-N, настройки — "settings".
const label = getCurrentWindow().label;
const Root = label.startsWith('compose-') ? ComposeWindow : label === 'settings' ? SettingsWindow : App;
mount(Root, { target: document.getElementById('app') });
