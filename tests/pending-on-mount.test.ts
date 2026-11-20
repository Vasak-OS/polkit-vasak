/**
 * La ventana se recarga al mostrarse (su proceso de WebKit se mata escondida) y
 * el evento del pedido en vuelo se pierde. Por eso los diálogos preguntan al
 * montar qué hay pendiente: `polkit_pending` / `unlock_pending`.
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { forgetThemeIcons } from '@vasakgroup/vue-libvasak';
import { mount, type VueWrapper } from '@vue/test-utils';
import PolkitModal from '@/components/PolkitModal.vue';
import UnlockModal from '@/components/UnlockModal.vue';
import { useDialogs } from '@/composables/useDialogs';
import { emit, forgetEverything, invocations, putInTheme, returns } from './doubles';

const { polkitAsking, unlockAsking } = useDialogs();
const views: VueWrapper[] = [];

async function mountAndSettle(component: typeof PolkitModal | typeof UnlockModal) {
	const view = mount(component, { attachTo: document.body });
	views.push(view);
	// Sin ningún `emit`: el pedido sólo puede llegar por la pregunta al montar.
	for (let i = 0; i < 5; i++) await view.vm.$nextTick();
	return view;
}

/**
 * Monta y deja llegar un evento justo cuando la pregunta al montar ya salió pero
 * todavía no volvió: es la carrera que la guarda tiene que ganar.
 */
async function mountWithEventDuringPull(
	component: typeof PolkitModal | typeof UnlockModal,
	pull: string,
	event: string,
	payload: unknown,
) {
	const view = mount(component, { attachTo: document.body });
	views.push(view);
	while (!invocations.includes(pull)) await Promise.resolve();
	await emit(event, payload);
	for (let i = 0; i < 5; i++) await view.vm.$nextTick();
	return view;
}

beforeEach(() => {
	putInTheme('dialog-password', 'escudo.png');
});

afterEach(() => {
	for (const view of views.splice(0)) view.unmount();
	polkitAsking.value = false;
	unlockAsking.value = false;
	forgetEverything();
	forgetThemeIcons();
});

describe('pedido pendiente al montar', () => {
	test('polkit muestra el pedido pendiente sin que llegue el evento', async () => {
		returns.set('polkit_pending', { message: 'Instalar paquete', cookie: 'c9' });

		const view = await mountAndSettle(PolkitModal);

		expect(invocations).toContain('polkit_pending');
		expect(polkitAsking.value).toBe(true);
		expect(view.text()).toContain('Instalar paquete');
	});

	test('polkit sin pedido pendiente se queda escondido', async () => {
		await mountAndSettle(PolkitModal);

		expect(invocations).toContain('polkit_pending');
		expect(polkitAsking.value).toBe(false);
	});

	test('el desbloqueo muestra el disco pendiente sin que llegue el evento', async () => {
		returns.set('unlock_pending', { id: 'd7', name: 'Respaldo', wrongPassphrase: false });

		await mountAndSettle(UnlockModal);

		expect(invocations).toContain('unlock_pending');
		expect(unlockAsking.value).toBe(true);
		expect(document.querySelector('input[type="password"]')).not.toBeNull();
	});

	test('el desbloqueo pendiente tras una frase equivocada muestra el error', async () => {
		returns.set('unlock_pending', { id: 'd7', name: 'Respaldo', wrongPassphrase: true });

		const view = await mountAndSettle(UnlockModal);

		expect(view.text()).toContain('unlock.wrongPassphrase');
	});

	test('el desbloqueo sin pedido pendiente se queda escondido', async () => {
		await mountAndSettle(UnlockModal);

		expect(invocations).toContain('unlock_pending');
		expect(unlockAsking.value).toBe(false);
	});

	test('polkit: un evento durante el montaje gana al pedido pendiente, que ya quedó viejo', async () => {
		returns.set('polkit_pending', { message: 'Pedido viejo', cookie: 'viejo' });

		const view = await mountWithEventDuringPull(PolkitModal, 'polkit_pending', 'polkit-request', {
			message: 'Pedido nuevo',
			cookie: 'nuevo',
		});

		expect(view.text()).toContain('Pedido nuevo');
		expect(view.text()).not.toContain('Pedido viejo');
	});

	test('polkit: la cancelación de OTRO pedido durante el montaje no descarta el pendiente', async () => {
		returns.set('polkit_pending', { message: 'Pedido A', cookie: 'A' });

		const view = await mountWithEventDuringPull(PolkitModal, 'polkit_pending', 'polkit-cancel', {
			cookie: 'B',
		});

		expect(polkitAsking.value).toBe(true);
		expect(view.text()).toContain('Pedido A');
	});

	test('polkit: la cancelación del MISMO pedido durante el montaje descarta el pendiente', async () => {
		returns.set('polkit_pending', { message: 'Pedido A', cookie: 'A' });

		const view = await mountWithEventDuringPull(PolkitModal, 'polkit_pending', 'polkit-cancel', {
			cookie: 'A',
		});

		expect(polkitAsking.value).toBe(false);
		expect(view.text()).not.toContain('Pedido A');
	});

	test('polkit: un reintento del mismo pedido durante el montaje no descarta el pendiente', async () => {
		returns.set('polkit_pending', { message: 'Pedido A', cookie: 'A' });

		const view = await mountWithEventDuringPull(PolkitModal, 'polkit_pending', 'polkit-result', {
			cookie: 'A',
			success: false,
			retry: true,
			message: 'Contraseña equivocada',
		});

		expect(polkitAsking.value).toBe(true);
		expect(view.text()).toContain('Pedido A');
	});

	test('el desbloqueo: un evento durante el montaje gana al disco pendiente, que ya quedó viejo', async () => {
		returns.set('unlock_pending', { id: 'viejo', name: 'Disco viejo', wrongPassphrase: true });

		const view = await mountWithEventDuringPull(UnlockModal, 'unlock_pending', 'unlock-request', {
			id: 'nuevo',
			name: 'Disco nuevo',
			wrongPassphrase: false,
		});

		// El nombre no se ve con el `t()` de pruebas: lo que distingue al pedido
		// viejo del nuevo es el error de frase equivocada que sólo el viejo trae.
		expect(unlockAsking.value).toBe(true);
		expect(view.text()).not.toContain('unlock.wrongPassphrase');
	});
});
