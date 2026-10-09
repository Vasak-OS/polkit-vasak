import I18n from '@vasakgroup/tauri-plugin-i18n';

/**
 * Cuánto se espera a las traducciones antes de montar.
 *
 * Se espera para que el diálogo no muestre las claves crudas, pero con plazo: si
 * el backend no contesta, es mejor un diálogo con las claves a la vista que una
 * ventana en blanco pidiendo una contraseña.
 */
export const TRANSLATIONS_TIMEOUT_MS = 1500;

/** Opciones de {@link loadTranslations}. Los valores por omisión son los de producción. */
export interface LoadTranslationsOptions {
	/** Plazo total, en ms, tras el cual se monta aunque la carga siga en curso. */
	timeoutMs?: number;
	/** Cuántas veces se intenta antes de rendirse. */
	maxAttempts?: number;
	/** Espera base del backoff exponencial, en ms. */
	baseDelayMs?: number;
	/** Techo de la espera entre intentos, en ms. */
	maxDelayMs?: number;
	/**
	 * Cómo se piden las traducciones. Por omisión, al backend de i18n; la prueba
	 * inyecta el suyo para no tener que doblar el módulo entero.
	 */
	load?: () => Promise<void>;
}

/**
 * Carga las traducciones con reintentos, pero sin pasarse del plazo.
 *
 * Antes de montar: el diálogo de contraseña aparece de golpe encima de lo que sea
 * que estés haciendo y se responde en dos segundos, así que mostrar `polkit.title`
 * y después corregirlo se ve peor que la espera.
 *
 * Un intento que falla se reintenta —el backend puede tardar o no escuchar a la
 * primera—, con backoff exponencial entre intentos. Pero la espera total sigue
 * acotada por `timeoutMs`: un backend colgado tiene que dar un diálogo con las
 * claves a la vista, no una ventana en blanco pidiendo una contraseña.
 */
export async function loadTranslations(options: LoadTranslationsOptions = {}): Promise<void> {
	const {
		timeoutMs = TRANSLATIONS_TIMEOUT_MS,
		maxAttempts = 3,
		baseDelayMs = 500,
		maxDelayMs = 2000,
		load = () => I18n.getInstance().load(),
	} = options;

	const attempt = async () => {
		for (let i = 0; i < maxAttempts; i++) {
			try {
				await load();
				return;
			} catch (error) {
				console.error(
					`No se pudieron cargar las traducciones (intento ${i + 1}/${maxAttempts}):`,
					error
				);
				if (i === maxAttempts - 1) return;
				const delay = Math.min(baseDelayMs * 2 ** i, maxDelayMs);
				await new Promise((resolve) => setTimeout(resolve, delay));
			}
		}
	};

	await Promise.race([attempt(), new Promise<void>((resolve) => setTimeout(resolve, timeoutMs))]);
}
