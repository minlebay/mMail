import { mount } from 'svelte';
import { getCurrentWindow } from '@tauri-apps/api/window';
import App from './App.svelte';
import ComposeWindow from './ComposeWindow.svelte';
import './app.css';

// Один фронтенд на все окна: окна писем создаются ядром с меткой compose-N.
const Root = getCurrentWindow().label.startsWith('compose-') ? ComposeWindow : App;
mount(Root, { target: document.getElementById('app') });
