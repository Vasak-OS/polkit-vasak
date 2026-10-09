/**
 * Los dobles de lo que sólo existe adentro de la ventana de Tauri.
 *
 * Montar el diálogo es lo único que comprueba de verdad que el marco que dibuja
 * es el compartido; sin estos, importarlo falla en la primera línea.
 */

const icons = new Map<string, string>();
const listeners = new Map<string, Set<(event: { payload: unknown }) => unknown>>();

/** Los comandos que se invocaron, en orden. */
export const invocations: string[] = [];
/** Y con qué argumentos, en el mismo orden. */
export const invocationArgs: unknown[] = [];

export async function invoke(command: string, args?: unknown) {
	invocations.push(command);
	invocationArgs.push(args);
	return undefined;
}

export async function listen(name: string, handler: (event: { payload: unknown }) => unknown) {
	const own = listeners.get(name) ?? new Set<(event: { payload: unknown }) => unknown>();
	own.add(handler);
	listeners.set(name, own);
	return () => {
		own.delete(handler);
	};
}

/** Dispara un evento de Tauri sobre los que se hayan suscrito. */
export async function emit(name: string, payload: unknown) {
	for (const handler of listeners.get(name) ?? []) await handler({ payload });
}

export function putInTheme(name: string, source: string) {
	icons.set(name, source);
}

export async function getIconSource(name: string) {
	return icons.get(name) ?? '';
}

export async function getSymbolSource(_name: string) {
	return '';
}

/** El `t()` devuelve la clave: una prueba que mire el texto mira la clave. */
export function useI18n() {
	return { t: (key: string) => key, locale: { value: 'es' } };
}

/**
 * El doble del export por omisión del plugin de i18n. `main.ts` y
 * `loadTranslations` importan `I18n` por omisión; sin este doble, el import se
 * cae con «Missing 'default' export» aunque nadie llame a `load()`.
 */
export const I18n = {
	getInstance() {
		return { load: async () => {} };
	},
};

/** La configuración de la ventana. El diálogo sólo la pide para los colores. */
export function useConfigStore() {
	return { config: {}, loadConfig: async () => {} };
}

/** Lo que `readConfig()` le devuelve al marco compartido para saber dónde va la barra. */
export async function readConfig() {
	return {};
}

export function forgetEverything() {
	invocations.length = 0;
	invocationArgs.length = 0;
	icons.clear();
	listeners.clear();
}
