/**
 * Lo que los dos diálogos de contraseña dejaron de dibujar por su cuenta.
 *
 * Son dos preguntas de contraseña —la de polkit y la de un disco cifrado— y
 * las dos tenían el mismo par de defectos, que es lo que pasa cuando algo se
 * copia:
 *
 * - el campo tenía `placeholder` y nada más. Un `placeholder` no es un nombre:
 *   se va con la primera letra, y un lector de pantalla no está obligado a
 *   leerlo. O sea que el campo se anunciaba como «campo de texto» a secas, en
 *   la pantalla donde se escribe una contraseña.
 * - el error —«contraseña incorrecta»— era un renglón rojo **sin rol y sin
 *   atar al campo**. Aparecía después de intentar, así que quien no mira la
 *   pantalla escribía mal y no se enteraba de nada: el diálogo parecía no
 *   haber hecho nada.
 *
 * Con `TextInput` el campo tiene nombre, `aria-invalid` cuando falla y
 * `aria-describedby` al renglón del error; el renglón tiene `role="alert"`, que
 * es lo que lo hace sonar al aparecer.
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { olvidarLosIconosDelTema, TextInput } from '@vasakgroup/vue-libvasak';
import { mount, type VueWrapper } from '@vue/test-utils';
import PolkitModal from '@/components/PolkitModal.vue';
import UnlockModal from '@/components/UnlockModal.vue';
import { useDialogos } from '@/composables/useDialogos';
import { emitir, olvidarTodo, ponerEnElTema } from './dobles';

const RAIZ = new URL('..', import.meta.url).pathname;

let vista: VueWrapper | null = null;

/** El diálogo de polkit, abierto como lo abre polkit. */
async function pedirAutorizacion() {
	vista = mount(PolkitModal);
	await Promise.resolve();
	await emitir('polkit-request', { message: 'Se necesita autenticación', cookie: 'c' });
	await vista.vm.$nextTick();
	return vista;
}

/** El de un disco cifrado, con el reintento que trae el error puesto. */
async function pedirFrase(wrongPassphrase = false) {
	vista = mount(UnlockModal);
	await Promise.resolve();
	await emitir('unlock-request', { id: 'd1', name: 'Respaldo', wrongPassphrase });
	await vista.vm.$nextTick();
	return vista;
}

beforeEach(() => {
	ponerEnElTema('dialog-password', 'escudo.png');
	ponerEnElTema('drive-harddisk-encrypted', 'disco.png');
});

/**
 * Quién tiene la ventana, de vuelta a nadie.
 *
 * `useDialogos` guarda el estado **en el módulo** —los dos diálogos tienen que
 * poder verse entre ellos— y un módulo se comparte entre pruebas. Sin esto, la
 * prueba de polkit deja `polkitPidiendo` en `true`, el de desbloqueo calcula
 * que no le toca aparecer y dibuja **nada**: la prueba siguiente falla buscando
 * un campo que sí existe en el código. Costó un rato encontrarlo, porque cada
 * prueba pasa sola.
 */
const { polkitPidiendo, desbloqueoPidiendo } = useDialogos();

afterEach(() => {
	vista?.unmount();
	vista = null;
	polkitPidiendo.value = false;
	desbloqueoPidiendo.value = false;
	olvidarTodo();
	olvidarLosIconosDelTema();
});

describe('el campo de contraseña de polkit', () => {
	test('tiene nombre, y no sólo un texto que se va al escribir', async () => {
		const abierto = await pedirAutorizacion();

		expect(abierto.find('input[type="password"]').attributes('aria-label')).toBe(
			'polkit.password'
		);
	});

	test('es el del sistema y no uno dibujado a mano', async () => {
		const abierto = await pedirAutorizacion();

		expect(abierto.findComponent(TextInput).exists()).toBe(true);
	});

	test('sin error no dice que sea inválido', async () => {
		const abierto = await pedirAutorizacion();
		const campo = abierto.find('input[type="password"]');

		expect(campo.attributes('aria-invalid')).toBeUndefined();
	});
});

describe('la frase del disco cifrado', () => {
	test('también tiene nombre', async () => {
		const abierto = await pedirFrase();

		expect(abierto.find('input[type="password"]').attributes('aria-label')).toBe(
			'unlock.passphrase'
		);
	});

	test('y cuando la frase anterior no abrió, el error se anuncia y cuelga del campo', async () => {
		// Es el caso real: se reintenta porque la anterior falló. Sin el rol, el
		// aviso aparece en pantalla y no suena; sin el `aria-describedby`, quien
		// no la ve oye que el campo es inválido y nunca por qué.
		const abierto = await pedirFrase(true);

		const aviso = abierto.find('[role="alert"]');
		expect(aviso.exists()).toBe(true);

		const campo = abierto.find('input[type="password"]');
		expect(campo.attributes('aria-invalid')).toBe('true');
		expect(campo.attributes('aria-describedby')).toBe(aviso.attributes('id'));
	});
});

describe('lo que los diálogos ya no dibujan', () => {
	test('el composable del icono se fue', async () => {
		expect(await Bun.file(`${RAIZ}src/composables/useReactiveIcon.ts`).exists()).toBe(false);
	});

	test('y nadie lo importa', async () => {
		const fuentes = [...new Bun.Glob('src/**/*.{vue,ts}').scanSync(RAIZ)];
		expect(fuentes.length).toBeGreaterThan(5);

		const culpables: string[] = [];
		for (const ruta of fuentes) {
			const texto = await Bun.file(`${RAIZ}${ruta}`).text();
			if (/useReactiveIcon/.test(texto)) culpables.push(ruta);
		}

		expect(culpables).toEqual([]);
	});
});
