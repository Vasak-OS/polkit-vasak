import { computed, ref } from 'vue';
import { type ActiveDialog, visibleDialog } from '@/tools/dialogs';

/**
 * Quién tiene la ventana.
 *
 * El estado es de módulo y no de componente porque los dos diálogos tienen que
 * poder verse entre ellos: cada uno sabe si él está pidiendo algo, pero para
 * saber si le toca aparecer necesita saber del otro. Ver `visibleDialog`.
 */
const polkitAsking = ref(false);
const unlockAsking = ref(false);

const active = computed<ActiveDialog>(() => visibleDialog(polkitAsking.value, unlockAsking.value));

export function useDialogs() {
	return {
		active,
		polkitAsking,
		unlockAsking,
	};
}
