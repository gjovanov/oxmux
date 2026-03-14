<template>
  <div class="login-page">
    <div class="login-card">
      <div class="login-header">
        <h1>Oxmux</h1>
        <p>Claude Code Fleet Manager</p>
      </div>

      <div class="login-form">
        <div class="tabs">
          <button :class="{ active: mode === 'login' }" @click="mode = 'login'">Login</button>
          <button :class="{ active: mode === 'register' }" @click="mode = 'register'">Register</button>
        </div>

        <form @submit.prevent="submit">
          <label class="field">
            <span class="field-label">Username</span>
            <input v-model="username" type="text" autofocus required />
          </label>

          <label class="field">
            <span class="field-label">Password</span>
            <input v-model="password" type="password" required minlength="4" />
          </label>

          <div v-if="auth.error" class="error-msg">{{ auth.error }}</div>

          <button type="submit" class="submit-btn" :disabled="auth.loading">
            {{ auth.loading ? 'Please wait...' : (mode === 'login' ? 'Login' : 'Register') }}
          </button>
        </form>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useAuthStore } from '@/stores/auth'

const auth = useAuthStore()
const mode = ref<'login' | 'register'>('login')
const username = ref('')
const password = ref('')

async function submit() {
  const success = mode.value === 'login'
    ? await auth.login(username.value, password.value)
    : await auth.register(username.value, password.value)
  // App.vue watches isAuthenticated and switches view
}
</script>

<style scoped>
.login-page {
  height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #1e1e2e;
}
.login-card {
  width: 360px;
  background: #181825;
  border: 1px solid #313244;
  border-radius: 12px;
  overflow: hidden;
}
.login-header {
  text-align: center;
  padding: 32px 24px 16px;
}
.login-header h1 { color: #89b4fa; font-size: 28px; margin-bottom: 4px; }
.login-header p { color: #585b70; font-size: 13px; }
.login-form { padding: 16px 24px 24px; }
.tabs {
  display: flex;
  gap: 0;
  margin-bottom: 20px;
  border: 1px solid #313244;
  border-radius: 6px;
  overflow: hidden;
}
.tabs button {
  flex: 1;
  padding: 8px;
  background: transparent;
  border: none;
  color: #585b70;
  font-size: 13px;
  cursor: pointer;
  font-weight: 600;
}
.tabs button.active { background: #313244; color: #cdd6f4; }
.field { display: block; margin-bottom: 14px; }
.field-label {
  display: block; font-size: 11px; color: #a6adc8;
  margin-bottom: 4px; text-transform: uppercase; letter-spacing: 0.5px;
}
.field input {
  width: 100%; padding: 10px 12px;
  background: #11111b; border: 1px solid #313244;
  border-radius: 6px; color: #cdd6f4; font-size: 14px; outline: none;
}
.field input:focus { border-color: #89b4fa; }
.error-msg {
  padding: 8px 12px; margin-bottom: 12px;
  background: #2d1520; color: #f38ba8;
  border-radius: 6px; font-size: 12px;
}
.submit-btn {
  width: 100%; padding: 10px;
  background: #89b4fa; color: #1e1e2e;
  border: none; border-radius: 6px;
  font-size: 14px; font-weight: 700; cursor: pointer;
}
.submit-btn:hover:not(:disabled) { background: #74c7ec; }
.submit-btn:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
