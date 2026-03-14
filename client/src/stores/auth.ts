import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

export interface User {
  id: string
  username: string
}

export const useAuthStore = defineStore('auth', () => {
  const token = ref<string | null>(localStorage.getItem('oxmux_token'))
  const user = ref<User | null>(null)
  const error = ref<string | null>(null)
  const loading = ref(false)

  const isAuthenticated = computed(() => !!token.value)

  async function login(username: string, password: string) {
    loading.value = true
    error.value = null
    try {
      const res = await fetch('/api/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username, password }),
      })
      const data = await res.json()
      if (!res.ok) {
        error.value = data.error || 'Login failed'
        return false
      }
      token.value = data.token
      user.value = data.user
      localStorage.setItem('oxmux_token', data.token)
      return true
    } catch (e) {
      error.value = 'Network error'
      return false
    } finally {
      loading.value = false
    }
  }

  async function register(username: string, password: string) {
    loading.value = true
    error.value = null
    try {
      const res = await fetch('/api/auth/register', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username, password }),
      })
      const data = await res.json()
      if (!res.ok) {
        error.value = data.error || 'Registration failed'
        return false
      }
      token.value = data.token
      user.value = data.user
      localStorage.setItem('oxmux_token', data.token)
      return true
    } catch (e) {
      error.value = 'Network error'
      return false
    } finally {
      loading.value = false
    }
  }

  async function checkAuth() {
    if (!token.value) return false
    try {
      const res = await fetch(`/api/auth/me?token=${token.value}`)
      if (!res.ok) {
        logout()
        return false
      }
      user.value = await res.json()
      return true
    } catch {
      return false
    }
  }

  function logout() {
    token.value = null
    user.value = null
    localStorage.removeItem('oxmux_token')
  }

  return { token, user, error, loading, isAuthenticated, login, register, checkAuth, logout }
})
