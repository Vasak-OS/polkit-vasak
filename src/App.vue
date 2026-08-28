<script setup lang="ts">
import { useConfigStore } from '@vasakgroup/plugin-config-manager';
import type { Store } from 'pinia';
import { onMounted } from 'vue';
import PolkitModal from '@/components/PolkitModal.vue';

// Sin esto el diálogo se queda con los colores por omisión —claros— aunque el
// escritorio esté en tema oscuro: los colores llegan por la configuración, como
// en el resto de las aplicaciones, y acá nadie la estaba pidiendo. Se ve
// enseguida, porque este diálogo aparece encima de lo que sea que estés
// haciendo.
onMounted(() => {
	// El tipo del store llega genérico desde el plugin; el mismo molde que usa
	// la pantalla de bloqueo.
	const configuracion = useConfigStore() as Store<
		'config',
		{ config: unknown; loadConfig: () => Promise<void> }
	>;
	configuracion.loadConfig().catch(() => {
		// Con los colores por omisión sigue siendo un diálogo usable; lo que no
		// puede es no aparecer.
	});
});
</script>

<template>
  <PolkitModal />
</template>
