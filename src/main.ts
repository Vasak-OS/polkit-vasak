import { createPinia } from 'pinia';
import { createApp } from 'vue';
import App from '@/App.vue';
import '@/assets/main.css';
import { loadTranslations } from '@/tools/translations';

const app = createApp(App);
const pinia = createPinia();

app.use(pinia);

// Las traducciones se cargan con reintentos y plazo acotado antes de montar: ver
// `loadTranslations`. Si el backend se cuelga, la ventana se monta igual con las
// claves a la vista en vez de quedar en blanco pidiendo una contraseña.
await loadTranslations();

app.mount('#app');
