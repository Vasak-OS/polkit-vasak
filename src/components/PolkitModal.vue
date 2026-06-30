<script setup lang="ts">
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { onMounted, onUnmounted, ref, nextTick } from 'vue';
import { getIconSource } from '@vasakgroup/plugin-vicons';
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

const visible = ref(false);
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
    error.value = typeof e === 'string' ? e : 'Error al enviar la contraseña';
    loading.value = false;
  });
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape') cancel();
}

async function cancel() {
  if (!cookie.value) return;
  visible.value = false;
  password.value = '';
  error.value = '';
  loading.value = false;
  await invoke('cancel_pending', { cookie: cookie.value }).catch(() => {});
}

function triggerShake() {
  shaking.value = true;
  setTimeout(() => { shaking.value = false; }, 500);
}

onMounted(async () => {
  document.addEventListener('keydown', onKeydown);

  unlistenRequest = await listen<PolkitRequest>('polkit-request', (event) => {
    message.value = event.payload.message;
    cookie.value = event.payload.cookie;
    password.value = '';
    error.value = '';
    loading.value = false;
    visible.value = true;
    nextTick(() => inputRef.value?.focus());
  });

  unlistenResult = await listen<PolkitResult>('polkit-result', (event) => {
    if (event.payload.success) {
      visible.value = false;
    } else {
      error.value = event.payload.message || 'Contraseña incorrecta. Intente de nuevo.';
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
    <div
      v-if="visible"
      :class="[
        'h-screen w-screen flex gap-4 rounded-corner-window border border-ui-border bg-ui-bg p-5',
        shaking ? 'animate-shake' : '',
      ]"
    >
      <img
        v-if="shieldIcon"
        :src="shieldIcon"
        class="self-stretch h-auto w-20 shrink-0 object-scale-down"
        alt=""
      />

      <div class="flex flex-col gap-3 min-w-0 flex-1">
        <span class="text-xs text-tx-muted tracking-wide uppercase">Autenticación requerida</span>

        <p class="text-sm text-tx-main leading-snug">{{ message }}</p>

        <form
          class="flex flex-col gap-2"
          @submit.prevent="submit"
        >
          <input
            ref="inputRef"
            v-model="password"
            type="password"
            placeholder="Contraseña"
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
              Cancelar
            </button>

            <button
              type="submit"
              :disabled="loading || !password"
              class="rounded-corner bg-primary px-4 py-1 text-sm font-medium text-tx-on-primary transition-opacity enabled:hover:opacity-90 disabled:opacity-50"
            >
              <span v-if="loading">Verificando…</span>
              <span v-else>Aceptar</span>
            </button>
          </div>
        </form>
      </div>
    </div>
  </Transition>
</template>

<style>
.dialog-enter-active {
  transition: opacity 0.25s ease-out, transform 0.25s ease-out !important;
}
.dialog-leave-active {
  transition: opacity 0.2s ease-in, transform 0.2s ease-in !important;
}
.dialog-enter-from {
  opacity: 0 !important;
  transform: scale(0.92) !important;
}
.dialog-leave-to {
  opacity: 0 !important;
  transform: scale(0.92) !important;
}

@keyframes shake {
  0%, 100% { transform: translateX(0); }
  20% { transform: translateX(-8px); }
  40% { transform: translateX(8px); }
  60% { transform: translateX(-5px); }
  80% { transform: translateX(5px); }
}
.animate-shake {
  animation: shake 0.4s ease-in-out !important;
}
</style>
