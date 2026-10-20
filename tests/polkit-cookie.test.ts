/**
 * El diálogo es uno solo y polkit puede tener varios pedidos vivos: lo que
 * termina o se cancela con **otra** cookie no tiene que tocar al que está a la
 * vista (`polkit-vasak#37`).
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { forgetThemeIcons } from '@vasakgroup/vue-libvasak';
import { mount, type VueWrapper } from '@vue/test-utils';
import PolkitModal from '@/components/PolkitModal.vue';
import { useDialogs } from '@/composables/useDialogs';
import { emit, forgetEverything, putInTheme, rejections } from './doubles';

const { polkitAsking } = useDialogs();
const views: VueWrapper[] = [];

function passwordField() {
	return document.querySelector('input') as HTMLInputElement;
}

async function askFor(cookie: string) {
	const view = mount(PolkitModal, { attachTo: document.body });
	views.push(view);
	await Promise.resolve();
	await emit('polkit-request', { message: 'Se necesita autenticación', cookie });
	await view.vm.$nextTick();
	await view.vm.$nextTick();
	return view;
}

async function typePassword(view: VueWrapper, value: string) {
	const field = passwordField();
	field.value = value;
	field.dispatchEvent(new Event('input', { bubbles: true }));
	await view.vm.$nextTick();
}

beforeEach(() => {
	putInTheme('dialog-password', 'escudo.png');
});

afterEach(() => {
	for (const view of views.splice(0)) view.unmount();
	polkitAsking.value = false;
	forgetEverything();
	forgetThemeIcons();
});

describe('polkit: varios pedidos, un solo diálogo', () => {
	test('un resultado de otra cookie no cierra ni marca error en el diálogo a la vista', async () => {
		const view = await askFor('c1');
		await typePassword(view, 'secreta');

		await emit('polkit-result', { success: true, cookie: 'otra', message: '' });
		await emit('polkit-result', { success: false, cookie: 'otra', message: 'mal' });
		await view.vm.$nextTick();

		expect(polkitAsking.value).toBe(true);
		expect(passwordField().value).toBe('secreta');
		expect(view.text()).not.toContain('mal');
	});

	test('el resultado de la cookie a la vista sí cierra el diálogo', async () => {
		const view = await askFor('c1');

		await emit('polkit-result', { success: true, cookie: 'c1', message: '' });
		await view.vm.$nextTick();

		expect(polkitAsking.value).toBe(false);
	});

	test('una cancelación de otra cookie no cierra el diálogo ni borra lo escrito', async () => {
		const view = await askFor('c1');
		await typePassword(view, 'secreta');

		await emit('polkit-cancel', { cookie: 'otra' });
		await view.vm.$nextTick();

		expect(polkitAsking.value).toBe(true);
		expect(passwordField().value).toBe('secreta');
	});

	test('la cancelación de la cookie a la vista cierra el diálogo y borra la contraseña', async () => {
		const view = await askFor('c1');
		await typePassword(view, 'secreta');

		await emit('polkit-cancel', { cookie: 'c1' });
		await view.vm.$nextTick();

		expect(polkitAsking.value).toBe(false);
		await emit('polkit-request', { message: 'otro', cookie: 'c2' });
		await view.vm.$nextTick();
		expect(passwordField().value).toBe('');
	});
});

describe('polkit: un envío que el backend rechaza', () => {
	test('muestra el mensaje traducido y no el texto crudo del backend', async () => {
		rejections.set('submit_password', 'No hay un pedido pendiente');
		const view = await askFor('c1');
		await typePassword(view, 'secreta');

		document.querySelector('form')?.dispatchEvent(new Event('submit', { cancelable: true }));
		await Promise.resolve();
		await view.vm.$nextTick();

		expect(view.text()).toContain('polkit.sendError');
		expect(view.text()).not.toContain('No hay un pedido pendiente');
	});
});
