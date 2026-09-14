import { describe, expect, test } from 'bun:test';
import { dialogoVisible } from '@/tools/dialogos';

describe('dialogoVisible', () => {
	test('sin nadie pidiendo nada, la ventana no muestra ningún diálogo', () => {
		expect(dialogoVisible(false, false)).toBe('ninguno');
	});

	test('cada pedido muestra el suyo', () => {
		expect(dialogoVisible(true, false)).toBe('polkit');
		expect(dialogoVisible(false, true)).toBe('desbloqueo');
	});

	test('con los dos vivos gana polkit', () => {
		// Abrir un disco interno pide las dos cosas: primero la frase del disco y,
		// al llamar a `Unlock`, la autorización de polkit — que llega mientras el
		// desbloqueo sigue esperando. Mostrar el de abajo sería pedir una frase
		// que no va a ir a ningún lado, y la respuesta se la llevaría el diálogo
		// equivocado.
		expect(dialogoVisible(true, true)).toBe('polkit');
	});
});
