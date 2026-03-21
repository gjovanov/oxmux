<template>
  <div class="dialog-overlay">
    <div class="dialog">
      <div class="dialog-header">
        <h3>New Session</h3>
        <button class="close-btn" @click="close">&times;</button>
      </div>

      <div class="dialog-body">
        <label class="field">
          <span class="field-label">Session Name</span>
          <input v-model="name" type="text" placeholder="my-project" autofocus />
        </label>

        <!-- Backend: how we reach tmux -->
        <label class="field">
          <span class="field-label">Backend</span>
          <select v-model="backendType">
            <option value="ssh">SSH (remote host)</option>
            <option value="agent">Agent (oxmux-agent on host)</option>
            <option value="local">Local (server tmux)</option>
          </select>
        </label>

        <!-- SSH backend fields -->
        <template v-if="backendType === 'ssh' || backendType === 'agent'">
          <label class="field">
            <span class="field-label">Host</span>
            <input v-model="host" type="text" placeholder="192.0.2.1" />
          </label>
          <label class="field" v-if="backendType === 'agent'">
            <span class="field-label">Agent Port</span>
            <input v-model.number="agentPort" type="number" placeholder="4433" />
          </label>
        </template>

        <template v-if="backendType === 'ssh'">
          <label class="field">
            <span class="field-label">SSH Port</span>
            <input v-model.number="sshPort" type="number" placeholder="22" />
          </label>
          <label class="field">
            <span class="field-label">User</span>
            <input v-model="user" type="text" placeholder="ubuntu" />
          </label>
          <label class="field">
            <span class="field-label">Auth Method</span>
            <select v-model="authMethod">
              <option value="agent">SSH Agent</option>
              <option value="private_key">Private Key (server path)</option>
              <option value="uploaded_key">Upload Key</option>
              <option value="password">Password</option>
            </select>
          </label>
          <label class="field" v-if="authMethod === 'private_key'">
            <span class="field-label">Key Path</span>
            <input v-model="keyPath" type="text" placeholder="~/.ssh/id_ed25519" />
          </label>
          <label class="field" v-if="authMethod === 'private_key'">
            <span class="field-label">Passphrase <span style="opacity:0.5">(leave empty if unencrypted)</span></span>
            <input v-model="passphrase" type="password" />
          </label>
          <template v-if="authMethod === 'uploaded_key'">
            <label class="field">
              <span class="field-label">Private Key File</span>
              <input type="file" @change="onKeyFileSelected" class="file-input" />
              <span v-if="uploadedKeyName" class="key-status">{{ uploadedKeyName }}</span>
            </label>
            <label class="field">
              <span class="field-label">Passphrase <span style="opacity:0.5">(leave empty if unencrypted)</span></span>
              <input v-model="uploadedKeyPassphrase" type="password" />
            </label>
          </template>
          <label class="field" v-if="authMethod === 'password'">
            <span class="field-label">Password</span>
            <input v-model="password" type="password" />
          </label>
        </template>

        <!-- Browser transport: how browser connects -->
        <label class="field">
          <span class="field-label">Browser Transport</span>
          <div class="transport-grid">
            <template v-if="backendType === 'ssh' || backendType === 'local'">
              <label class="transport-option" :class="{ active: browserTransport === 'websocket' }">
                <input type="radio" v-model="browserTransport" value="websocket" />
                <span class="transport-badge ws">WS</span> WebSocket
                <span class="transport-tag">default</span>
              </label>
              <label class="transport-option" :class="{ active: browserTransport === 'quic' }">
                <input type="radio" v-model="browserTransport" value="quic" />
                <span class="transport-badge quic-badge">QUIC</span> WebTransport
              </label>
              <label class="transport-option" :class="{ active: browserTransport === 'webrtc' }">
                <input type="radio" v-model="browserTransport" value="webrtc" />
                <span class="transport-badge webrtc-badge">RTC</span> WebRTC
              </label>
            </template>
            <template v-if="backendType === 'agent'">
              <label class="transport-option" :class="{ active: browserTransport === 'quic' }">
                <input type="radio" v-model="browserTransport" value="quic" />
                <span class="transport-badge quic-badge">QUIC</span> P2P
                <span class="transport-tag">fastest</span>
              </label>
              <label class="transport-option" :class="{ active: browserTransport === 'webrtc' }">
                <input type="radio" v-model="browserTransport" value="webrtc" />
                <span class="transport-badge webrtc-badge">RTC</span> P2P
              </label>
            </template>
          </div>
        </label>

        <div class="transport-info">
          Transport #{{ transportNumber }}: {{ transportDescription }}
        </div>
      </div>

      <div class="dialog-footer">
        <button class="btn btn-secondary" @click="close">Cancel</button>
        <button class="btn btn-primary" @click="create" :disabled="!isValid || isUploading">
          {{ isUploading ? 'Uploading key...' : 'Create' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { useTmuxStore, type TransportConfig, type BrowserTransportType } from '@/stores/tmux'
import { useAuthStore } from '@/stores/auth'

const store = useTmuxStore()
const authStore = useAuthStore()

const name = ref('')
const backendType = ref<'ssh' | 'agent' | 'local'>('ssh')
const browserTransport = ref<BrowserTransportType>('websocket')
const host = ref('')
const sshPort = ref<number>(22)
const agentPort = ref<number>(4433)
const user = ref('')
const authMethod = ref<'agent' | 'private_key' | 'password' | 'uploaded_key'>('private_key')
const keyPath = ref('')
const passphrase = ref('')
const password = ref('')
const uploadedKeyContent = ref('')
const uploadedKeyName = ref('')
const uploadedKeyId = ref('')
const uploadedKeyPassphrase = ref('')
const isUploading = ref(false)

// When backend changes, adjust default browser transport
watch(backendType, (bt) => {
  if (bt === 'agent') {
    browserTransport.value = 'quic'
  } else {
    browserTransport.value = 'websocket'
  }
})

const transportNumber = computed(() => {
  const bt = browserTransport.value
  const be = backendType.value
  if (be === 'ssh' && bt === 'websocket') return 1
  if (be === 'ssh' && bt === 'quic') return 2
  if (be === 'ssh' && bt === 'webrtc') return 3
  if (be === 'agent' && bt === 'quic') return 4
  if (be === 'agent' && bt === 'webrtc') return 5
  return 0
})

const transportDescription = computed(() => {
  const descriptions: Record<number, string> = {
    1: 'Browser -WS-> Server -SSH-> Host',
    2: 'Browser -QUIC-> Server -SSH-> Host',
    3: 'Browser -WebRTC-> Server -SSH-> Host',
    4: 'Browser -QUIC-> Agent (P2P)',
    5: 'Browser -WebRTC-> Agent (P2P)',
    0: 'Local server tmux',
  }
  return descriptions[transportNumber.value] || ''
})

const isValid = computed(() => {
  if (!name.value.trim()) return false
  if (backendType.value === 'ssh') {
    if (!host.value.trim() || !user.value.trim()) return false
    if (authMethod.value === 'uploaded_key' && !uploadedKeyContent.value) return false
  }
  if (backendType.value === 'agent') {
    if (!host.value.trim()) return false
  }
  return true
})

function buildTransport(): TransportConfig {
  const browser = browserTransport.value

  if (backendType.value === 'ssh') {
    let auth: any = { method: 'agent' }
    if (authMethod.value === 'private_key') {
      auth = { method: 'private_key', path: keyPath.value, passphrase: passphrase.value || undefined }
    } else if (authMethod.value === 'uploaded_key') {
      auth = { method: 'uploaded_key', key_id: uploadedKeyId.value }
    } else if (authMethod.value === 'password') {
      auth = { method: 'password', password: password.value }
    }
    return {
      browser,
      backend: { type: 'ssh', host: host.value, port: sshPort.value || 22, user: user.value, auth },
    }
  }

  if (backendType.value === 'agent') {
    return {
      browser,
      backend: { type: 'agent', host: host.value, port: agentPort.value || 4433 },
    }
  }

  return { browser, backend: { type: 'local' } }
}

function onKeyFileSelected(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]
  if (!file) return
  if (file.size > 8192) {
    alert('Key file too large (max 8KB)')
    return
  }
  uploadedKeyName.value = file.name
  const reader = new FileReader()
  reader.onload = () => { uploadedKeyContent.value = reader.result as string }
  reader.readAsText(file)
}

async function create() {
  // If using uploaded key, upload it first and get key_id
  if (authMethod.value === 'uploaded_key' && uploadedKeyContent.value) {
    isUploading.value = true
    try {
      const resp = await fetch('/api/ssh-keys', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${authStore.token}`,
        },
        body: JSON.stringify({
          key_pem: uploadedKeyContent.value,
          passphrase: uploadedKeyPassphrase.value || undefined,
        }),
      })
      if (!resp.ok) {
        const err = await resp.text()
        alert(`Key upload failed: ${err}`)
        return
      }
      const { key_id } = await resp.json()
      uploadedKeyId.value = key_id
      uploadedKeyContent.value = ''  // Clear sensitive data from memory
      uploadedKeyPassphrase.value = ''
    } catch (e) {
      alert(`Key upload failed: ${e}`)
      return
    } finally {
      isUploading.value = false
    }
  }

  store.createSession(name.value.trim(), buildTransport())
  close()
}

function close() {
  store.showNewSessionDialog = false
}
</script>

<style scoped>
.dialog-overlay {
  position: fixed; inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex; align-items: center; justify-content: center;
  z-index: 100;
}
.dialog {
  background: #1e1e2e; border: 1px solid #313244;
  border-radius: 8px; width: 460px; max-width: 90vw;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
}
.dialog-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 16px 20px; border-bottom: 1px solid #313244;
}
.dialog-header h3 { margin: 0; color: #89b4fa; font-size: 15px; }
.close-btn {
  background: none; border: none; color: #585b70; font-size: 20px;
  cursor: pointer; padding: 0 4px;
}
.close-btn:hover { color: #cdd6f4; }
.dialog-body { padding: 16px 20px; }
.field { display: block; margin-bottom: 12px; }
.field-label { display: block; font-size: 11px; color: #a6adc8; margin-bottom: 4px; text-transform: uppercase; letter-spacing: 0.5px; }
.field input, .field select {
  width: 100%; padding: 8px 10px; background: #11111b; border: 1px solid #313244;
  border-radius: 4px; color: #cdd6f4; font-size: 13px; outline: none;
}
.field input:focus, .field select:focus { border-color: #89b4fa; }
.transport-grid { display: flex; flex-direction: column; gap: 4px; }
.transport-option {
  display: flex; align-items: center; gap: 8px;
  padding: 6px 10px; background: #11111b; border: 1px solid #313244;
  border-radius: 4px; cursor: pointer; font-size: 12px; color: #a6adc8;
}
.transport-option:hover { border-color: #45475a; }
.transport-option.active { border-color: #89b4fa; color: #cdd6f4; }
.transport-option input[type="radio"] { display: none; }
.transport-badge {
  font-size: 9px; font-weight: 700; padding: 2px 5px; border-radius: 3px;
  text-transform: uppercase;
}
.transport-badge.ws { background: #1e4620; color: #a6e3a1; }
.transport-badge.quic-badge { background: #4c1d95; color: #cba6f7; }
.transport-badge.webrtc-badge { background: #5f3a1e; color: #fab387; }
.transport-tag { margin-left: auto; font-size: 10px; color: #585b70; }
.transport-info {
  font-size: 11px; color: #585b70; padding: 8px 0 4px;
  border-top: 1px solid #313244; margin-top: 4px;
}
.dialog-footer {
  display: flex; gap: 8px; justify-content: flex-end;
  padding: 12px 20px; border-top: 1px solid #313244;
}
.btn {
  padding: 6px 14px; border: none; border-radius: 4px;
  font-size: 12px; cursor: pointer; font-weight: 600;
}
.btn:disabled { opacity: 0.4; cursor: not-allowed; }
.btn-secondary { background: #313244; color: #cdd6f4; }
.btn-secondary:hover:not(:disabled) { background: #45475a; }
.btn-primary { background: #89b4fa; color: #1e1e2e; }
.btn-primary:hover:not(:disabled) { background: #74c7ec; }
.file-input {
  padding: 6px !important; font-size: 12px !important;
}
.key-status {
  display: block; margin-top: 4px; font-size: 11px; color: #a6e3a1;
}
</style>
