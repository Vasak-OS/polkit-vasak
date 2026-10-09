import { describe, expect, test } from 'bun:test';
import { loadTranslations } from '@/tools/translations';

describe('loadTranslations', () => {
	test('carga a la primera y no reintenta cuando el backend responde', async () => {
		let llamadas = 0;
		await loadTranslations({
			baseDelayMs: 0,
			maxDelayMs: 0,
			load: async () => {
				llamadas++;
			},
		});
		expect(llamadas).toBe(1);
	});

	test('reintenta y termina cargando cuando los primeros intentos fallan', async () => {
		let llamadas = 0;
		await loadTranslations({
			baseDelayMs: 0,
			maxDelayMs: 0,
			load: async () => {
				llamadas++;
				if (llamadas < 3) throw new Error('el backend todavía no escucha');
			},
		});
		expect(llamadas).toBe(3);
	});

	test('se rinde sin lanzar tras agotar los intentos', async () => {
		let llamadas = 0;
		await loadTranslations({
			maxAttempts: 3,
			baseDelayMs: 0,
			maxDelayMs: 0,
			load: async () => {
				llamadas++;
				throw new Error('el backend está caído');
			},
		});
		// No relanza: el diálogo tiene que montarse igual, con las claves a la vista.
		expect(llamadas).toBe(3);
	});

	test('monta igual cuando la carga se cuelga y vence el plazo', async () => {
		let resuelto = false;
		await loadTranslations({
			timeoutMs: 10,
			// Una carga que no termina nunca: el plazo tiene que ganar la carrera.
			load: () => new Promise<void>(() => {}),
		});
		resuelto = true;
		expect(resuelto).toBe(true);
	});
});
