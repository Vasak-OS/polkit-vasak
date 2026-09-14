/**
 * Interpolación de textos traducidos.
 *
 * El `t()` del plugin toma una sola clave y **no interpola**, así que el valor se
 * mete después. La convención del escritorio es el marcador `{0}`, `{1}`…
 *
 * # Por qué no `replace` a secas
 *
 * `String.prototype.replace` interpreta `$&`, `$$`, `` $` `` y `$'` **en la cadena
 * de reemplazo**. Acá el valor que se interpola es la etiqueta de un disco, que
 * la eligió quien lo formateó: un volumen llamado «Rock $& Roll» aparecería en el
 * diálogo como «Rock {0} Roll», y uno con `$'` **perdería el texto que viene
 * después**. En un diálogo que pide una contraseña, no decir bien de qué disco se
 * trata es exactamente lo que no puede pasar.
 */
export function interpolar(plantilla: string, ...valores: unknown[]): string {
	// Una sola pasada, y con función de reemplazo: así un valor que contenga el
	// texto de otro marcador no lo reemplaza la pasada siguiente, y los `$` del
	// valor no se interpretan.
	return plantilla.replace(/\{(\d+)\}/g, (completo, indice: string) => {
		const valor = valores[Number(indice)];
		// Un marcador sin valor se deja como está, en lugar de decir «undefined».
		return valor === undefined ? completo : String(valor);
	});
}
