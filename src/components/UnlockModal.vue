<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useI18n } from '@vasakgroup/tauri-plugin-i18n';
import {
	ActionButton,
	Checkbox,
	FormGroup,
	SectionHeading,
	TextInput,
	ThemeIcon,
	WindowFrame,
} from '@vasakgroup/vue-libvasak';
import { computed, nextTick, onMounted, onUnmounted, ref } from 'vue';
import { useDialogs } from '@/composables/useDialogs';
import { interpolate } from '@/tools/interpolate';

interface UnlockRequest {
	id: string;
	/** La etiqueta del volumen, o su `/dev/sdX` si no tiene. */
	name: string;
	/** Si esto es un reintento porque la frase anterior no abrió el disco. */
	wrongPassphrase: boolean;
}

const { t } = useI18n();
const { active, unlockAsking } = useDialogs();

const id = ref('');
const name = ref('');
const passphrase = ref('');
const remember = ref(false);
const error = ref('');
const sending = ref(false);
const shaking = ref(false);
const field = ref<InstanceType<typeof TextInput> | null>(null);

let unlistenRequest: UnlistenFn | null = null;
let unlistenDone: UnlistenFn | null = null;

const visible = computed(() => active.value === 'unlock');

// El `t()` propio no interpola, así que el nombre del disco entra a mano. Y no
// con `replace`: la etiqueta de un disco puede tener un `$&`. Ver `interpolate`.
const message = computed(() => interpolate(t('unlock.prompt'), name.value));

async function submit() {
	if (!passphrase.value || !id.value || sending.value) {
		return;
	}

	sending.value = true;
	error.value = '';

	try {
		// Los nombres del comando y de sus argumentos son los del backend en
		// Rust, que este cambio no toca.
		await invoke('enviar_frase', {
			id: id.value,
			frase: passphrase.value,
			recordar: remember.value,
		});
	} catch (failure: unknown) {
		error.value = typeof failure === 'string' ? failure : t('unlock.sendError');
		sending.value = false;
	}
}

async function cancel() {
	if (!id.value) {
		return;
	}

	const pending = id.value;
	close();

	await invoke('cancelar_desbloqueo', { id: pending }).catch(() => {
		// El desbloqueo ya había terminado por su cuenta. La ventana ya se fue.
	});
}

function close() {
	unlockAsking.value = false;
	id.value = '';
	passphrase.value = '';
	error.value = '';
	sending.value = false;
}

/**
 * Escape cancela, pero sólo si este diálogo es el que se ve.
 *
 * Mientras polkit pregunta encima, el desbloqueo sigue vivo y escuchando el
 * mismo `document`: sin mirar quién está a la vista, el Escape que cancelaba la
 * autorización cancelaba también este pedido, que nadie estaba mirando.
 */
function onKeydown(event: KeyboardEvent) {
	if (event.key !== 'Escape' || !visible.value) return;
	event.preventDefault();
	void cancel();
}

function shake() {
	shaking.value = true;
	setTimeout(() => {
		shaking.value = false;
	}, 500);
}

onMounted(async () => {
	document.addEventListener('keydown', onKeydown);

	unlistenRequest = await listen<UnlockRequest>('unlock-request', (event) => {
		id.value = event.payload.id;
		name.value = event.payload.name;
		passphrase.value = '';
		sending.value = false;
		unlockAsking.value = true;

		// Un reintento llega como un pedido nuevo: el diálogo ya estaba abierto y
		// lo único que cambia es que ahora dice por qué se lo está preguntando de
		// vuelta.
		if (event.payload.wrongPassphrase) {
			error.value = t('unlock.wrongPassphrase');
			shake();
		} else {
			error.value = '';
		}

		nextTick(() => field.value?.focus());
	});

	// El desbloqueo terminó —bien o mal— y la ventana se va. Quien avisa de qué
	// pasó es el gestor de archivos, que es donde la persona está mirando.
	unlistenDone = await listen('unlock-done', () => {
		close();
	});
});

onUnmounted(() => {
	document.removeEventListener('keydown', onKeydown);
	unlistenRequest?.();
	unlistenDone?.();
});
</script>

<template>
  <Transition name="dialog">
    <!-- El marco compartido, como el de polkit: era una superficie dibujada a
         mano con su propio borde y su propia esquina, y los dos diálogos
         aparecen en la misma ventana, uno después del otro. -->
    <WindowFrame
      v-if="visible"
      hide-bar
      :class="shaking ? 'animate-shake' : ''"
    >
      <!-- El contenedor es lo que deja al diálogo adaptarse al ancho que le
           den sin preguntarle a la pantalla (WebKitGTK no avisa de `resize`):
           por debajo de 20rem el icono de 80 se esconde y el texto se queda
           con ese ancho, en vez de partir las palabras letra por letra. -->
      <div class="@container flex min-w-0 flex-1">
        <div class="flex min-w-0 flex-1 gap-4 p-5">
          <!-- Un disco con candado, no el escudo de polkit: la contraseña de la
               cuenta y la frase de un disco son preguntas distintas y tienen que
               verse distintas. -->
          <ThemeIcon name="drive-harddisk-encrypted" :size="80" class="hidden self-start @[20rem]:block" />

          <div class="flex flex-col gap-3 min-w-0 flex-1">
            <SectionHeading :title="t('unlock.title')" as="h2" />

            <p class="text-sm text-tx-main leading-snug line-clamp-3" :title="message">{{ message }}</p>

            <form class="flex flex-col gap-2" @submit.prevent="submit">
              <!-- La casilla va adentro del grupo, entre el campo y el error: es
                   el orden de siempre, y el error sigue atado al campo. -->
              <FormGroup label="" :error="error" v-slot="{ id: fieldId, describedBy, invalid }">
                <TextInput
                  :id="fieldId"
                  ref="field"
                  v-model="passphrase"
                  type="password"
                  :ariaLabel="t('unlock.passphrase')"
                  :placeholder="t('unlock.passphrase')"
                  autocomplete="off"
                  :invalid="invalid"
                  :describedBy="describedBy"
                />

                <Checkbox v-model="remember" :label="t('unlock.remember')" />
              </FormGroup>

              <!-- Cancelar es `type="button"`: Enter envía con el primer botón de
                   envío, y si Cancelar lo fuera, Enter cancelaría. -->
              <div class="flex flex-wrap justify-end gap-2 pt-1">
                <ActionButton
                  variant="secondary"
                  custom-class="shrink-0"
                  :label="t('unlock.cancel')"
                  @click="cancel"
                />

                <ActionButton
                  type="submit"
                  custom-class="shrink-0"
                  :label="sending ? t('unlock.unlocking') : t('unlock.accept')"
                  :loading="sending"
                  :disabled="!passphrase"
                />
              </div>
            </form>
          </div>
        </div>
      </div>
    </WindowFrame>
  </Transition>
</template>
