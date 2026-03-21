<template>
  <div class="mashed-view">
    <!-- Toolbar -->
    <div class="mashed-toolbar">
      <span class="mashed-title">Mashed View</span>
      <span class="mashed-count">{{ assignedCount }} / {{ totalSlots }} cells</span>
      <div v-if="!isMobile" class="grid-selector">
        <button
          v-for="size in [2, 3, 4, 5].filter(s => s <= recommendedGridSize)"
          :key="size"
          class="grid-btn"
          :class="{ active: gridSize === size }"
          @click="gridSize = size"
        >{{ size }}x{{ size }}</button>
      </div>
    </div>

    <!-- Grid -->
    <div
      class="mashed-grid"
      :style="{
        gridTemplateColumns: `repeat(${gridSize}, 1fr)`,
        gridTemplateRows: `repeat(${gridSize}, 1fr)`,
      }"
    >
      <template v-for="(slot, i) in gridSlots" :key="slot ? slot.qualifiedId : `empty-${i}`">
        <!-- Occupied cell -->
        <MashedCell
          v-if="slot"
          :qualified-pane-id="slot.qualifiedId"
          :session-name="slot.sessionName"
          :pane-command="slot.currentCommand"
          :transport-mode="store.getTransportMode(slot.sessionId)"
          :host="getHost(slot.sessionId)"
          :is-focused="store.activePane === slot.qualifiedId"
          @focus="onCellFocus(slot)"
          @remove="removeSlot(i)"
          @expand="onExpand(slot)"
        />
        <!-- Empty cell -->
        <div v-else class="mashed-empty-cell" @click="showPicker(i)">
          <div class="empty-cell-content">
            <span class="empty-plus">+</span>
            <span class="empty-label">Add pane</span>
          </div>
          <!-- Pane picker dropdown -->
          <div v-if="pickerIndex === i" class="pane-picker" @click.stop>
            <div
              v-for="pane in availablePanes"
              :key="pane.qualifiedId"
              class="picker-item"
              @click="assignPane(i, pane)"
            >
              <span class="picker-session">{{ pane.sessionName }}</span>
              <span class="picker-pane">{{ pane.id }} — {{ pane.currentCommand }}</span>
            </div>
            <div v-if="availablePanes.length === 0" class="picker-empty">No available panes</div>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { useTmuxStore } from '@/stores/tmux'
import { qualifyPaneId, type QualifiedPaneId } from '@/utils/paneId'
import { useResponsive } from '@/composables/useResponsive'
import MashedCell from '@/components/MashedCell.vue'

const store = useTmuxStore()
const { isMobile, recommendedGridSize } = useResponsive()

const emit = defineEmits<{
  'switch-view': [mode: 'single']
}>()

// Grid size: clamped to recommended max for viewport
const gridSize = ref(Math.min(
  parseInt(localStorage.getItem('oxmux_mashed_grid') || '2'),
  recommendedGridSize.value
))
watch(gridSize, (val) => localStorage.setItem('oxmux_mashed_grid', String(val)))
// Clamp grid size when viewport shrinks
watch(recommendedGridSize, (max) => {
  if (gridSize.value > max) gridSize.value = max
})

// Assignments: qualifiedPaneId or null per slot
const assignments = ref<(QualifiedPaneId | null)[]>([])

const totalSlots = computed(() => gridSize.value * gridSize.value)

const gridSlots = computed(() => {
  const slots: (ReturnType<typeof store.allPanes>[number] | null)[] = []
  for (let i = 0; i < totalSlots.value; i++) {
    const qid = assignments.value[i]
    if (qid) {
      const pane = store.allPanes.find(p => p.qualifiedId === qid)
      slots.push(pane || null)
    } else {
      slots.push(null)
    }
  }
  return slots
})

const assignedCount = computed(() => gridSlots.value.filter(Boolean).length)

const assignedIds = computed(() => new Set(assignments.value.filter(Boolean)))

const availablePanes = computed(() =>
  store.allPanes.filter(p => !assignedIds.value.has(p.qualifiedId))
)

const pickerIndex = ref<number | null>(null)

function showPicker(index: number) {
  pickerIndex.value = pickerIndex.value === index ? null : index
}

function assignPane(slotIndex: number, pane: ReturnType<typeof store.allPanes>[number]) {
  while (assignments.value.length <= slotIndex) {
    assignments.value.push(null)
  }
  assignments.value[slotIndex] = pane.qualifiedId
  pickerIndex.value = null
}

function removeSlot(index: number) {
  if (index < assignments.value.length) {
    assignments.value[index] = null
  }
}

function onCellFocus(pane: ReturnType<typeof store.allPanes>[number]) {
  store.focusedSessionId = pane.sessionId
  store.activePane = pane.qualifiedId
}

function onExpand(pane: ReturnType<typeof store.allPanes>[number]) {
  store.focusedSessionId = pane.sessionId
  store.activePane = pane.qualifiedId
  emit('switch-view', 'single')
}

function getHost(sessionId: string): string {
  const ms = store.managedSessions.find(s => s.id === sessionId)
  return (ms?.transport?.backend as any)?.host || ''
}

// Auto-fill: assign available panes to empty slots
function autoFill() {
  const available = store.allPanes
  const slots: (QualifiedPaneId | null)[] = []
  const used = new Set<string>()

  for (let i = 0; i < totalSlots.value; i++) {
    const existing = assignments.value[i]
    if (existing && available.some(p => p.qualifiedId === existing)) {
      slots.push(existing)
      used.add(existing)
    } else {
      const next = available.find(p => !used.has(p.qualifiedId))
      if (next) {
        slots.push(next.qualifiedId)
        used.add(next.qualifiedId)
      } else {
        slots.push(null)
      }
    }
  }
  assignments.value = slots
}

watch(gridSize, () => autoFill())
watch(() => store.allPanes.length, () => autoFill())

onMounted(() => autoFill())

// Close picker on outside click
function onDocClick() { pickerIndex.value = null }
onMounted(() => document.addEventListener('click', onDocClick))
onUnmounted(() => document.removeEventListener('click', onDocClick))

// Keyboard navigation: Ctrl+Arrow moves between grid cells
function onKeyDown(e: KeyboardEvent) {
  if (!e.ctrlKey || !['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key)) return

  const currentQid = store.activePane
  const occupiedSlots = gridSlots.value.map((s, i) => s ? i : -1).filter(i => i >= 0)
  if (occupiedSlots.length === 0) return

  const currentIdx = currentQid
    ? gridSlots.value.findIndex(s => s?.qualifiedId === currentQid)
    : -1

  let nextIdx = currentIdx
  const g = gridSize.value

  switch (e.key) {
    case 'ArrowRight': nextIdx = currentIdx + 1; break
    case 'ArrowLeft': nextIdx = currentIdx - 1; break
    case 'ArrowDown': nextIdx = currentIdx + g; break
    case 'ArrowUp': nextIdx = currentIdx - g; break
  }

  // Wrap and find next occupied cell
  if (nextIdx < 0 || nextIdx >= totalSlots.value) return
  const slot = gridSlots.value[nextIdx]
  if (slot) {
    e.preventDefault()
    onCellFocus(slot)
  }
}

onMounted(() => document.addEventListener('keydown', onKeyDown))
onUnmounted(() => document.removeEventListener('keydown', onKeyDown))
</script>

<style scoped>
.mashed-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

.mashed-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 12px;
  border-bottom: 1px solid #313244;
  flex-shrink: 0;
}

.mashed-title { font-weight: 700; color: #89b4fa; font-size: 13px; }
.mashed-count { font-size: 11px; color: #585b70; }

.grid-selector { display: flex; gap: 4px; margin-left: auto; }
.grid-btn {
  background: #313244; border: none; color: #a6adc8;
  padding: 2px 8px; border-radius: 4px; font-size: 11px;
  cursor: pointer; font-weight: 600;
}
.grid-btn:hover { background: #45475a; }
.grid-btn.active { background: #89b4fa; color: #1e1e2e; }

.mashed-grid {
  display: grid;
  gap: 3px;
  flex: 1;
  min-height: 0;
  padding: 3px;
}

.mashed-empty-cell {
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px dashed #45475a;
  border-radius: 4px;
  cursor: pointer;
  position: relative;
  min-height: 0;
}
.mashed-empty-cell:hover { border-color: #89b4fa; background: #181825; }

.empty-cell-content { display: flex; flex-direction: column; align-items: center; gap: 4px; color: #585b70; }
.empty-plus { font-size: 20px; }
.empty-label { font-size: 10px; }

.pane-picker {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  background: #1e1e2e;
  border: 1px solid #45475a;
  border-radius: 6px;
  padding: 4px;
  z-index: 10;
  max-height: 200px;
  overflow-y: auto;
  min-width: 180px;
}
.picker-item {
  display: flex;
  flex-direction: column;
  padding: 4px 8px;
  cursor: pointer;
  border-radius: 4px;
  font-size: 11px;
}
.picker-item:hover { background: #313244; }
.picker-session { color: #89b4fa; font-weight: 600; font-size: 10px; }
.picker-pane { color: #a6adc8; font-size: 10px; }
.picker-empty { padding: 8px; color: #585b70; font-size: 10px; text-align: center; }
</style>
