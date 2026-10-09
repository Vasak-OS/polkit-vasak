import I18n from '@vasakgroup/tauri-plugin-i18n';
import { createPinia } from 'pinia';
import { createApp } from 'vue';
import App from '@/App.vue';
import '@/assets/main.css';

/**
 * Cuánto se espera a las traducciones antes de montar.
 *
 * Se espera para que el diálogo no muestre las claves crudas, pero con plazo: si
 * el backend no contesta, es mejor un diálogo con las claves a la vista que una
 * ventana en blanco pidiendo una contraseña.
 */
const PLAZO_TRADUCCIONES_MS = 1500;

const app = createApp(App);
const pinia = createPinia();

app.use(pinia);

// Antes de montar: este diálogo aparece de golpe encima de lo que sea que estés
// haciendo y se responde en dos segundos, así que mostrar `polkit.title` y
// después corregirlo se ve peor que la espera.
//
// Un intento que falla se reintenta —el backend puede tardar o no escuchar a la
// primera—, pero la espera total sigue acotada por `PLAZO_TRADUCCIONES_MS`: un
// backend colgado tiene que dar un diálogo con las claves a la vista, no una
// ventana en blanco pidiendo una contraseña.
async function cargarTraducciones(): Promise<void> {
	const MAX_INTENTOS = 3;
	const ESPERA_BASE_MS = 500;
	const ESPERA_MAX_MS = 2000;

	const intentar = async () => {
		for (let intento = 0; intento < MAX_INTENTOS; intento++) {
			try {
				await I18n.getInstance().load();
				return;
			} catch (error) {
				console.error(
					`No se pudieron cargar las traducciones (intento ${intento + 1}/${MAX_INTENTOS}):`,
					error
				);
				if (intento === MAX_INTENTOS - 1) return;
				const espera = Math.min(ESPERA_BASE_MS * 2 ** intento, ESPERA_MAX_MS);
				await new Promise((resolve) => setTimeout(resolve, espera));
			}
		}
	};

	await Promise.race([
		intentar(),
		new Promise((resolve) => setTimeout(resolve, PLAZO_TRADUCCIONES_MS)),
	]);
}

await cargarTraducciones();

app.mount('#app');
