/**
 * Cuál de los dos diálogos se ve.
 *
 * La ventana es una sola y ahora hay dos cosas que puede preguntar: la
 * contraseña de la cuenta, que pide polkit, y la frase de paso de un disco
 * cifrado. No son alternativas —abrir un disco interno necesita las dos, una
 * después de la otra— así que puede haber dos pedidos vivos al mismo tiempo.
 */
export type DialogoActivo = 'ninguno' | 'polkit' | 'desbloqueo';

/**
 * Cuando los dos están pidiendo algo, se ve el de polkit.
 *
 * No es una preferencia: el pedido de polkit llega *adentro* del desbloqueo —se
 * dispara al llamar a `Unlock` sobre un disco que requiere autorización— y lo
 * deja esperando. Mostrar el de abajo sería pedir una frase que no va a ir a
 * ningún lado hasta que se conteste la de arriba, y encima la respuesta se la
 * llevaría el diálogo equivocado.
 */
export function dialogoVisible(
	polkitPidiendo: boolean,
	desbloqueoPidiendo: boolean
): DialogoActivo {
	if (polkitPidiendo) {
		return 'polkit';
	}

	if (desbloqueoPidiendo) {
		return 'desbloqueo';
	}

	return 'ninguno';
}
