<template>
  <div class="claude-pane">
    <!-- Session header -->
    <div class="claude-header">
      <div class="claude-title">
        <span class="claude-icon">🤖</span>
        <span>Claude Code</span>
        <span class="session-id">{{ sessionId.slice(0, 8) }}</span>
      </div>
      <div class="claude-meta">
        <CostMeter :cost="accumulator?.totalCostUsd ?? 0" :alert-threshold="costAlertThreshold" />
        <ContextBar v-if="accumulator?.lastUsage" :usage="accumulator.lastUsage" />
        <span
          class="status-badge"
          :class="statusClass"
          :data-testid="`session-status-${sessionId}`"
        >{{ statusLabel }}</span>
      </div>
    </div>

    <!-- Event stream -->
    <div class="event-stream" ref="streamRef">
      <template v-for="(event, idx) in events" :key="idx">
        <!-- Assistant message -->
        <div v-if="event.type === 'assistant'" class="event-assistant">
          <div
            v-for="(block, bi) in event.message.content"
            :key="bi"
          >
            <div v-if="block.type === 'text'" class="text-block" v-html="renderMarkdown(block.text)" />
            <ToolUseBlock
              v-else-if="block.type === 'tool_use'"
              :name="block.name"
              :input="block.input"
              :tool-id="block.id"
            />
          </div>
        </div>

        <!-- User message (tool results) -->
        <div v-else-if="event.type === 'user'" class="event-user">
          <template v-for="(block, bi) in event.message.content" :key="bi">
            <div v-if="block.type === 'tool_result'" class="tool-result-block">
              <span class="tool-result-label" :class="{ error: block.is_error }">
                {{ block.is_error ? '❌ Error' : '✅ Result' }}
              </span>
            </div>
          </template>
        </div>

        <!-- Session result -->
        <div v-else-if="event.type === 'result'" class="event-result" :class="{ error: event.is_error }">
          <span>{{ event.is_error ? '❌ Session failed' : '✅ Session complete' }}</span>
          <span v-if="event.cost_usd" class="cost">${{ event.cost_usd.toFixed(4) }}</span>
          <span class="turns">{{ event.num_turns }} turns</span>
          <span class="duration">{{ (event.duration_ms / 1000).toFixed(1) }}s</span>
        </div>
      </template>

      <!-- Streaming indicator -->
      <div v-if="isStreaming" class="streaming-indicator">
        <span class="dot" />
        <span class="dot" />
        <span class="dot" />
      </div>
    </div>

    <!-- Changed files sidebar -->
    <div v-if="fileChanges.length > 0" class="file-changes" :data-testid="`changed-files`">
      <div class="file-changes-title">Files changed</div>
      <div
        v-for="change in fileChanges"
        :key="change.path"
        class="file-change-item"
        :class="change.kind"
        @click="$emit('open-diff', change.path)"
      >
        <span class="file-icon">{{ fileIcon(change.kind) }}</span>
        <span class="file-path">{{ change.path }}</span>
      </div>
    </div>

    <!-- Prompt injection bar -->
    <div class="prompt-bar" v-if="!accumulator?.isComplete">
      <input
        v-model="promptInput"
        placeholder="Inject a message into this session..."
        @keydown.enter="injectPrompt"
        class="prompt-input"
      />
      <button @click="injectPrompt" class="prompt-send">Send</button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick } from 'vue'
import { useTmuxStore } from '@/stores/tmux'
import ToolUseBlock from './ToolUseBlock.vue'
import CostMeter from './CostMeter.vue'
import ContextBar from './ContextBar.vue'

const props = defineProps<{
  sessionId: string
  costAlertThreshold?: number
}>()

const emit = defineEmits<{
  'open-diff': [path: string]
}>()

const store = useTmuxStore()
const events = ref<unknown[]>([])
const streamRef = ref<HTMLElement | null>(null)
const promptInput = ref('')
const isStreaming = ref(false)

const accumulator = computed(() => store.claudeSessions.get(props.sessionId))

const fileChanges = computed(() => accumulator.value?.fileChanges ?? [])

const statusClass = computed(() => ({
  complete: accumulator.value?.isComplete && !accumulator.value?.isError,
  error: accumulator.value?.isError,
  running: !accumulator.value?.isComplete,
}))

const statusLabel = computed(() => {
  if (!accumulator.value) return 'Starting...'
  if (accumulator.value.isError) return 'Error'
  if (accumulator.value.isComplete) return 'Complete'
  return 'Running'
})

// Subscribe to claude events
store.subscribeClaudeSession(props.sessionId, (event) => {
  events.value.push(event)
  isStreaming.value = (event as { type: string }).type !== 'result'
  nextTick(() => {
    streamRef.value?.scrollTo({ top: streamRef.value.scrollHeight, behavior: 'smooth' })
  })
})

function renderMarkdown(text: string): string {
  // Minimal inline markdown: code, bold, italic
  return text
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/\n/g, '<br>')
}

function fileIcon(kind: string) {
  return kind === 'create' ? '+' : kind === 'delete' ? '−' : '✎'
}

function injectPrompt() {
  if (!promptInput.value.trim()) return
  store.ping() // TODO: replace with actual inject when server supports it
  promptInput.value = ''
}
</script>

<style scoped>
.claude-pane {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: #1e1e2e;
  color: #cdd6f4;
  font-family: "JetBrains Mono", monospace;
  font-size: 13px;
}

.claude-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  background: #181825;
  border-bottom: 1px solid #313244;
  gap: 12px;
}

.claude-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
}

.session-id {
  color: #585b70;
  font-size: 11px;
  font-weight: 400;
}

.claude-meta {
  display: flex;
  align-items: center;
  gap: 12px;
}

.status-badge {
  padding: 2px 8px;
  border-radius: 10px;
  font-size: 11px;
  font-weight: 600;
}
.status-badge.running  { background: #313244; color: #89b4fa; }
.status-badge.complete { background: #a6e3a1; color: #1e1e2e; }
.status-badge.error    { background: #f38ba8; color: #1e1e2e; }

.event-stream {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.event-assistant {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.text-block {
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}

.text-block :deep(code) {
  background: #313244;
  padding: 1px 4px;
  border-radius: 3px;
  font-size: 12px;
}

.event-result {
  display: flex;
  gap: 12px;
  padding: 8px;
  border-radius: 6px;
  background: #313244;
  align-items: center;
}

.event-result.error { background: rgba(243, 139, 168, 0.15); }

.cost   { color: #f9e2af; }
.turns  { color: #94e2d5; }
.duration { color: #89b4fa; }

.streaming-indicator {
  display: flex;
  gap: 4px;
  padding: 4px 0;
}
.dot {
  width: 6px; height: 6px;
  border-radius: 50%;
  background: #585b70;
  animation: pulse 1.2s ease-in-out infinite;
}
.dot:nth-child(2) { animation-delay: 0.2s; }
.dot:nth-child(3) { animation-delay: 0.4s; }
@keyframes pulse { 0%,100% { opacity: 0.3; } 50% { opacity: 1; } }

.file-changes {
  border-top: 1px solid #313244;
  padding: 8px 12px;
  max-height: 180px;
  overflow-y: auto;
}

.file-changes-title {
  font-size: 11px;
  color: #585b70;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 6px;
}

.file-change-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 0;
  cursor: pointer;
  font-size: 12px;
}
.file-change-item:hover { color: #89b4fa; }
.file-change-item.create .file-icon { color: #a6e3a1; }
.file-change-item.edit   .file-icon { color: #f9e2af; }
.file-change-item.delete .file-icon { color: #f38ba8; }

.file-path { font-family: monospace; }

.prompt-bar {
  display: flex;
  gap: 8px;
  padding: 8px 12px;
  border-top: 1px solid #313244;
}

.prompt-input {
  flex: 1;
  background: #313244;
  border: 1px solid #45475a;
  border-radius: 6px;
  padding: 6px 10px;
  color: #cdd6f4;
  font-size: 13px;
  outline: none;
}
.prompt-input:focus { border-color: #89b4fa; }

.prompt-send {
  background: #89b4fa;
  color: #1e1e2e;
  border: none;
  border-radius: 6px;
  padding: 6px 14px;
  font-weight: 600;
  cursor: pointer;
}
.prompt-send:hover { background: #74c7ec; }
</style>
