// Запрос «перевести письмо» из списка: Reader подхватывает его, когда письмо откроется.
import { writable } from 'svelte/store';

export const translateRequest = writable(null);
export const requestTranslation = (id) => translateRequest.set({ id, at: Date.now() });
