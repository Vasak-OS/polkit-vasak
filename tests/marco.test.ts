/**
 * El diálogo dibuja el marco compartido, no uno propio.
 *
 * Este diálogo aparece encima de lo que sea que estés haciendo, así que es
 * donde más se nota si el borde, la esquina o el fondo no son los mismos que
 * los del resto del escritorio. Tenía los suyos copiados a mano, y ya habían
 * derivado.
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { WindowFrame } from '@vasakgroup/vue-libvasak';
import { mount, type VueWrapper } from '@vue/test-utils';
import PolkitModal from '@/components/PolkitModal.vue';
import { emitir, olvidarTodo, ponerEnElTema } from './dobles';

let vista: VueWrapper | null = null;

/** Abre el diálogo como lo abre polkit: mandando un pedido. */
async function pedirAutorizacion() {
	vista = mount(PolkitModal);
	// `onMounted` se suscribe con un `await` adentro, así que el oyente no está
	// puesto todavía cuando `mount` vuelve.
	await Promise.resolve();
	await emitir('polkit-request', { message: 'Se necesita autenticación', cookie: 'c' });
	await vista.vm.$nextTick();
	return vista;
}

beforeEach(() => {
	ponerEnElTema('security-high', 'escudo.png');
});

afterEach(() => {
	vista?.unmount();
	vista = null;
	olvidarTodo();
});

describe('el marco del diálogo', () => {
	test('es el compartido y no uno copiado a mano', async () => {
		const abierto = await pedirAutorizacion();

		expect(abierto.findComponent(WindowFrame).exists()).toBe(true);
	});

	test('va sin barra', async () => {
		// No se minimiza ni se cierra desde un botón: se responde. Una barra con
		// los tres controles de ventana acá sería una salida sin respuesta.
		const abierto = await pedirAutorizacion();

		expect(abierto.findComponent(WindowFrame).props('hideBar')).toBe(true);
	});

	test('el temblor del error llega al marco', async () => {
		// La contraseña equivocada sacude la ventana entera. Al envolver el
		// contenido en el marco, la clase se quedaba en el div de adentro y el
		// temblor no se veía: un `div` con el alto del contenido moviéndose
		// dentro de una ventana quieta no se nota.
		const abierto = await pedirAutorizacion();
		const marco = abierto.findComponent(WindowFrame);

		expect(marco.classes()).not.toContain('animate-shake');

		await emitir('polkit-result', { success: false, message: '' });
		await abierto.vm.$nextTick();

		expect(abierto.findComponent(WindowFrame).classes()).toContain('animate-shake');
	});
});

describe('lo que ya no está', () => {
	test('no queda ninguna ventana dibujada a mano', async () => {
		// `rounded-corner-window` es el borde de la ventana: sale del marco
		// compartido y de ningún otro lado. Con dos, la esquina y el fondo se
		// dibujan dos veces y se ven los dos.
		const abierto = await pedirAutorizacion();

		expect(abierto.findAll('.rounded-corner-window').length).toBe(1);
	});
});
