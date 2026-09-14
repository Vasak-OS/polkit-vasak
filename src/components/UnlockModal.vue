<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getIconSource } from '@vasakgroup/plugin-vicons';
import { useI18n } from '@vasakgroup/tauri-plugin-i18n';
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { useDialogos } from '@/composables/useDialogos';
import { useReactiveIcon } from '@/composables/useReactiveIcon';
import { interpolar } from '@/tools/interpolar';

interface PedidoDeDesbloqueo {
	id: string;
	/** La etiqueta del volumen, o su `/dev/sdX` si no tiene. */
	name: string;
	/** Si esto es un reintento porque la frase anterior no abrió el disco. */
	wrongPassphrase: boolean;
}

const { t } = useI18n();
const { activo, desbloqueoPidiendo } = useDialogos();

const id = ref('');
const nombre = ref('');
const frase = ref('');
const recordar = ref(false);
const error = ref('');
const enviando = ref(false);
const temblando = ref(false);
const campo = ref<HTMLInputElement | null>(null);

let dejarDeEscucharPedido: UnlistenFn | null = null;
let dejarDeEscucharFin: UnlistenFn | null = null;

// Un disco con candado, no el escudo de polkit: la contraseña de la cuenta y la
// frase de un disco son preguntas distintas y tienen que verse distintas. Ver
// `desbloqueo.rs`.
const iconoDeDisco = useReactiveIcon(() => getIconSource('drive-harddisk-encrypted'));

const visible = computed(() => activo.value === 'desbloqueo');

// El `t()` propio no interpola, así que el nombre del disco entra a mano. Y no
// con `replace`: la etiqueta de un disco puede tener un `$&`. Ver `interpolar`.
const mensaje = computed(() => interpolar(t('unlock.prompt'), nombre.value));

async function enviar() {
	if (!frase.value || !id.value || enviando.value) {
		return;
	}

	enviando.value = true;
	error.value = '';

	try {
		await invoke('enviar_frase', {
			id: id.value,
			frase: frase.value,
			recordar: recordar.value,
		});
	} catch (fallo: unknown) {
		error.value = typeof fallo === 'string' ? fallo : t('unlock.sendError');
		enviando.value = false;
	}
}

async function cancelar() {
	if (!id.value) {
		return;
	}

	const pendiente = id.value;
	cerrar();

	await invoke('cancelar_desbloqueo', { id: pendiente }).catch(() => {
		// El desbloqueo ya había terminado por su cuenta. La ventana ya se fue.
	});
}

function cerrar() {
	desbloqueoPidiendo.value = false;
	id.value = '';
	frase.value = '';
	error.value = '';
	enviando.value = false;
}

function alTeclear(evento: KeyboardEvent) {
	if (evento.key === 'Escape') {
		void cancelar();
	}
}

function temblar() {
	temblando.value = true;
	setTimeout(() => {
		temblando.value = false;
	}, 500);
}

onMounted(async () => {
	document.addEventListener('keydown', alTeclear);

	dejarDeEscucharPedido = await listen<PedidoDeDesbloqueo>('unlock-request', (evento) => {
		id.value = evento.payload.id;
		nombre.value = evento.payload.name;
		frase.value = '';
		enviando.value = false;
		desbloqueoPidiendo.value = true;

		// Un reintento llega como un pedido nuevo: el diálogo ya estaba abierto y
		// lo único que cambia es que ahora dice por qué se lo está preguntando de
		// vuelta.
		if (evento.payload.wrongPassphrase) {
			error.value = t('unlock.wrongPassphrase');
			temblar();
		} else {
			error.value = '';
		}

		nextTick(() => campo.value?.focus());
	});

	// El desbloqueo terminó —bien o mal— y la ventana se va. Quien avisa de qué
	// pasó es el gestor de archivos, que es donde la persona está mirando.
	dejarDeEscucharFin = await listen('unlock-done', () => {
		cerrar();
	});
});

onUnmounted(() => {
	document.removeEventListener('keydown', alTeclear);
	dejarDeEscucharPedido?.();
	dejarDeEscucharFin?.();
});
</script>

<template>
  <Transition name="dialog">
    <div
      v-if="visible"
      :class="[
        'h-screen w-screen flex gap-4 overflow-hidden rounded-corner-window border border-ui-border bg-ui-bg/80 p-5',
        temblando ? 'animate-shake' : '',
      ]"
    >
      <img
        v-if="iconoDeDisco"
        :src="iconoDeDisco"
        class="self-stretch h-auto w-20 shrink-0 object-scale-down"
        alt=""
      />

      <div class="flex flex-col gap-3 min-w-0 flex-1">
        <span class="text-xs text-tx-muted tracking-wide uppercase">{{ t('unlock.title') }}</span>

        <p class="text-sm text-tx-main leading-snug line-clamp-3" :title="mensaje">{{ mensaje }}</p>

        <form class="flex flex-col gap-2" @submit.prevent="enviar">
          <input
            ref="campo"
            v-model="frase"
            type="password"
            :placeholder="t('unlock.passphrase')"
            autocomplete="off"
            class="w-full rounded-corner border border-ui-border bg-ui-surface/50 px-3 py-1.5 text-sm text-tx-main placeholder:text-tx-muted/60 outline-none focus:border-primary transition-colors"
          />

          <label class="flex items-center gap-2 text-xs text-tx-muted cursor-pointer">
            <input v-model="recordar" type="checkbox" class="accent-primary" />
            {{ t('unlock.remember') }}
          </label>

          <p v-if="error" class="text-xs text-status-error">{{ error }}</p>

          <div class="flex justify-end gap-2 pt-1">
            <button
              type="button"
              class="rounded-corner border border-ui-border px-4 py-1 text-sm text-tx-main transition-colors hover:bg-ui-surface/50"
              @click="cancelar"
            >
              {{ t('unlock.cancel') }}
            </button>

            <button
              type="submit"
              :disabled="enviando || !frase"
              class="rounded-corner bg-primary px-4 py-1 text-sm font-medium text-tx-on-primary transition-opacity enabled:hover:opacity-90 disabled:opacity-50"
            >
              <span v-if="enviando">{{ t('unlock.unlocking') }}</span>
              <span v-else>{{ t('unlock.accept') }}</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  </Transition>
</template>
