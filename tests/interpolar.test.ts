import { describe, expect, test } from 'bun:test';
import { interpolar } from '@/tools/interpolar';

describe('interpolar', () => {
	test('reemplaza el marcador por el nombre del disco', () => {
		expect(interpolar('Escribí la frase de {0}', 'Respaldos')).toBe(
			'Escribí la frase de Respaldos'
		);
	});

	test('una etiqueta con «$&» no se destroza', () => {
		// `replace` con una cadena interpreta `$&` como «lo que coincidió», o sea
		// el propio marcador: el diálogo diría «Rock {0} Roll».
		expect(interpolar('Frase de {0}', 'Rock $& Roll')).toBe('Frase de Rock $& Roll');
	});

	test("y con «$'» no se pierde el resto del texto", () => {
		expect(interpolar('Frase de {0} para abrirlo', "Disco$'")).toBe(
			"Frase de Disco$' para abrirlo"
		);
	});

	test('una etiqueta que parece un marcador no se vuelve a reemplazar', () => {
		expect(interpolar('Frase de {0}', '{1}')).toBe('Frase de {1}');
	});

	test('un marcador sin valor queda como está, en lugar de decir «undefined»', () => {
		expect(interpolar('Frase de {0}')).toBe('Frase de {0}');
	});
});
