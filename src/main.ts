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
await Promise.race([
	I18n.getInstance()
		.load()
		.catch((error) => {
			console.error('No se pudieron cargar las traducciones', error);
		}),
	new Promise((resolve) => setTimeout(resolve, PLAZO_TRADUCCIONES_MS)),
]);

app.mount('#app');
