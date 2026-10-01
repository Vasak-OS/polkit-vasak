/**
 * Lo que los dos diálogos de contraseña dejaron de dibujar por su cuenta.
 *
 * Son dos preguntas de contraseña —la de polkit y la de un disco cifrado— y
 * las dos tenían los mismos defectos, que es lo que pasa cuando algo se copia:
 *
 * - el campo tenía `placeholder` y nada más. Un `placeholder` no es un nombre:
 *   se va con la primera letra, y un lector de pantalla no está obligado a
 *   leerlo.
 * - el error —«contraseña incorrecta»— era un renglón rojo escrito a mano. Ahora
 *   lo dibuja `FormGroup`, que lo ata al campo (`aria-describedby`) y lo anuncia
 *   al aparecer (`aria-live`), y que además lo deja en el color del texto: el
 *   rojo de fábrica sobre la ventana no llega a 4,5:1.
 * - los botones, la casilla «Recordarla en el llavero», el encabezado y el
 *   marco del diálogo del disco estaban dibujados a mano (vue-libvasak#74).
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import {
	ActionButton,
	Checkbox,
	FormGroup,
	forgetThemeIcons,
	SectionHeading,
	TextInput,
	WindowFrame,
} from '@vasakgroup/vue-libvasak';
import { mount, type VueWrapper } from '@vue/test-utils';
import PolkitModal from '@/components/PolkitModal.vue';
import UnlockModal from '@/components/UnlockModal.vue';
import { useDialogs } from '@/composables/useDialogs';
import { emit, forgetEverything, putInTheme } from './doubles';

const ROOT = new URL('..', import.meta.url).pathname;

let mounted: VueWrapper | null = null;

/** El diálogo de polkit, abierto como lo abre polkit. */
async function askForAuth() {
	mounted = mount(PolkitModal, { attachTo: document.body });
	await Promise.resolve();
	await emit('polkit-request', { message: 'Se necesita autenticación', cookie: 'c' });
	await mounted.vm.$nextTick();
	return mounted;
}

/** El de un disco cifrado, con el reintento que trae el error puesto. */
async function askForPassphrase(wrongPassphrase = false) {
	mounted = mount(UnlockModal, { attachTo: document.body });
	await Promise.resolve();
	await emit('unlock-request', { id: 'd1', name: 'Respaldo', wrongPassphrase });
	await mounted.vm.$nextTick();
	return mounted;
}

/** El renglón del error de `FormGroup`: existe siempre, vacío y oculto sin error. */
function errorLine(view: VueWrapper) {
	return view.find('[aria-live="polite"]');
}

beforeEach(() => {
	putInTheme('dialog-password', 'escudo.png');
	putInTheme('drive-harddisk-encrypted', 'disco.png');
});

/**
 * Quién tiene la ventana, de vuelta a nadie.
 *
 * `useDialogs` guarda el estado **en el módulo** —los dos diálogos tienen que
 * poder verse entre ellos— y un módulo se comparte entre pruebas. Sin esto, la
 * prueba de polkit deja `polkitAsking` en `true`, el de desbloqueo calcula que
 * no le toca aparecer y dibuja **nada**: la prueba siguiente falla buscando un
 * campo que sí existe en el código.
 */
const { polkitAsking, unlockAsking } = useDialogs();

afterEach(() => {
	mounted?.unmount();
	mounted = null;
	polkitAsking.value = false;
	unlockAsking.value = false;
	forgetEverything();
	forgetThemeIcons();
});

describe('el campo de contraseña de polkit', () => {
	test('tiene nombre, y no sólo un texto que se va al escribir', async () => {
		const opened = await askForAuth();

		expect(opened.find('input[type="password"]').attributes('aria-label')).toBe('polkit.password');
	});

	test('es el del sistema, dentro del grupo que ata el error', async () => {
		const opened = await askForAuth();

		expect(opened.findComponent(TextInput).exists()).toBe(true);
		expect(opened.findComponent(FormGroup).findComponent(TextInput).exists()).toBe(true);
	});

	test('sin error no dice que sea inválido ni apunta a ningún renglón', async () => {
		// `FormGroup` deja el renglón del error siempre puesto —oculto, para que
		// la región ya exista cuando el error llega y se anuncie—, así que lo que
		// hay que vigilar es que el campo no lo nombre mientras está vacío: un
		// `aria-describedby` hacia un renglón vacío no es neutro.
		const opened = await askForAuth();
		const field = opened.find('input[type="password"]');

		expect(field.attributes('aria-invalid')).toBeUndefined();
		expect(field.attributes('aria-describedby')).toBeUndefined();
		expect(errorLine(opened).text()).toBe('');
	});

	test('con la contraseña equivocada, el error se anuncia y cuelga del campo', async () => {
		const opened = await askForAuth();

		await emit('polkit-result', { success: false, message: '' });
		await opened.vm.$nextTick();

		const line = errorLine(opened);
		expect(line.text()).toBe('polkit.wrongPassword');
		const field = opened.find('input[type="password"]');
		expect(field.attributes('aria-invalid')).toBe('true');
		expect(field.attributes('aria-describedby')).toBe(line.attributes('id'));
	});
});

describe('la frase del disco cifrado', () => {
	test('también tiene nombre', async () => {
		const opened = await askForPassphrase();

		expect(opened.find('input[type="password"]').attributes('aria-label')).toBe('unlock.passphrase');
	});

	test('y sin error tampoco cuelga de un renglón', async () => {
		const opened = await askForPassphrase();

		expect(errorLine(opened).text()).toBe('');
		expect(opened.find('input[type="password"]').attributes('aria-describedby')).toBeUndefined();
	});

	test('y cuando la frase anterior no abrió, el error se anuncia y cuelga del campo', async () => {
		// Es el caso real: se reintenta porque la anterior falló. Sin la región
		// viva el aviso aparece y no suena; sin el `aria-describedby`, quien no
		// lo ve oye que el campo es inválido y nunca por qué.
		const opened = await askForPassphrase(true);

		const line = errorLine(opened);
		expect(line.text()).toBe('unlock.wrongPassphrase');

		const field = opened.find('input[type="password"]');
		expect(field.attributes('aria-invalid')).toBe('true');
		expect(field.attributes('aria-describedby')).toBe(line.attributes('id'));
	});

	test('la casilla queda entre el campo y el error, como estaba', async () => {
		// El formato de la pantalla no cambia (decisión 8): meter la casilla en
		// el grupo es lo que la deja en su lugar.
		const opened = await askForPassphrase(true);
		const group = opened.findComponent(FormGroup).element;
		const order = [...group.querySelectorAll('input, [aria-live]')].map((node) =>
			node.getAttribute('type') ?? 'error',
		);

		expect(order).toEqual(['password', 'checkbox', 'error']);
	});

	test('y «recordar» viaja con la frase', async () => {
		const opened = await askForPassphrase();
		const { invocations, invocationArgs } = await import('./doubles');

		await opened.find('input[type="password"]').setValue('frase');
		await opened.find('input[type="checkbox"]').setValue(true);
		await opened.find('form').trigger('submit');

		expect(invocations).toContain('enviar_frase');
		expect(invocationArgs[invocations.indexOf('enviar_frase')]).toEqual({
			id: 'd1',
			frase: 'frase',
			recordar: true,
		});
	});
});

describe('las piezas son las de la librería', () => {
	for (const [name, open] of [
		['polkit', askForAuth],
		['el disco', () => askForPassphrase()],
	] as const) {
		test(`en ${name}: los dos botones, el encabezado y el marco`, async () => {
			const opened = await open();

			expect(opened.findAllComponents(ActionButton)).toHaveLength(2);
			expect(opened.findComponent(SectionHeading).exists()).toBe(true);
			expect(opened.findComponent(WindowFrame).props('hideBar')).toBe(true);
			// Ningún botón dibujado a mano al lado de los de la librería.
			const own = opened.findAll('button').filter((button) => !button.classes().includes('rounded-corner-m'));
			expect(own).toHaveLength(0);
		});
	}

	for (const [name, open] of [
		['polkit', askForAuth],
		['el disco', () => askForPassphrase()],
	] as const) {
		test(`en ${name}, lo angosto va en una columna, por contenedor y no por pantalla`, async () => {
			// WebKitGTK no avisa de `resize` ni de `matchMedia`, así que todo cuelga
			// de una consulta de contenedor sobre el propio diálogo. Desde 24rem
			// (la ventana mide 400) es lo de siempre; más angosto, una columna por
			// vez: el icono arriba, el mensaje entero, los botones apilados y a
			// todo el ancho, y lo que no entra se desplaza en vez de cortarse.
			const opened = await open();
			for (let i = 0; i < 20 && !opened.find('img').exists(); i++) {
				await new Promise((resolve) => setTimeout(resolve, 0));
			}
			const icon = opened.find('img');
			// El icono no se esconde en ningún ancho.
			expect(icon.classes()).not.toContain('hidden');

			const body = icon.element.parentElement as HTMLElement;
			expect(body.closest('.\\@container')).not.toBeNull();
			expect([...body.classList]).toEqual(
				expect.arrayContaining(['flex-col', 'overflow-y-auto', 'min-h-0', '@[24rem]:flex-row']),
			);

			// El mensaje sólo se recorta en el ancho de la ventana.
			const message = opened.find('p[title]');
			expect([...message.classes()].filter((name) => name.includes('line-clamp'))).toEqual([
				expect.stringMatching(/^@\[24rem\]:line-clamp-\d$/),
			]);

			const row = opened.findComponent(ActionButton).element.parentElement as HTMLElement;
			expect([...row.classList]).toEqual(expect.arrayContaining(['flex-col', '@[24rem]:flex-row']));
			for (const button of opened.findAllComponents(ActionButton)) {
				expect(button.classes()).toEqual(expect.arrayContaining(['w-full', '@[24rem]:w-auto', 'shrink-0']));
			}
		});
	}

	test('la casilla del llavero es la del sistema', async () => {
		const opened = await askForPassphrase();

		const box = opened.findComponent(Checkbox);
		expect(box.exists()).toBe(true);
		expect(box.props('label')).toBe('unlock.remember');
	});

	test('mientras verifica, el botón muestra la rueda y lo dice', async () => {
		// Antes cambiaba sólo el texto. Ahora además gira la rueda del tema y
		// el botón queda apagado: un segundo Enter no manda la contraseña dos
		// veces.
		const opened = await askForAuth();

		await opened.find('input[type="password"]').setValue('secreta');
		await opened.find('form').trigger('submit');

		const accept = opened.findAllComponents(ActionButton)[1];
		expect(accept?.props('loading')).toBe(true);
		expect(accept?.text()).toContain('polkit.checking');
		expect(accept?.find('button').attributes('disabled')).toBeDefined();
	});
});

describe('lo que los diálogos ya no dibujan', () => {
	test('el composable del icono se fue', async () => {
		expect(await Bun.file(`${ROOT}src/composables/useReactiveIcon.ts`).exists()).toBe(false);
	});

	test('ni una casilla, un botón ni un encabezado a mano en los fuentes', async () => {
		const sources = [...new Bun.Glob('src/**/*.{vue,ts}').scanSync(ROOT)];
		expect(sources.length).toBeGreaterThan(5);

		const culprits: string[] = [];
		for (const path of sources) {
			const text = await Bun.file(`${ROOT}${path}`).text();
			if (/useReactiveIcon|<button[\s>]|type="checkbox"|uppercase|\.enfocar\(/.test(text)) culprits.push(path);
		}

		expect(culprits).toEqual([]);
	});
});
