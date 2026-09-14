import { computed, ref } from 'vue';
import { type DialogoActivo, dialogoVisible } from '@/tools/dialogos';

/**
 * Quién tiene la ventana.
 *
 * El estado es de módulo y no de componente porque los dos diálogos tienen que
 * poder verse entre ellos: cada uno sabe si él está pidiendo algo, pero para
 * saber si le toca aparecer necesita saber del otro. Ver `dialogoVisible`.
 */
const polkitPidiendo = ref(false);
const desbloqueoPidiendo = ref(false);

const activo = computed<DialogoActivo>(() =>
	dialogoVisible(polkitPidiendo.value, desbloqueoPidiendo.value)
);

export function useDialogos() {
	return {
		activo,
		polkitPidiendo,
		desbloqueoPidiendo,
	};
}
