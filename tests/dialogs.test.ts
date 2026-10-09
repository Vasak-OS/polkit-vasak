import { describe, expect, test } from 'bun:test';
import { visibleDialog } from '@/tools/dialogs';

describe('visibleDialog', () => {
	test('sin nadie pidiendo nada, la ventana no muestra ningún diálogo', () => {
		expect(visibleDialog(false, false)).toBe('none');
	});

	test('cada pedido muestra el suyo', () => {
		expect(visibleDialog(true, false)).toBe('polkit');
		expect(visibleDialog(false, true)).toBe('unlock');
	});

	test('con los dos vivos gana polkit', () => {
		// Abrir un disco interno pide las dos cosas: primero la frase del disco y,
		// al llamar a `Unlock`, la autorización de polkit — que llega mientras el
		// desbloqueo sigue esperando. Mostrar el de abajo sería pedir una frase
		// que no va a ir a ningún lado, y la respuesta se la llevaría el diálogo
		// equivocado.
		expect(visibleDialog(true, true)).toBe('polkit');
	});
});
