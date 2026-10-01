<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useI18n } from '@vasakgroup/tauri-plugin-i18n';
import {
	ActionButton,
	FormGroup,
	SectionHeading,
	TextInput,
	ThemeIcon,
	WindowFrame,
} from '@vasakgroup/vue-libvasak';
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { useDialogs } from '@/composables/useDialogs';

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
// `visibleDialog`.
const { active, polkitAsking } = useDialogs();

const visible = computed(() => active.value === 'polkit');
const message = ref('');
const cookie = ref('');
const password = ref('');
const error = ref('');
const loading = ref(false);
const shaking = ref(false);
const inputRef = ref<InstanceType<typeof TextInput> | null>(null);

let unlistenRequest: UnlistenFn | null = null;
let unlistenResult: UnlistenFn | null = null;

async function submit() {
	if (!password.value || !cookie.value || loading.value) return;

	loading.value = true;
	error.value = '';
	await invoke('submit_password', {
		password: password.value,
		cookie: cookie.value,
	}).catch((e: unknown) => {
		error.value = typeof e === 'string' ? e : t('polkit.sendError');
		loading.value = false;
	});
}

/**
 * Escape cancela, pero sólo si este diálogo es el que se ve.
 *
 * Los dos diálogos escuchan el mismo `document`, y abrir un disco interno los
 * deja vivos a la vez: sin mirar quién está a la vista, un Escape sobre el de
 * polkit cancelaba también el desbloqueo que esperaba debajo. El
 * `preventDefault` es para que la tecla no haga además lo suyo en el campo, y
 * es también la marca de que alguien ya la atendió: el que la recibe segundo
 * no puede fiarse de `visible`, porque el primero ya cambió quién se ve al
 * cancelar, en el mismo despacho del evento.
 */
function onKeydown(e: KeyboardEvent) {
	if (e.key !== 'Escape' || e.defaultPrevented || !visible.value) return;
	e.preventDefault();
	void cancel();
}

async function cancel() {
	if (!cookie.value) return;
	polkitAsking.value = false;
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
		polkitAsking.value = true;
		nextTick(() => inputRef.value?.focus());
	});

	unlistenResult = await listen<PolkitResult>('polkit-result', (event) => {
		if (event.payload.success) {
			polkitAsking.value = false;
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
      <!-- El contenedor es lo que deja al diálogo adaptarse al ancho que le
           den sin preguntarle a la pantalla (WebKitGTK no avisa de `resize`).
           Desde 24rem —la ventana mide 400— es lo de siempre: el icono a la
           izquierda, el mensaje recortado y los botones a la derecha. Más
           angosto va una columna por vez, como en un teléfono: el icono
           arriba, el mensaje entero, los botones uno debajo del otro, y lo que
           no entre en el alto se desplaza en vez de cortarse. -->
      <div class="@container flex min-h-0 min-w-0 flex-1">
        <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-4 overflow-y-auto p-5 @[24rem]:flex-row @[24rem]:overflow-visible">
          <ThemeIcon name="dialog-password" :size="80" class="self-start" />

          <div class="flex flex-col gap-3 min-w-0 flex-1">
            <SectionHeading :title="t('polkit.title')" as="h2" />

            <!-- El mensaje lo escribe la acción de polkit que pidió permiso, y hay
                 algunas largas: la de limpiar paquetes huérfanos lleva el comando
                 entero adentro. Antes desbordaba y aparecía una barra de
                 desplazamiento dentro de un diálogo modal, que además tapaba los
                 botones.
                 En el ancho de la ventana el texto se recorta con puntos
                 suspensivos en la cantidad de renglones que siempre entra, y el
                 texto completo queda en el `title`. Más angosto, donde va una
                 columna por vez, se muestra entero: ahí recortar dejaba el
                 comando a media línea, y lo que no entra se desplaza. -->
            <p class="text-sm text-tx-main leading-snug break-words @[24rem]:line-clamp-6" :title="message">{{ message }}</p>

            <form
              class="flex flex-col gap-2"
              @submit.prevent="submit"
            >
              <!-- `FormGroup` ata el error al campo (`aria-describedby`) y lo
                   anuncia al aparecer: sin eso, escribir mal la contraseña no le
                   dice nada a quien no mira la pantalla. Va sin etiqueta visible;
                   el nombre del campo lo da `ariaLabel`. -->
              <FormGroup label="" :error="error" v-slot="{ id, describedBy, invalid }">
                <TextInput
                  :id="id"
                  ref="inputRef"
                  v-model="password"
                  type="password"
                  :ariaLabel="t('polkit.password')"
                  :placeholder="t('polkit.password')"
                  autocomplete="current-password"
                  :invalid="invalid"
                  :describedBy="describedBy"
                />
              </FormGroup>

              <!-- Cancelar va primero y es `type="button"`: Enter en el campo
                   envía con el primer botón de envío del formulario, y si
                   Cancelar lo fuera, Enter cancelaría. -->
              <div class="flex flex-col gap-2 pt-1 @[24rem]:flex-row @[24rem]:flex-wrap @[24rem]:justify-end">
                <ActionButton
                  variant="secondary"
                  custom-class="w-full shrink-0 @[24rem]:w-auto"
                  :label="t('polkit.cancel')"
                  @click="cancel"
                />

                <ActionButton
                  type="submit"
                  custom-class="w-full shrink-0 @[24rem]:w-auto"
                  :label="loading ? t('polkit.checking') : t('polkit.accept')"
                  :loading="loading"
                  :disabled="!password"
                />
              </div>
            </form>
          </div>
        </div>
      </div>
    </WindowFrame>
  </Transition>
</template>
