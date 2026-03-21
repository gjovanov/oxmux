import { ref, computed, onMounted, onUnmounted } from 'vue'

const MOBILE_MAX = 767
const TABLET_MAX = 1023

const width = ref(typeof window !== 'undefined' ? window.innerWidth : 1200)
const sidebarOpen = ref(false)

function updateWidth() {
  width.value = window.innerWidth
  // Auto-close sidebar when switching to desktop
  if (width.value > TABLET_MAX) sidebarOpen.value = false
}

let listening = false

export function useResponsive() {
  onMounted(() => {
    if (!listening) {
      window.addEventListener('resize', updateWidth)
      listening = true
    }
    updateWidth()
  })

  const isMobile = computed(() => width.value <= MOBILE_MAX)
  const isTablet = computed(() => width.value > MOBILE_MAX && width.value <= TABLET_MAX)
  const isDesktop = computed(() => width.value > TABLET_MAX)

  const recommendedGridSize = computed(() => {
    if (isMobile.value) return 1
    if (isTablet.value) return 2
    return parseInt(localStorage.getItem('oxmux_mashed_grid') || '2')
  })

  function toggleSidebar() {
    sidebarOpen.value = !sidebarOpen.value
  }

  function closeSidebar() {
    sidebarOpen.value = false
  }

  return {
    isMobile,
    isTablet,
    isDesktop,
    sidebarOpen,
    toggleSidebar,
    closeSidebar,
    recommendedGridSize,
    viewportWidth: width,
  }
}
