/**
 * El teclado en los dos diálogos: adónde va el foco, Enter y Escape.
 *
 * Es lo que más se usa de este diálogo y lo que ninguna captura muestra: llega
 * encima de lo que se esté haciendo, se escribe la contraseña sin tocar el
 * ratón y se contesta con Enter o se descarta con Escape. Pasar los botones y
 * el campo a la librería cambia justo los elementos de los que eso depende.
 *
 * # Cómo se prueba sin mentir
 *
 * Un `KeyboardEvent` despachado a mano no hace lo que hace el navegador con la
 * tecla (memoria `el-tab-despachado-a-mano-no-navega`), así que una prueba que
 * despacha la tecla y mira que «no pasó nada malo» pasa siempre. Lo que se
 * comprueba acá, en cada caso, es **quién se quedó con la tecla**
 * (`defaultPrevented`, con `cancelable: true`) y **adónde fue el foco**
 * (`document.activeElement`).
 *
 * Enter tampoco se despacha: happy-dom no hace el envío implícito. Lo que hace
 * el navegador, según HTML, es buscar el **botón por omisión** del formulario
 * —el primer botón de envío en el orden del documento— y, si no está apagado,
 * hacerle clic. Eso es lo que se reproduce: se busca ese botón como lo busca el
 * navegador, se comprueba que sea Aceptar y no Cancelar, y se le hace clic.
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { olvidarLosIconosDelTema as forgetThemeIcons } from '@vasakgroup/vue-libvasak';
import { mount, type VueWrapper } from '@vue/test-utils';
import PolkitModal from '@/components/PolkitModal.vue';
import UnlockModal from '@/components/UnlockModal.vue';
import { useDialogs } from '@/composables/useDialogs';
import { emit, forgetEverything, invocationArgs, invocations, putInTheme } from './doubles';

const views: VueWrapper[] = [];

function mountAttached(component: typeof PolkitModal | typeof UnlockModal) {
	// Enganchado al documento: fuera de él no hay `activeElement` que mirar.
	const view = mount(component, { attachTo: document.body });
	views.push(view);
	return view;
}

async function askForAuth() {
	const view = mountAttached(PolkitModal);
	await Promise.resolve();
	await emit('polkit-request', { message: 'Se necesita autenticación', cookie: 'c' });
	await view.vm.$nextTick();
	await view.vm.$nextTick();
	return view;
}

async function askForPassphrase() {
	const view = mountAttached(UnlockModal);
	await Promise.resolve();
	await emit('unlock-request', { id: 'd1', name: 'Respaldo', wrongPassphrase: false });
	await view.vm.$nextTick();
	await view.vm.$nextTick();
	return view;
}

function passwordField(): HTMLInputElement {
	const field = document.querySelector<HTMLInputElement>('input[type="password"]');
	if (!field) throw new Error('no hay campo de contraseña a la vista');
	return field;
}

/** El botón que el navegador aprieta con Enter: el primer botón de envío del formulario. */
function defaultButton(form: HTMLFormElement): HTMLButtonElement | null {
	return form.querySelector<HTMLButtonElement>('button[type="submit"], input[type="submit"]');
}

/** Enter en el campo, como lo hace el navegador: clic al botón por omisión si no está apagado. */
async function pressEnterIn(field: HTMLInputElement) {
	const form = field.form;
	if (!form) throw new Error('el campo no está en un formulario: Enter no envía nada');
	const button = defaultButton(form);
	let submitted: Event | null = null;
	form.addEventListener('submit', (event) => (submitted = event), { once: true });
	if (button && !button.disabled) button.click();
	await Promise.resolve();
	return submitted as Event | null;
}

function pressEscapeIn(target: Element): KeyboardEvent {
	const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
	target.dispatchEvent(event);
	return event;
}

async function type(field: HTMLInputElement, value: string) {
	field.value = value;
	field.dispatchEvent(new Event('input', { bubbles: true }));
	await Promise.resolve();
}

const { polkitAsking, unlockAsking } = useDialogs();

beforeEach(() => {
	putInTheme('dialog-password', 'escudo.png');
	putInTheme('drive-harddisk-encrypted', 'disco.png');
});

afterEach(() => {
	for (const view of views.splice(0)) view.unmount();
	polkitAsking.value = false;
	unlockAsking.value = false;
	forgetEverything();
	forgetThemeIcons();
	(document.activeElement as HTMLElement | null)?.blur?.();
});

describe('polkit: el foco', () => {
	test('al pedir la contraseña, el cursor ya está en el campo', async () => {
		await askForAuth();

		expect(document.activeElement).toBe(passwordField());
	});

	test('después de una contraseña equivocada vuelve al campo, venga de donde venga', async () => {
		// Con el ratón el foco queda en Aceptar, que se apaga mientras verifica;
		// el reintento tiene que poder escribirse sin volver a hacer clic.
		const view = await askForAuth();
		const field = passwordField();
		await type(field, 'mala');
		await pressEnterIn(field);
		const cancel = view.findAll('button').find((button) => button.text() === 'polkit.cancel');
		(cancel?.element as HTMLButtonElement).focus();
		expect(document.activeElement).not.toBe(field);

		await emit('polkit-result', { success: false, message: '' });
		await view.vm.$nextTick();
		await view.vm.$nextTick();

		expect(document.activeElement).toBe(passwordField());
	});
});

describe('polkit: Enter', () => {
	test('el botón que aprieta Enter es Aceptar, no Cancelar', async () => {
		// Cancelar va primero en el documento: si fuera de envío, Enter
		// cancelaría el pedido en vez de mandar la contraseña.
		await askForAuth();
		const button = defaultButton(passwordField().form as HTMLFormElement);

		expect(button?.textContent?.trim()).toBe('polkit.accept');
		const cancel = [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'polkit.cancel');
		expect(cancel?.getAttribute('type')).toBe('button');
	});

	test('con la contraseña escrita, envía sin recargar la ventana y el foco se queda', async () => {
		await askForAuth();
		const field = passwordField();
		await type(field, 'secreta');

		const submitted = await pressEnterIn(field);

		expect(submitted).not.toBeNull();
		// Sin el `preventDefault` el formulario navega y la ventana del diálogo
		// se recarga en blanco con la contraseña en la dirección.
		expect(submitted?.defaultPrevented).toBe(true);
		expect(invocations).toContain('submit_password');
		expect(invocationArgs[invocations.indexOf('submit_password')]).toEqual({
			password: 'secreta',
			cookie: 'c',
		});
		expect(document.activeElement).toBe(field);
	});

	test('con el campo vacío no envía nada', async () => {
		await askForAuth();
		const field = passwordField();

		expect(defaultButton(field.form as HTMLFormElement)?.disabled).toBe(true);
		expect(await pressEnterIn(field)).toBeNull();
		expect(invocations).not.toContain('submit_password');
	});

	test('un segundo Enter mientras verifica no la manda dos veces', async () => {
		await askForAuth();
		const field = passwordField();
		await type(field, 'secreta');
		await pressEnterIn(field);
		await Promise.resolve();

		expect(await pressEnterIn(field)).toBeNull();
		expect(invocations.filter((command) => command === 'submit_password')).toHaveLength(1);
	});
});

describe('polkit: Escape', () => {
	test('cancela el pedido, se queda con la tecla y cierra el diálogo', async () => {
		const view = await askForAuth();

		const event = pressEscapeIn(passwordField());
		await view.vm.$nextTick();

		expect(event.defaultPrevented).toBe(true);
		expect(invocations).toContain('cancel_pending');
		expect(invocationArgs[invocations.indexOf('cancel_pending')]).toEqual({ cookie: 'c' });
		expect(document.querySelector('input[type="password"]')).toBeNull();
	});

	test('otra tecla no la toca', async () => {
		await askForAuth();
		const event = new KeyboardEvent('keydown', { key: 'a', bubbles: true, cancelable: true });

		passwordField().dispatchEvent(event);

		expect(event.defaultPrevented).toBe(false);
		expect(invocations).not.toContain('cancel_pending');
	});
});

describe('el disco cifrado', () => {
	test('el cursor arranca en la frase', async () => {
		await askForPassphrase();

		expect(document.activeElement).toBe(passwordField());
	});

	test('Enter envía la frase sin recargar y el foco se queda', async () => {
		await askForPassphrase();
		const field = passwordField();
		await type(field, 'frase');
		expect(defaultButton(field.form as HTMLFormElement)?.textContent?.trim()).toBe('unlock.accept');

		const submitted = await pressEnterIn(field);

		expect(submitted?.defaultPrevented).toBe(true);
		expect(invocations).toContain('enviar_frase');
		expect(document.activeElement).toBe(field);
	});

	test('Escape cancela el desbloqueo y se queda con la tecla', async () => {
		const view = await askForPassphrase();

		const event = pressEscapeIn(passwordField());
		await view.vm.$nextTick();

		expect(event.defaultPrevented).toBe(true);
		expect(invocations).toContain('cancelar_desbloqueo');
		expect(document.querySelector('input[type="password"]')).toBeNull();
	});

	test('Espacio sobre la casilla la marca: es un control del teclado de verdad', async () => {
		// Es un `input` nativo con el dibujo apagado, así que el navegador pone
		// el Espacio. Lo que se comprueba es que llega al teclado —que se puede
		// enfocar— y que marcarla cambia lo que se manda.
		await askForPassphrase();
		const box = document.querySelector<HTMLInputElement>('input[type="checkbox"]');
		box?.focus();

		expect(document.activeElement).toBe(box);
		box?.click();
		await type(passwordField(), 'frase');
		await pressEnterIn(passwordField());

		expect(invocationArgs[invocations.indexOf('enviar_frase')]).toMatchObject({ recordar: true });
	});
});

describe('sin nada que preguntar', () => {
	for (const [name, component] of [
		['polkit', PolkitModal],
		['el disco', UnlockModal],
	] as const) {
		test(`${name} montado y escondido no se queda con el Escape`, async () => {
			// La ventana vive todo el tiempo, con los dos diálogos montados y
			// escondidos. Un Escape que llegue entonces no es de ninguno.
			const view = mountAttached(component);
			await Promise.resolve();
			await view.vm.$nextTick();

			const event = pressEscapeIn(document.body);

			expect(event.defaultPrevented).toBe(false);
			expect(invocations).toEqual([]);
		});
	}
});

describe('con los dos pedidos vivos', () => {
	/**
	 * Los dos montados primero y los pedidos después, como en la ventana real:
	 * `App.vue` monta los dos diálogos al arrancar y los pedidos llegan más
	 * tarde. El orden de montaje es el orden de los oyentes del `document`, y
	 * es lo que decide quién recibe el Escape primero.
	 */
	async function bothAsking(order: 'app' | 'reverse') {
		const components = order === 'app' ? [PolkitModal, UnlockModal] : [UnlockModal, PolkitModal];
		const mounted = components.map((component) => mountAttached(component));
		await Promise.resolve();
		await emit('unlock-request', { id: 'd1', name: 'Respaldo', wrongPassphrase: false });
		await emit('polkit-request', { message: 'Se necesita autenticación', cookie: 'c' });
		for (const view of mounted) await view.vm.$nextTick();
		return mounted[0] as VueWrapper;
	}

	for (const order of ['app', 'reverse'] as const) {
		test(`Escape cancela sólo el que se ve (montados ${order === 'app' ? 'como en App.vue' : 'al revés'})`, async () => {
			// Abrir un disco interno pide la frase y, adentro del desbloqueo, la
			// autorización de polkit. Los dos escuchan el mismo `document`: el
			// Escape sobre el de polkit no puede cancelar también el de abajo,
			// que nadie estaba mirando. En el orden de `App.vue` el de polkit
			// cancela primero y deja al otro como visible en el mismo despacho;
			// la prueba que montaba al revés no lo veía (lo marcó CodeRabbit).
			const view = await bothAsking(order);
			expect(passwordField().getAttribute('aria-label')).toBe('polkit.password');

			const event = pressEscapeIn(passwordField());
			await view.vm.$nextTick();

			expect(event.defaultPrevented).toBe(true);
			expect(invocations).toContain('cancel_pending');
			expect(invocations).not.toContain('cancelar_desbloqueo');
			// Y el de abajo vuelve a la vista, con su campo.
			expect(passwordField().getAttribute('aria-label')).toBe('unlock.passphrase');
		});
	}
});
