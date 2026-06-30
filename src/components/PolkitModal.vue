<script setup lang="ts">
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { onMounted, onUnmounted, ref, nextTick } from 'vue';

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
const inputRef = ref<HTMLInputElement | null>(null);

let unlistenRequest: UnlistenFn | null = null;
let unlistenResult: UnlistenFn | null = null;
let unlistenCancel: UnlistenFn | null = null;

async function submit() {
  if (!password.value || !cookie.value) return;

  loading.value = true;
  error.value = '';
  try {
    await invoke('submit_password', {
      password: password.value,
      cookie: cookie.value,
    });
  } catch (e: any) {
    error.value = typeof e === 'string' ? e : 'Error al enviar la contraseña';
  } finally {
    loading.value = false;
  }
}

onMounted(async () => {
  unlistenRequest = await listen<PolkitRequest>('polkit-request', (event) => {
    message.value = event.payload.message;
    cookie.value = event.payload.cookie;
    password.value = '';
    error.value = '';
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
      nextTick(() => inputRef.value?.focus());
    }
  });

  unlistenCancel = await listen('polkit-cancel', () => {
    visible.value = false;
    password.value = '';
    loading.value = false;
  });
});

onUnmounted(() => {
  unlistenRequest?.();
  unlistenResult?.();
  unlistenCancel?.();
});
</script>

<template>
  <div
    v-if="visible"
    class="h-screen w-screen flex flex-col gap-3 rounded-corner-window border border-ui-border bg-ui-bg p-5"
  >
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

      <div class="flex justify-end pt-1">
        <button
          type="submit"
          :disabled="loading || !password"
          class="rounded-corner bg-primary px-4 py-1 text-sm font-medium text-tx-on-primary transition-opacity enabled:hover:opacity-90 disabled:opacity-50"
        >
          <span v-if="loading">Verificando…</span>
          <span v-else>Autenticar</span>
        </button>
      </div>
    </form>
  </div>
</template>
