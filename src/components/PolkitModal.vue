<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getIconSource } from '@vasakgroup/plugin-vicons';
import { useI18n } from '@vasakgroup/tauri-plugin-i18n';
import { WindowFrame } from '@vasakgroup/vue-libvasak';
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { useDialogos } from '@/composables/useDialogos';
import { useReactiveIcon } from '@/composables/useReactiveIcon';

interface PolkitRequest {
	message: string;
	cookie: string;
}

interface PolkitResult {
	success: boolean;
	cookie: string;
	message?: string;
}

const { t } = useI18n();
// La ventana es una sola y ahora hay otro diálogo compartiéndola. Ver
// `dialogoVisible`.
const { activo, polkitPidiendo } = useDialogos();

const visible = computed(() => activo.value === 'polkit');
const message = ref('');
const cookie = ref('');
const password = ref('');
const error = ref('');
const loading = ref(false);
const shaking = ref(false);
const inputRef = ref<HTMLInputElement | null>(null);

let unlistenRequest: UnlistenFn | null = null;
let unlistenResult: UnlistenFn | null = null;

const shieldIcon = useReactiveIcon(() => getIconSource('dialog-password'));
async function submit() {
	if (!password.value || !cookie.value || loading.value) return;

	loading.value = true;
	error.value = '';
	await invoke('submit_password', {
		password: password.value,
		cookie: cookie.value,
	}).catch((e: any) => {
		error.value = typeof e === 'string' ? e : t('polkit.sendError');
		loading.value = false;
	});
}

function onKeydown(e: KeyboardEvent) {
	if (e.key === 'Escape') cancel();
}

async function cancel() {
	if (!cookie.value) return;
	polkitPidiendo.value = false;
	password.value = '';
	error.value = '';
	loading.value = false;
	await invoke('cancel_pending', { cookie: cookie.value }).catch(() => {});
}

function triggerShake() {
	shaking.value = true;
	setTimeout(() => {
		shaking.value = false;
	}, 500);
}

onMounted(async () => {
	document.addEventListener('keydown', onKeydown);

	unlistenRequest = await listen<PolkitRequest>('polkit-request', (event) => {
		message.value = event.payload.message;
		cookie.value = event.payload.cookie;
		password.value = '';
		error.value = '';
		loading.value = false;
		polkitPidiendo.value = true;
		nextTick(() => inputRef.value?.focus());
	});

	unlistenResult = await listen<PolkitResult>('polkit-result', (event) => {
		if (event.payload.success) {
			polkitPidiendo.value = false;
		} else {
			error.value = event.payload.message || t('polkit.wrongPassword');
			password.value = '';
			loading.value = false;
			triggerShake();
			nextTick(() => inputRef.value?.focus());
		}
	});
});

onUnmounted(() => {
	document.removeEventListener('keydown', onKeydown);
	unlistenRequest?.();
	unlistenResult?.();
});
</script>

<template>
  <Transition name="dialog">
    <!-- El marco es el compartido: este diálogo aparece encima de cualquier
         cosa, así que es donde más se nota si el borde o la esquina no son los
         mismos que los del resto del escritorio. `hide-bar` porque no lleva
         barra: no se minimiza ni se cierra desde un botón, se responde. -->
    <WindowFrame
      v-if="visible"
      hide-bar
      :class="shaking ? 'animate-shake' : ''"
    >
      <div class="flex min-w-0 flex-1 gap-4 p-5">
        <img
          v-if="shieldIcon"
          :src="shieldIcon"
          class="self-stretch h-auto w-20 shrink-0 object-scale-down"
          alt=""
        />

        <div class="flex flex-col gap-3 min-w-0 flex-1">
          <span class="text-xs text-tx-muted tracking-wide uppercase">{{ t('polkit.title') }}</span>

          <!-- El mensaje lo escribe la acción de polkit que pidió permiso, y hay
               algunas largas: la de limpiar paquetes huérfanos lleva el comando
               entero adentro. Antes desbordaba y aparecía una barra de
               desplazamiento dentro de un diálogo modal, que además tapaba los
               botones.
               Ahora la ventana es más alta y el texto se recorta con puntos
               suspensivos en la cantidad de renglones que siempre entra: recortar
               es preferible a una barra, y el texto completo queda en el `title`
               para quien lo necesite. -->
          <p class="text-sm text-tx-main leading-snug line-clamp-6" :title="message">{{ message }}</p>

          <form
            class="flex flex-col gap-2"
            @submit.prevent="submit"
          >
            <input
              ref="inputRef"
              v-model="password"
              type="password"
              :placeholder="t('polkit.password')"
              autocomplete="current-password"
              class="w-full rounded-corner border border-ui-border bg-ui-surface/50 px-3 py-1.5 text-sm text-tx-main placeholder:text-tx-muted/60 outline-none focus:border-primary transition-colors"
            />

            <p
              v-if="error"
              class="text-xs text-status-error"
            >
              {{ error }}
            </p>

            <div class="flex justify-end gap-2 pt-1">
              <button
                type="button"
                class="rounded-corner border border-ui-border px-4 py-1 text-sm text-tx-main transition-colors hover:bg-ui-surface/50"
                @click="cancel"
              >
                {{ t('polkit.cancel') }}
              </button>

              <button
                type="submit"
                :disabled="loading || !password"
                class="rounded-corner bg-primary px-4 py-1 text-sm font-medium text-tx-on-primary transition-opacity enabled:hover:opacity-90 disabled:opacity-50"
              >
                <span v-if="loading">{{ t('polkit.checking') }}</span>
                <span v-else>{{ t('polkit.accept') }}</span>
              </button>
            </div>
          </form>
        </div>
      </div>
    </WindowFrame>
  </Transition>
</template>
