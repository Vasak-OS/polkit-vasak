import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { applyTheme, saveTheme, THEME_KEY } from '@/tools/theme-cache';

/** Un `Storage` mínimo en memoria. */
function memoryStorage(initial: Record<string, string> = {}): Storage {
	const data = new Map(Object.entries(initial));
	return {
		get length() {
			return data.size;
		},
		clear: () => data.clear(),
		getItem: (key: string) => data.get(key) ?? null,
		key: (index: number) => Array.from(data.keys())[index] ?? null,
		removeItem: (key: string) => void data.delete(key),
		setItem: (key: string, value: string) => void data.set(key, value),
	};
}

/** Un `Storage` que tira en cada llamada, como un WebView sin `localStorage`. */
function brokenStorage(): Storage {
	const fail = () => {
		throw new Error('localStorage no disponible');
	};
	return {
		length: 0,
		clear: fail,
		getItem: fail,
		key: fail,
		removeItem: fail,
		setItem: fail,
	};
}

function newRoot(): HTMLElement {
	return document.createElement('div');
}

const source = (path: string) => readFileSync(resolve(import.meta.dir, '..', path), 'utf8');

describe('theme-cache', () => {
	test('la clave es la que comparte el script de index.html', () => {
		expect(THEME_KEY).toBe('vsk-theme');
	});

	test('saveTheme guarda la clase oscura y sólo las variables --*', () => {
		const root = newRoot();
		root.classList.add('dark');
		root.style.setProperty('--ui-background', '#111111');
		root.style.setProperty('--ui-accent', '#ff0000');
		root.style.setProperty('color', 'red');
		const storage = memoryStorage();

		saveTheme(root, storage);

		expect(JSON.parse(storage.getItem(THEME_KEY) as string)).toEqual({
			dark: true,
			vars: { '--ui-background': '#111111', '--ui-accent': '#ff0000' },
		});
	});

	test('saveTheme en claro guarda dark en falso', () => {
		const storage = memoryStorage();
		saveTheme(newRoot(), storage);
		expect(JSON.parse(storage.getItem(THEME_KEY) as string)).toEqual({ dark: false, vars: {} });
	});

	test('applyTheme reaplica .dark y las variables', () => {
		const root = newRoot();
		const storage = memoryStorage({
			[THEME_KEY]: JSON.stringify({ dark: true, vars: { '--ui-background': '#222222' } }),
		});

		applyTheme(root, storage);

		expect(root.classList.contains('dark')).toBe(true);
		expect(root.style.getPropertyValue('--ui-background')).toBe('#222222');
	});

	test('ida y vuelta: lo guardado se reaplica igual sobre otra raíz', () => {
		const first = newRoot();
		first.classList.add('dark');
		first.style.setProperty('--ui-surface', '#333333');
		const storage = memoryStorage();
		saveTheme(first, storage);

		const second = newRoot();
		applyTheme(second, storage);

		expect(second.classList.contains('dark')).toBe(true);
		expect(second.style.getPropertyValue('--ui-surface')).toBe('#333333');
	});

	test('sin clave, applyTheme no toca nada', () => {
		const root = newRoot();
		applyTheme(root, memoryStorage());
		expect(root.classList.contains('dark')).toBe(false);
		expect(root.getAttribute('style')).toBeNull();
	});

	test('un JSON inválido no lanza ni toca la raíz', () => {
		const root = newRoot();
		expect(() => applyTheme(root, memoryStorage({ [THEME_KEY]: '{no es json' }))).not.toThrow();
		expect(root.classList.contains('dark')).toBe(false);
	});

	test('un Storage que tira no hace fallar ni a saveTheme ni a applyTheme', () => {
		expect(() => saveTheme(newRoot(), brokenStorage())).not.toThrow();
		expect(() => applyTheme(newRoot(), brokenStorage())).not.toThrow();
	});
});

describe('cableado del tema recordado', () => {
	test('index.html reaplica el tema antes de montar', () => {
		const html = source('index.html');
		expect(html).toContain('vsk-theme');
		expect(html).toContain('classList.add("dark")');
		expect(html).toContain('setProperty');
		// El script tiene que ir antes del módulo que monta la aplicación.
		expect(html.indexOf('vsk-theme')).toBeLessThan(html.indexOf('type="module"'));
	});

	test('App.vue guarda el tema tras loadConfig, sin tocar el flujo de polkit', () => {
		const app = source('src/App.vue');
		expect(app).toContain("import { saveTheme } from '@/tools/theme-cache'");
		expect(app).toMatch(/loadConfig\(\)[\s\S]*\.then\(\(\) => saveTheme\(\)\)/);
	});
});
