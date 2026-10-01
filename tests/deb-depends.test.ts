/**
 * Lo que el `.deb` declara tiene que ser lo que el binario enlaza.
 *
 * La lista venía de la plantilla `vapp`, con `libsoup2.4-1` **y**
 * `libsoup-3.0-0` —las dos generaciones a la vez— y `libpango-1.0-0`, que el
 * binario no enlaza; y sin `libdbus-1-3` ni `libjavascriptcoregtk-4.1-0`, que
 * sí. Medido con `readelf -d … | grep NEEDED` sobre el binario de release,
 * nunca con `ldd`, que suma las transitivas.
 *
 * Además van los dos servicios con los que el agente habla por el bus sin
 * enlazarlos: `polkitd`, que es quien le pide la contraseña, y `udisks2`, con
 * el que abre los discos cifrados. Son los mismos dos que la receta de Arch
 * declara como `polkit` y `udisks2`.
 */

import { describe, expect, test } from 'bun:test';

const conf = await Bun.file(new URL('../src-tauri/tauri.conf.json', import.meta.url)).json();
const depends: string[] = conf.bundle.linux.deb.depends;

describe('las dependencias del .deb', () => {
	test('traen lo que enlaza el binario', () => {
		for (const pkg of [
			'libc6',
			'libgcc-s1',
			'libcairo2',
			'libdbus-1-3',
			'libgdk-pixbuf-2.0-0',
			'libglib2.0-0t64',
			'libgtk-3-0t64',
			'libjavascriptcoregtk-4.1-0',
			'libsoup-3.0-0',
			'libwebkit2gtk-4.1-0',
		]) {
			expect(depends, `falta ${pkg}`).toContain(pkg);
		}
	});

	test('y los servicios con los que habla por el bus', () => {
		expect(depends).toContain('polkitd');
		expect(depends).toContain('udisks2');
	});

	test('y no lo que no enlaza', () => {
		// libsoup 2 es la de WebKitGTK 4.0; Tauri 2 va con la 4.1, que usa la 3.
		expect(depends).not.toContain('libsoup2.4-1');
		expect(depends).not.toContain('libpango-1.0-0');
	});

	test('sin repetidos', () => {
		expect(new Set(depends).size).toBe(depends.length);
	});
});
